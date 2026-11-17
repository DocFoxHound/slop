// SPDX-License-Identifier: AGPL-3.0-or-later
//! Review and moderation actions, run by SLAPorSLOP staff from the command line (`slop-api review …`).
//!
//! Phase 1 has no web admin: an operator with production access runs these, which keeps the
//! admin surface off the public site (SECURITY §8.4). Every action is written to the audit log.

use anyhow::{Context, bail, ensure};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::catalog::AppStage;

pub(crate) async fn audit(
    tx: &mut sqlx::PgConnection,
    actor: &str,
    action: &str,
    subject: &str,
    detail: serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query("insert into audit_log (actor, action, subject, detail) values ($1, $2, $3, $4)")
        .bind(actor)
        .bind(action)
        .bind(subject)
        .bind(detail)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

fn check_reviewer(reviewer: &str) -> anyhow::Result<()> {
    ensure!(
        !reviewer.trim().is_empty(),
        "say who you are with --reviewer"
    );
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct PendingRelease {
    pub id: Uuid,
    pub app_slug: String,
    pub version: String,
    pub commit_sha: String,
    pub commit_verified: bool,
    pub repo_url: String,
    pub created_at: OffsetDateTime,
    /// A browser game's release: its play address.
    pub play_address: Option<String>,
    /// What holds it (holds.rs): scan, defender, defender_missing, rating_lowered, or question
    /// (its maker asks about a failed release).
    pub kinds: Vec<String>,
    /// Its maker wrote to staff and nobody has answered yet.
    pub unanswered: bool,
}

/// The holds that need SLAPorSLOP staff (holds.rs): releases on hold for a finding or a lowered
/// rating, or whose maker wrote to staff, and questions about failed releases. Makers waiting
/// for an answer first, then the oldest.
pub async fn pending(db: &PgPool) -> anyhow::Result<Vec<PendingRelease>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select r.id, a.slug as app_slug, r.version, r.commit_sha, r.commit_verified, a.repo_url, r.created_at,
                r.play_address, h.kinds, {unanswered} as unanswered
         from release_holds h join releases r on r.id = h.release_id join apps a on a.id = r.app_id
         where {needs}
         order by {order}",
        unanswered = crate::holds::UNANSWERED,
        needs = crate::holds::NEEDS_STAFF,
        order = crate::holds::QUEUE_ORDER,
    )))
    .fetch_all(db)
    .await?)
}

/// What approval requires, from the server's configuration.
#[derive(Clone, Copy, Debug)]
pub struct ReviewRules {
    /// Every file needs a verified maker attestation (SECURITY §5.3).
    pub require_maker_signature: bool,
    pub scanner: crate::scan::Scanner,
}

impl ReviewRules {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            require_maker_signature: config.releases.require_maker_signature,
            scanner: config.releases.scanner,
        }
    }
}

/// Accepts a release on hold (`POST /admin/releases/{id}/approve`, `slop-api review approve`):
/// SLAPorSLOP staff judged what held it a false positive, or a lowered rating right. Only what
/// its hold showed them (`release_holds.kinds`) is accepted: each file a scanner or Microsoft
/// Defender flagged is cleared (logged, as `clear_scan` does), and a lowered rating is recorded
/// as accepted (`releases.rating_accepted_*`, audited `release.rating_lowered_accepted`). A
/// missing Defender check is never accepted this way: staff waive it on its own, signed in
/// themselves (`waive_defender`). Then its checks end again (`publish::finish_checks`) with the
/// reviewer as the actor: it's ready for its maker to publish, or held again for what's left or
/// what's new (the maker and staff are told). Refused when a check fails that staff can't accept
/// (a frozen app, a file never hashed). Nothing is signed and players get nothing until the maker
/// presses Publish (publish.rs).
pub async fn approve(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    reviewer: &str,
) -> anyhow::Result<crate::publish::Outcome> {
    accept(db, rules, release_id, reviewer, None, None).await
}

/// Accepting a lowered age rating: for a SLAPorSLOP staff member signed in themselves, never an
/// AI agent's token (AGE-7: a lowered rating waits for a person).
pub(crate) const RATING_NEEDS_A_PERSON: &str = "A lowered age rating is accepted by a SLAPorSLOP staff member signed in themselves, not by an AI agent's token. Leave a note for staff instead.";

/// `approve`, with an optional note for the maker (plain text, up to 1,000 characters): kept on
/// each file it clears, and added to the hold's conversation from `staff` (the staff member's
/// account, and whether their AI agent wrote it; none from the command line). An AI agent can't
/// accept a lowered rating (`RATING_NEEDS_A_PERSON`).
pub async fn accept(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    reviewer: &str,
    staff: Option<(Uuid, bool)>,
    note: Option<&str>,
) -> anyhow::Result<crate::publish::Outcome> {
    use crate::publish::HoldKind;
    let note = clean_note(note)?;
    check_reviewer(reviewer)?;
    let by_agent = staff.is_some_and(|(_, agent)| agent);
    let mut tx = db.begin().await?;
    let status: Option<String> =
        sqlx::query_scalar("select status::text from releases where id = $1 for update")
            .bind(release_id)
            .fetch_optional(&mut *tx)
            .await?;
    let status = status.context("no such release")?;
    ensure!(
        status == "awaiting_review",
        "release is {status}, not on hold"
    );
    // What staff were shown: the open hold's kinds.
    let shown: Vec<String> = sqlx::query_scalar(
        "select kinds from release_holds where release_id = $1 and status = 'open' for update",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or_default();
    let saw = |k: HoldKind| shown.iter().any(|s| s == k.as_str());
    let verdict = crate::publish::evaluate(&mut tx, rules, release_id).await?;
    // Something the maker has to change (a frozen app, a file never hashed) isn't staff's to
    // accept: the release stays as it is.
    if let crate::publish::Verdict::Failed(reason) = &verdict {
        bail!("{reason}");
    }
    if let crate::publish::Verdict::Held(holds) = verdict {
        // A lowered rating staff were shown: accepting it is recorded on the release, with the
        // change itself, so a worse one later is held again (AGE-7). One they weren't shown
        // (the published rating rose meanwhile) stays: the checks hold it again, for them to see.
        if saw(HoldKind::RatingLowered)
            && holds.iter().any(|h| h.kind == HoldKind::RatingLowered)
            && let Some(r) = crate::releases::rating_change(&mut tx, release_id).await?
        {
            ensure!(!by_agent, "{RATING_NEEDS_A_PERSON}");
            sqlx::query(
                "update releases set rating_accepted_at = now(), rating_accepted_by = $2,
                        rating_accepted_from = $3, rating_accepted_to = $4, rating_accepted_labels = $5
                 where id = $1",
            )
            .bind(release_id)
            .bind(reviewer)
            .bind(r.from_age)
            .bind(r.to_age)
            .bind(&r.dropped_labels)
            .execute(&mut *tx)
            .await?;
            audit(
                &mut tx,
                reviewer,
                "release.rating_lowered_accepted",
                &release_id.to_string(),
                json!({ "release": release_id, "from": r.from_age, "to": r.to_age, "dropped": r.dropped_labels }),
            )
            .await?;
        }
        // Findings staff were shown: the scanner's, and Microsoft Defender's detections.
        let mut cleared: Vec<(&str, bool)> = Vec::new();
        for h in &holds {
            let defender = match h.kind {
                HoldKind::Scan if saw(HoldKind::Scan) => false,
                HoldKind::Defender if saw(HoldKind::Defender) => true,
                _ => continue,
            };
            match cleared.iter_mut().find(|(p, _)| *p == h.platform) {
                Some((_, d)) => *d |= defender,
                None => cleared.push((&h.platform, defender)),
            }
        }
        for (platform, defender) in cleared {
            sqlx::query(
                "update release_files set cleared_by = coalesce(cleared_by, $3),
                        cleared_at = coalesce(cleared_at, now()),
                        cleared_reason = coalesce(cleared_reason, $4),
                        defender_cleared_at = case when $5 then coalesce(defender_cleared_at, now())
                                                   else defender_cleared_at end
                 where release_id = $1 and platform = $2",
            )
            .bind(release_id)
            .bind(platform)
            .bind(reviewer)
            .bind(note.as_deref())
            .bind(defender)
            .execute(&mut *tx)
            .await?;
            audit(
                &mut tx,
                reviewer,
                "release.clear_scan",
                &release_id.to_string(),
                json!({ "release": release_id, "platform": platform, "reason": note, "defender": defender }),
            )
            .await?;
        }
    }
    if let Some(note) = &note {
        let author = staff.map(|(id, _)| id);
        crate::holds::add_message(&mut tx, release_id, author, true, by_agent, note).await?;
    }
    let outcome = crate::publish::finish_checks_in(&mut tx, rules, release_id, reviewer).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// A note for the maker: plain text as staff write it (admin_messages.rs `clean`), trimmed,
/// empty for none, at most 1,000 characters.
fn clean_note(note: Option<&str>) -> anyhow::Result<Option<String>> {
    match note.map(str::trim).filter(|n| !n.is_empty()) {
        None => Ok(None),
        Some(n) => crate::admin_messages::clean("reason", n, 1000, false)
            .map(Some)
            .map_err(|_| {
                anyhow::anyhow!(
                    "keep the note to 1,000 characters of plain text, without control characters other than new lines"
                )
            }),
    }
}

/// Clears a file's scan finding a reviewer judged a false positive, and Microsoft Defender's
/// detections in it when Defender flagged it, with an optional note for the maker (plain text,
/// up to 1,000 characters), then ends the release's checks again (`publish::finish_checks`): if
/// nothing holds it any more, it's ready for its maker to publish and they're told. A missing
/// Defender check isn't a finding and isn't cleared (`waive_defender`). Audit-logged.
pub async fn clear_scan(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    platform: &str,
    reviewer: &str,
    reason: Option<&str>,
) -> anyhow::Result<crate::publish::Outcome> {
    check_reviewer(reviewer)?;
    let reason = clean_note(reason)?;
    let mut tx = db.begin().await?;
    // The release first, then its files and hold, as finish_checks and `accept` lock them.
    let slug: Option<String> = sqlx::query_scalar(
        "select a.slug from releases r join apps a on a.id = r.app_id
         where r.id = $1 and r.status = 'awaiting_review' for update of r",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?;
    let slug = slug.context("no such file on hold")?;
    let file: Option<(bool, bool)> = sqlx::query_as(
        "select coalesce(scan_result in ('on_hold', 'rejected', 'blocked'), false) and cleared_at is null,
                platform = 'windows' and lower(defender ->> 'sha256') = lower(sha256)
                    and jsonb_array_length(coalesce(defender -> 'detections', '[]')) > 0
                    and defender_cleared_at is null
         from release_files where release_id = $1 and platform = $2 for update",
    )
    .bind(release_id)
    .bind(platform)
    .fetch_optional(&mut *tx)
    .await?;
    let (scan_flagged, defender_flagged) = file.context("no such file on hold")?;
    ensure!(
        scan_flagged || defender_flagged,
        "nothing on this file is flagged to clear; a missing Microsoft Defender check is waived on its own, or the release rejected"
    );
    sqlx::query(
        "update release_files set cleared_by = $3, cleared_reason = $4, cleared_at = now(),
                defender_cleared_at = case when $5 then now() else defender_cleared_at end
         where release_id = $1 and platform = $2",
    )
    .bind(release_id)
    .bind(platform)
    .bind(reviewer)
    .bind(reason.as_deref())
    .bind(defender_flagged)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        reviewer,
        "release.clear_scan",
        &slug,
        json!({ "release": release_id, "platform": platform, "reason": reason, "defender": defender_flagged }),
    )
    .await?;
    let outcome = crate::publish::finish_checks_in(&mut tx, rules, release_id, reviewer).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// Waiving Microsoft Defender's check: for a SLAPorSLOP staff member signed in themselves, never
/// an AI agent's token.
pub(crate) const WAIVER_NEEDS_A_PERSON: &str = "Waiving Microsoft Defender's check is for a SLAPorSLOP staff member signed in themselves, not an AI agent's token.";

/// Waives the missing Microsoft Defender check of release `release_id`'s Windows file
/// (`POST /admin/releases/{id}/files/{platform}/waive-defender`): a staff member signed in
/// themselves checked the build by hand, typically for a repository outside GitHub, where Slop
/// Check can't run. Recorded on the file (`defender_waived_at`/`_by`), audited
/// `release.waive_defender`, said in the hold's log with the optional note (plain text, up to
/// 1,000 characters) from `staff`; then the release's checks end again. A Defender report that
/// flags the file later still holds it.
pub async fn waive_defender(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    platform: &str,
    reviewer: &str,
    staff: Option<Uuid>,
    note: Option<&str>,
) -> anyhow::Result<crate::publish::Outcome> {
    check_reviewer(reviewer)?;
    let note = clean_note(note)?;
    let mut tx = db.begin().await?;
    let status: Option<String> =
        sqlx::query_scalar("select status::text from releases where id = $1 for update")
            .bind(release_id)
            .fetch_optional(&mut *tx)
            .await?;
    let status = status.context("no such release")?;
    ensure!(
        status == "awaiting_review",
        "release is {status}, not on hold"
    );
    let file: Option<(String, bool)> = sqlx::query_as(
        "select file_name, defender is not null and lower(defender ->> 'sha256') = lower(sha256)
         from release_files where release_id = $1 and platform = $2 for update",
    )
    .bind(release_id)
    .bind(platform)
    .fetch_optional(&mut *tx)
    .await?;
    let (file_name, checked) = file.context("no such file on hold")?;
    ensure!(
        platform == "windows",
        "only a Windows build needs Microsoft Defender's check"
    );
    ensure!(
        !checked,
        "Microsoft Defender has checked this file, so there's nothing to waive"
    );
    sqlx::query(
        "update release_files set defender_waived_at = now(), defender_waived_by = $3
         where release_id = $1 and platform = $2",
    )
    .bind(release_id)
    .bind(platform)
    .bind(reviewer)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        reviewer,
        "release.waive_defender",
        &release_id.to_string(),
        json!({ "release": release_id, "platform": platform, "reason": note }),
    )
    .await?;
    let has_hold: bool =
        sqlx::query_scalar("select exists (select 1 from release_holds where release_id = $1)")
            .bind(release_id)
            .fetch_one(&mut *tx)
            .await?;
    if has_hold {
        crate::publish::system_line(
            &mut tx,
            release_id,
            &format!(
                "SLAPorSLOP staff checked the {} build by hand and waived Microsoft Defender's check.",
                crate::release_progress::file_label(platform, &file_name)
            ),
            None,
        )
        .await?;
    }
    if let Some(note) = &note {
        crate::holds::add_message(&mut tx, release_id, staff, true, false, note).await?;
    }
    let outcome = crate::publish::finish_checks_in(&mut tx, rules, release_id, reviewer).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// Rejects a release that's on hold, being checked, ready for its maker to publish (it never
/// reached players), or whose checks failed. The reason is shown to the maker.
pub async fn reject(
    db: &PgPool,
    release_id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let reason = reason.trim();
    ensure!(!reason.is_empty(), "give the maker a reason with --reason");
    let mut tx = db.begin().await?;
    let updated = sqlx::query(
        "update releases set status = 'rejected', status_reason = $2, reviewed_at = now(), reviewed_by = $3
         where id = $1 and status in ('awaiting_review', 'fetching', 'ready', 'failed')",
    )
    .bind(release_id)
    .bind(reason)
    .bind(reviewer)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(updated == 1, "no unpublished release with that ID");
    if crate::publish::close_hold(&mut tx, release_id, "rejected", reviewer).await? {
        crate::publish::system_line(
            &mut tx,
            release_id,
            &format!("Not accepted by SLAPorSLOP staff: {reason}"),
            None,
        )
        .await?;
    }
    audit(
        &mut tx,
        reviewer,
        "release.reject",
        &release_id.to_string(),
        json!({ "reason": reason }),
    )
    .await?;
    crate::releases::tell_maker(
        &mut tx,
        release_id,
        crate::releases::ReleaseEvent::Rejected { reason },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The kill switch (SECURITY §5.5): a revoked release stops launching on every client. For a
/// browser game, Play stops opening the address: its publish time is kept, so if it was the
/// newest published address, Play opens none until its maker publishes a new one
/// (`catalog::PLAY_ADDRESS`), rather than an older address that may be the same site.
pub async fn revoke(
    db: &PgPool,
    release_id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(
        !reason.trim().is_empty(),
        "say why with --reason; it's shown to players"
    );
    let mut tx = db.begin().await?;
    let updated: Option<Option<String>> = sqlx::query_scalar(
        "update releases set status = 'revoked', status_reason = $2, revoked_at = now()
         where id = $1 and status = 'approved' returning play_address",
    )
    .bind(release_id)
    .bind(reason.trim())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(play_address) = updated else {
        anyhow::bail!("no approved release with that ID");
    };
    // An app held at this release goes back to its newest approved one.
    sqlx::query("update apps set pinned_release_id = null where pinned_release_id = $1")
        .bind(release_id)
        .execute(&mut *tx)
        .await?;
    // A play address is named, so the alert (ops.rs) says Play stops opening it.
    let mut detail = json!({ "reason": reason });
    if let Some(address) = play_address {
        detail["play_address"] = json!(address);
    }
    audit(
        &mut tx,
        reviewer,
        "release.revoke",
        &release_id.to_string(),
        detail,
    )
    .await?;
    crate::releases::tell_maker(
        &mut tx,
        release_id,
        crate::releases::ReleaseEvent::Withdrawn { reason },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Whether app `a` (`apps`) has something to play, as SQL: a published (approved) release, and for
/// a browser game an address Play opens (`catalog::PLAY_RELEASE`). With its newest address
/// withdrawn, a browser game has none, even with older ones published. What Beta, Released and
/// listing need.
pub(crate) fn playable_sql() -> String {
    format!(
        "(case when 'browser' = any(a.platforms) then {} is not null
               else exists (select 1 from releases pr where pr.app_id = a.id and pr.status = 'approved') end)",
        crate::catalog::PLAY_RELEASE
    )
}

/// `playable_sql` for one app.
pub(crate) async fn has_playable_release(
    tx: &mut sqlx::PgConnection,
    app_id: Uuid,
) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select {} from apps a where a.id = $1",
        playable_sql()
    )))
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?)
}

/// Why an app can't go in the store yet: the checks every listing makes.
pub(crate) enum NotListable {
    Private,
    Frozen(String),
    NoPulse,
    NoRelease(AppStage),
    /// A browser game's release is its play address.
    NoAddress(AppStage),
    PicturesWaiting,
}

/// "a Beta", "a Released", for "… app needs …".
fn stage_words(stage: AppStage) -> &'static str {
    match stage {
        AppStage::ComingSoon => "a Coming soon",
        AppStage::Beta => "a Beta",
        AppStage::Released => "a Released",
    }
}

impl NotListable {
    fn for_reviewer(&self) -> String {
        match self {
            Self::Private => "it's a private app: private apps are never listed in the store \
                              (unless the owner asks to make it public)"
                .into(),
            Self::Frozen(reason) => format!(
                "the app is frozen ({reason}); it can be listed once SLAPorSLOP can read its repository"
            ),
            Self::NoPulse => "this community build's original counts players with Slop Pulse, so \
                              the build must too (DESIGN §9): no Pulse handshake from it yet. The \
                              author launches it through SLAPorSLOP once and the handshake is recorded."
                .into(),
            Self::NoRelease(stage) => format!(
                "{} app needs a published release before it's listed, and it has none yet: its \
                 maker submits one from Releases & progress and publishes it once it passes the \
                 checks. A Coming soon page can be listed without one.",
                capitalized(stage_words(*stage))
            ),
            Self::NoAddress(stage) => format!(
                "{} browser game needs a published play address before it's listed, and it has \
                 none yet: its maker gives one on Releases & progress and publishes it once it \
                 passes the checks. A Coming soon page can be listed without one.",
                capitalized(stage_words(*stage))
            ),
            Self::PicturesWaiting => {
                "review the app's pictures first: a new set is waiting under Pictures to review"
                    .into()
            }
        }
    }

    /// For the maker, when an appeal reverses a delisting but the app can't go back in yet.
    pub(crate) fn for_maker(&self) -> String {
        match self {
            Self::Private => "It's private, so it isn't listed in the store.".into(),
            Self::Frozen(reason) => format!(
                "It stays out of the store while it's frozen ({reason}). It goes back in once \
                 SLAPorSLOP can read its repository again."
            ),
            Self::NoPulse => "It stays out of the store until SLAPorSLOP records a Pulse handshake \
                              from it: launch it through SLAPorSLOP once. A reviewer then puts it back."
                .into(),
            Self::NoRelease(_) => "It stays out of the store until a release is published: submit \
                                   one from Releases & progress and publish it once it passes the \
                                   checks. A reviewer then puts it back."
                .into(),
            Self::NoAddress(_) => "It stays out of the store until a play address is published: \
                                   give it on Releases & progress and publish it once it passes \
                                   the checks. A reviewer then puts it back."
                .into(),
            Self::PicturesWaiting => "It stays out of the store until its new pictures are \
                                      reviewed. A reviewer then puts it back."
                .into(),
        }
    }

    /// For the maker putting their app in the store the first time (manage.rs `list_app`): what
    /// to do first, on the page they're on or the one named.
    pub(crate) fn for_first_listing(&self) -> String {
        match self {
            Self::Private => "It's private, so it can't go in the store. To list it, ask to make \
                              it public on this page: SLAPorSLOP staff check the repository is \
                              public, then put it in the store."
                .into(),
            Self::Frozen(reason) => format!(
                "It can't go in the store while it's frozen ({reason}). It can once SLAPorSLOP \
                 can read its repository again."
            ),
            Self::NoPulse => {
                "Its original counts players with Slop Pulse, so this build must too: \
                              launch it through SLAPorSLOP once, then put it in the store."
                    .into()
            }
            Self::NoRelease(stage) => format!(
                "{} app needs a published release first: submit one on the Releases page and \
                 publish it once it passes the checks. A Coming soon page can go in without one.",
                capitalized(stage_words(*stage))
            ),
            Self::NoAddress(stage) => format!(
                "{} browser game needs a published play address first: give it on the Releases \
                 page and publish it once it passes the checks. A Coming soon page can go in \
                 without one.",
                capitalized(stage_words(*stage))
            ),
            Self::PicturesWaiting => "Its new pictures are waiting for review: you can put it in \
                                      the store once they're reviewed."
                .into(),
        }
    }
}

/// `s` with its first letter in capitals.
fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The apps SLAPorSLOP staff put in the store, as SQL on `apps a`: one reinstated on appeal that
/// waits to go back, and a private app whose owner asked to make it public (SECURITY §12.3).
/// Makers put a draft in the store themselves the first time (`POST /v1/apps/{slug}/list`,
/// manage.rs). With `playable_sql`, what the staff queue's "ready to list" holds.
pub(crate) const STAFF_LISTS: &str = "((a.status = 'delisted' and a.relist_after_appeal)
      or (a.status = 'draft' and a.visibility = 'private' and a.public_requested_at is not null))";

/// What stops the app going in the store, if anything.
pub(crate) async fn not_listable(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    stage: AppStage,
) -> anyhow::Result<Option<NotListable>> {
    if crate::catalog::is_private(&mut *tx, id).await? {
        return Ok(Some(NotListable::Private));
    }
    if let Some(reason) = crate::frozen::frozen_reason(&mut *tx, id).await? {
        return Ok(Some(NotListable::Frozen(reason)));
    }
    if crate::branches::needs_pulse(tx, id).await? && !crate::branches::has_pulse(tx, id).await? {
        return Ok(Some(NotListable::NoPulse));
    }
    if stage != AppStage::ComingSoon && !has_playable_release(tx, id).await? {
        return Ok(Some(
            if crate::catalog::plays_in_browser(&mut *tx, id).await? {
                NotListable::NoAddress(stage)
            } else {
                NotListable::NoRelease(stage)
            },
        ));
    }
    if crate::media::has_pending(tx, id).await? {
        return Ok(Some(NotListable::PicturesWaiting));
    }
    Ok(None)
}

/// Puts an app in the store, from staff (`slop-api review list`, the admin panel): only one
/// reinstated on appeal (`STAFF_LISTS`). Makers put their own drafts in the store (manage.rs
/// `list_app`), and a private app goes public with `make_public`. Beta and Released apps need a
/// published release first; Coming soon pages have no build (DESIGN §8).
pub async fn list_app(db: &PgPool, slug: &str, reviewer: &str) -> anyhow::Result<()> {
    list_app_as(db, slug, reviewer, false).await
}

/// Private apps whose owner asked to make it public (SECURITY §12.3), once the admin panel has
/// checked its repository is public (frozen.rs): it becomes public and goes in the store. Its
/// private shares end, since everyone can see it now.
pub async fn make_public(db: &PgPool, slug: &str, reviewer: &str) -> anyhow::Result<()> {
    list_app_as(db, slug, reviewer, true).await
}

async fn list_app_as(
    db: &PgPool,
    slug: &str,
    reviewer: &str,
    going_public: bool,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, AppStage, String)> =
        sqlx::query_as("select id, stage, status::text from apps where slug = $1 for update")
            .bind(slug)
            .fetch_optional(&mut *tx)
            .await?;
    let (id, stage, status) = row.context("no such app")?;
    ensure!(status != "listed", "already listed");
    let staffs: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select {STAFF_LISTS} from apps a where a.id = $1"
    )))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        staffs,
        "staff list an app reinstated on appeal, or a private one whose owner asked to make it public; its maker puts a draft in the store themselves, from its Manage page"
    );
    if going_public {
        let requested: bool = sqlx::query_scalar(
            "select visibility = 'private' and public_requested_at is not null from apps where id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(requested, "its owner hasn't asked to make it public");
        // Nothing was counted while it was private: its History starts today (history.rs).
        sqlx::query(
            "update apps set visibility = 'public', public_requested_at = null,
                    history_from = (now() at time zone 'UTC')::date,
                    players_from = (now() at time zone 'UTC')::date
             where id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("delete from app_shares where app_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("delete from app_team_shares where app_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(why) = not_listable(&mut tx, id, stage).await? {
        bail!("{}", why.for_reviewer());
    }
    put_in_store(&mut tx, id).await?;
    audit(
        &mut tx,
        reviewer,
        if going_public {
            "app.make_public"
        } else {
            "app.list"
        },
        slug,
        json!({ "from": status }),
    )
    .await?;
    if going_public {
        // Its dedicated servers were shared with the same people: they go public with it, and
        // the ones ready for the store go in with it (docs/DEDICATED-SERVERS.md §8).
        let servers: Vec<(Uuid, AppStage, String)> = sqlx::query_as(
            "update apps set visibility = 'public', updated_at = now()
             where parent_app_id = $1 and branch_kind = 'server' and visibility = 'private'
             returning id, stage, slug",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        for (server, stage, server_slug) in servers {
            if not_listable(&mut tx, server, stage).await?.is_none() {
                sqlx::query(
                    "update apps set status = 'listed' where id = $1 and status <> 'listed'",
                )
                .bind(server)
                .execute(&mut *tx)
                .await?;
                audit(
                    &mut tx,
                    reviewer,
                    "app.list",
                    &server_slug,
                    json!({ "with": slug }),
                )
                .await?;
            }
        }
    }
    if going_public {
        let (maker, title): (Uuid, String) =
            sqlx::query_as("select maker_id, title from apps where id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        crate::notify::send(
            &mut tx,
            crate::notify::Notice {
                user_id: maker,
                category: "apps",
                title: format!("{title} is public and in the store"),
                body: "A reviewer checked it again from scratch. Anyone can find it now; the \
                       people you'd shared it with keep it in their library like everyone else."
                    .into(),
                link: format!("/app/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Puts app `id` (locked by the caller, `not_listable` found nothing) in the store; an appeal's
/// relist is done with it. The caller writes the audit entry.
pub(crate) async fn put_in_store(tx: &mut sqlx::PgConnection, id: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        "update apps set status = 'listed', relist_after_appeal = false, updated_at = now() where id = $1",
    )
    .bind(id)
    .execute(tx)
    .await?;
    Ok(())
}

/// Takes an app out of the store (moderation). Installs keep working unless a release is revoked.
/// The maker gets a statement of reasons and can appeal (moderation.rs).
pub async fn delist_app(
    db: &PgPool,
    slug: &str,
    reviewer: &str,
    rule: &str,
    facts: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(!facts.trim().is_empty(), "say why with --reason");
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "select id, maker_id, title from apps where slug = $1 and status = 'listed' for update",
    )
    .bind(slug)
    .fetch_optional(&mut *tx)
    .await?;
    let (id, maker, title) = row.context("no listed app with that address")?;
    crate::moderation::decide(
        &mut tx,
        &crate::moderation::NewDecision {
            user_id: maker,
            subject_kind: "app",
            subject_id: id,
            subject_label: &format!("App: {title}"),
            action: "delist_app",
            rule,
            law: None,
            facts,
            decided_by: reviewer,
        },
    )
    .await?;
    audit(
        &mut tx,
        reviewer,
        "app.delist",
        slug,
        json!({ "rule": rule, "reason": facts }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Moves an app to a new stage. Beta and Released need a published release.
pub async fn set_stage(
    db: &PgPool,
    slug: &str,
    stage: AppStage,
    reviewer: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, AppStage)> =
        sqlx::query_as("select id, stage from apps where slug = $1 for update")
            .bind(slug)
            .fetch_optional(&mut *tx)
            .await?;
    let (id, from) = row.context("no such app")?;
    ensure!(from != stage, "already in that stage");
    if stage != AppStage::ComingSoon && !has_playable_release(&mut tx, id).await? {
        bail!(
            "{}",
            if crate::catalog::plays_in_browser(&mut *tx, id).await? {
                "its maker publishes a play address first: a Beta or Released browser game needs an address Play opens"
            } else {
                "its maker publishes a release first: Beta and Released apps need a published release"
            }
        );
    }
    move_stage(&mut tx, id, slug, from, stage, reviewer).await?;
    tx.commit().await?;
    Ok(())
}

/// Moves an app (locked by the caller) from `from` to `stage`: restarts the stage's clock, tells
/// everyone Interested, and records who did it. The caller checks the move is allowed.
pub(crate) async fn move_stage(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    slug: &str,
    from: AppStage,
    stage: AppStage,
    actor: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "update apps set stage = $2, stage_entered_at = now(), updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(stage)
    .execute(&mut *tx)
    .await?;
    let told = crate::interested::stage_moved(&mut *tx, id, stage).await?;
    audit(
        &mut *tx,
        actor,
        "app.stage",
        slug,
        json!({ "from": from, "to": stage, "interested_told": told }),
    )
    .await?;
    Ok(())
}

/// Gives or takes away admin rights. `who` is the account's ID, email or exact display name.
pub async fn set_admin(db: &PgPool, who: &str, admin: bool, actor: &str) -> anyhow::Result<String> {
    check_reviewer(actor)?;
    // By account ID only when granting: display names are chosen by anyone, so someone could
    // sign up under the name an operator is about to grant (security review, 2026-10-22).
    let matches: Vec<(Uuid, String)> = if admin {
        let id: Uuid = who.trim().parse().map_err(|_| {
            anyhow::anyhow!("give the account's ID (from its profile link) to grant admin rights")
        })?;
        sqlx::query_as("select id, display_name from users where id = $1")
            .bind(id)
            .fetch_all(db)
            .await?
    } else {
        sqlx::query_as(
            "select id, display_name from users
             where id::text = $1 or lower(email) = lower($1) or display_name = $1",
        )
        .bind(who.trim())
        .fetch_all(db)
        .await?
    };
    let (id, name) = match matches.as_slice() {
        [one] => one.clone(),
        [] => bail!("no account matches {who:?}; sign in on the website first"),
        many => bail!(
            "{} accounts match {who:?}; use the account ID: {}",
            many.len(),
            many.iter()
                .map(|(id, n)| format!("{id} ({n})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut tx = db.begin().await?;
    sqlx::query("update users set is_admin = $2 where id = $1")
        .bind(id)
        .bind(admin)
        .execute(&mut *tx)
        .await?;
    // From the command line, staff start as owners (the server's operator is trusted); on the
    // website, roles are given one at a time (staff.rs). Losing staff rights ends every role.
    if admin {
        sqlx::query(
            "insert into staff_roles (user_id, role, granted_by) values ($1, 'owner', $2) on conflict do nothing",
        )
        .bind(id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query("delete from staff_roles where user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    audit(
        &mut tx,
        actor,
        if admin { "admin.grant" } else { "admin.revoke" },
        &id.to_string(),
        json!({ "name": name }),
    )
    .await?;
    tx.commit().await?;
    Ok(name)
}
