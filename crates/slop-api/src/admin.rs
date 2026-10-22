// SPDX-License-Identifier: AGPL-3.0-or-later
//! Review and moderation actions, run by Slop staff from the command line (`slop-api review …`).
//!
//! Phase 1 has no web admin: an operator with production access runs these, which keeps the
//! admin surface off the public site (SECURITY §8.4). Every action is written to the audit log.

use anyhow::{Context, bail, ensure};
use serde_json::json;
use slop_manifest::{
    FileKind, Launch, MANIFEST_SCHEMA, ManifestFile, Platform, Purpose, ReleaseManifest,
};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
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
}

/// Releases waiting for a reviewer, oldest first.
pub async fn pending(db: &PgPool) -> anyhow::Result<Vec<PendingRelease>> {
    Ok(sqlx::query_as(
        "select r.id, a.slug as app_slug, r.version, r.commit_sha, r.commit_verified, a.repo_url, r.created_at
         from releases r join apps a on a.id = r.app_id
         where r.status = 'awaiting_review' order by r.created_at",
    )
    .fetch_all(db)
    .await?)
}

#[derive(sqlx::FromRow)]
struct ReleaseRow {
    status: String,
    app_id: Uuid,
    slug: String,
    title: String,
    maker: String,
    repo_url: String,
    version: String,
    commit_sha: String,
    /// A mod or community build: the original's ID, address and name.
    branch_kind: Option<String>,
    mods_folder: Option<serde_json::Value>,
    runtimes: Vec<String>,
    game_id: Option<Uuid>,
    game_slug: Option<String>,
    game_title: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FileRow {
    platform: String,
    kind: String,
    url: String,
    file_name: String,
    exec: Option<String>,
    args: Vec<String>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    attestation: Option<String>,
    provenance_problem: Option<String>,
    maker_signature: Option<sqlx::types::Json<slop_manifest::MakerSignature>>,
    /// In parts (releases.rs `download_parts`): each with its URL, size, hash and attestation.
    parts: Option<sqlx::types::Json<Vec<PartRow>>>,
    scan_result: Option<String>,
    scan_detail: Option<String>,
    defender: Option<sqlx::types::Json<crate::scan::DefenderCheck>>,
    cleared_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct PartRow {
    url: String,
    size: Option<u64>,
    sha256: Option<String>,
    attestation: Option<String>,
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

/// Approves a release and signs its manifest, if every file meets `rules`.
pub async fn approve(
    db: &PgPool,
    key: Option<&slop_manifest::ed25519_dalek::SigningKey>,
    rules: ReviewRules,
    release_id: Uuid,
    reviewer: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let key = key.context("SLOP_SIGNING_KEY isn't set, so nothing can be signed")?;
    let mut tx = db.begin().await?;
    let release: ReleaseRow = sqlx::query_as(
        "select r.status::text as status, a.id as app_id, a.slug, a.title, u.display_name as maker,
                a.repo_url, r.version, r.commit_sha, a.branch_kind, a.mods_folder, r.runtimes, g.id as game_id, g.slug as game_slug,
                g.title as game_title
         from releases r join apps a on a.id = r.app_id join users u on u.id = a.maker_id
              left join apps g on g.id = a.parent_app_id
         where r.id = $1 for update of r",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("no such release")?;
    let status = &release.status;
    ensure!(
        status == "awaiting_review",
        "release is {status}, not awaiting review"
    );
    if let Some(reason) = crate::frozen::frozen_reason(&mut *tx, release.app_id).await? {
        bail!("the app is frozen ({reason}), so its releases can't be checked or approved");
    }

    let rows: Vec<FileRow> = sqlx::query_as(
        "select platform, kind, url, file_name, exec, args, size_bytes, sha256, attestation, provenance_problem,
                (maker_signature - 'fingerprint' - 'key_name') as maker_signature, parts,
                scan_result, scan_detail, defender, cleared_reason
         from release_files where release_id = $1 order by platform",
    )
    .bind(release_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut files = Vec::new();
    for f in rows {
        let (Some(size), Some(sha256)) = (f.size_bytes, f.sha256) else {
            bail!("the {} build hasn't been hashed", f.platform);
        };
        let scan = f
            .scan_result
            .as_deref()
            .and_then(crate::scan::ScanResult::parse);
        let cleared = f.cleared_reason.is_some();
        match scan {
            Some(_) if cleared => {}
            Some(result) if rules.scanner.approvable(result) => {}
            Some(result) => bail!(
                "the {} build's scan result is {}: {}. If it's a false positive, clear it with a reason",
                f.platform,
                result.as_str(),
                f.scan_detail.as_deref().unwrap_or("no details")
            ),
            None => bail!("the {} build hasn't been through the scan step", f.platform),
        }
        // Windows builds need Microsoft Defender's all-clear, so players' Windows runs them.
        if rules.scanner.is_on() && f.platform == "windows" && !cleared {
            match &f.defender {
                Some(d) if !d.sha256.eq_ignore_ascii_case(&sha256) => bail!(
                    "the Windows build's Defender check was for a different file; run Slop Check again"
                ),
                Some(d) if !d.detections.is_empty() => bail!(
                    "Microsoft Defender flags the Windows build ({}). If it's a false positive, the maker submits it to Microsoft, or a reviewer clears it with a reason",
                    d.detections
                        .iter()
                        .map(|x| x.threat.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Some(_) => {}
                None => bail!(
                    "the Windows build hasn't been checked with Microsoft Defender: Slop Build does it, and builds made elsewhere run Slop Check"
                ),
            }
        }
        let parts: Vec<slop_manifest::FilePart> = f
            .parts
            .map(|p| p.0)
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                Ok(slop_manifest::FilePart {
                    url: p.url,
                    size: p.size.context("a part wasn't fetched")?,
                    sha256: p.sha256.context("a part wasn't hashed")?,
                    attestation: p.attestation,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let parts_signed = !parts.is_empty() && parts.iter().all(|p| p.attestation.is_some());
        if rules.require_maker_signature
            && f.attestation.is_none()
            && f.maker_signature.is_none()
            && !parts_signed
        {
            bail!(
                "the {} build has no verified maker signature: {}",
                f.platform,
                f.provenance_problem.as_deref().unwrap_or("not checked")
            );
        }
        files.push(ManifestFile {
            platform: Platform::parse(&f.platform).context("bad platform")?,
            kind: FileKind::parse(&f.kind).context("bad kind")?,
            url: f.url,
            file_name: f.file_name,
            size: size as u64,
            sha256,
            launch: f.exec.map(|exec| Launch { exec, args: f.args }),
            maker_signature: if f.attestation.is_none() {
                f.maker_signature.map(|s| s.0)
            } else {
                None
            },
            attestation: f.attestation,
            parts,
        });
    }
    ensure!(!files.is_empty(), "release has no files");

    let now = OffsetDateTime::now_utc().replace_nanosecond(0)?;
    let is_mod = release.branch_kind.as_deref() == Some("addon");
    let original = match (release.game_id, release.game_slug, release.game_title) {
        (Some(id), Some(slug), Some(title)) => Some(slop_manifest::ModFor {
            app_id: id.to_string(),
            app_slug: slug,
            app_title: title,
        }),
        _ => None,
    };
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: release.app_id.to_string(),
        app_slug: release.slug.clone(),
        app_title: release.title,
        maker: release.maker,
        repo_url: release.repo_url,
        release_id: release_id.to_string(),
        version: release.version.clone(),
        commit: release.commit_sha,
        files,
        approved_at: now.format(&Rfc3339)?,
        mod_for: original.clone().filter(|_| is_mod),
        build_of: original.filter(|_| !is_mod),
        mods_folder: release
            .mods_folder
            .and_then(|f| serde_json::from_value(f).ok())
            .filter(|f: &slop_manifest::ModsFolder| !is_mod && f.is_valid()),
        runtimes: release.runtimes,
    };
    let signed = slop_manifest::sign(&manifest, Purpose::ReleaseManifest, key);
    sqlx::query(
        "update releases set status = 'approved', reviewed_at = now(), signed_manifest = $2, status_reason = null,
                reviewed_by = $3
         where id = $1",
    )
    .bind(release_id)
    .bind(sqlx::types::Json(&signed))
    .bind(reviewer)
    .execute(&mut *tx)
    .await?;
    // The store page shows the rating of the release players get (AGE-7).
    sqlx::query(
        "update apps a set min_age = r.min_age, content_labels = r.content_labels, updated_at = now()
         from releases r where r.id = $1 and a.id = r.app_id and r.min_age is not null",
    )
    .bind(release_id)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        reviewer,
        "release.approve",
        &release_id.to_string(),
        json!({ "app": release.slug, "version": release.version, "key_id": signed.key_id }),
    )
    .await?;
    crate::releases::tell_maker(
        &mut tx,
        release_id,
        crate::releases::ReleaseEvent::Approved {
            automatically: reviewer == crate::updates::AUTO_REVIEWER,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Rejects a release. The reason is shown to the maker.
/// Clears a scan finding (or a missing or flagging Defender check) a reviewer judged a false
/// positive, with the reason, so the release can be approved. Audit-logged.
pub async fn clear_scan(
    db: &PgPool,
    release_id: Uuid,
    platform: &str,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let reason = reason.trim();
    ensure!(
        (10..=1000).contains(&reason.chars().count()),
        "say why it's a false positive (10 to 1,000 characters)"
    );
    let mut tx = db.begin().await?;
    let slug: Option<String> = sqlx::query_scalar(
        "update release_files f set cleared_by = $3, cleared_reason = $4, cleared_at = now()
         from releases r join apps a on a.id = r.app_id
         where f.release_id = $1 and f.platform = $2 and r.id = f.release_id and r.status = 'awaiting_review'
         returning a.slug",
    )
    .bind(release_id)
    .bind(platform)
    .bind(reviewer)
    .bind(reason)
    .fetch_optional(&mut *tx)
    .await?;
    let slug = slug.context("no such file waiting for review")?;
    audit(
        &mut tx,
        reviewer,
        "release.clear_scan",
        &slug,
        json!({ "release": release_id, "platform": platform, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reject(
    db: &PgPool,
    release_id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(
        !reason.trim().is_empty(),
        "give the maker a reason with --reason"
    );
    let mut tx = db.begin().await?;
    let updated = sqlx::query(
        "update releases set status = 'rejected', status_reason = $2, reviewed_at = now(), reviewed_by = $3
         where id = $1 and status in ('awaiting_review', 'fetching', 'failed')",
    )
    .bind(release_id)
    .bind(reason.trim())
    .bind(reviewer)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(updated == 1, "no release waiting for review with that ID");
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

/// The kill switch (SECURITY §5.5): a revoked release stops launching on every client.
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
    let updated = sqlx::query(
        "update releases set status = 'revoked', status_reason = $2, revoked_at = now()
         where id = $1 and status = 'approved'",
    )
    .bind(release_id)
    .bind(reason.trim())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(updated == 1, "no approved release with that ID");
    // An app held at this release goes back to its newest approved one.
    sqlx::query("update apps set pinned_release_id = null where pinned_release_id = $1")
        .bind(release_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        reviewer,
        "release.revoke",
        &release_id.to_string(),
        json!({ "reason": reason }),
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

async fn has_approved_release(tx: &mut sqlx::PgConnection, app_id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "select exists (select 1 from releases where app_id = $1 and status = 'approved')",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?)
}

/// Puts a reviewed draft in the store. Beta and Released apps need an approved release first;
/// Coming soon pages have no build (DESIGN §8).
pub async fn list_app(db: &PgPool, slug: &str, reviewer: &str) -> anyhow::Result<()> {
    list_app_as(db, slug, reviewer, false).await
}

/// Private Slop whose owner asked to make it public (SECURITY §12.3), once the admin panel has
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
    if going_public {
        let requested: bool = sqlx::query_scalar(
            "select visibility = 'private' and public_requested_at is not null from apps where id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(requested, "its owner hasn't asked to make it public");
        sqlx::query(
            "update apps set visibility = 'public', public_requested_at = null where id = $1",
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
    ensure!(
        !crate::catalog::is_private(&mut *tx, id).await?,
        "it's Private Slop: private apps are never listed in the store (unless the owner asks to make it public)"
    );
    if let Some(reason) = crate::frozen::frozen_reason(&mut *tx, id).await? {
        bail!("the app is frozen ({reason}); it can be listed once Slop can read its repository");
    }
    if crate::branches::needs_pulse(&mut tx, id).await? {
        ensure!(
            crate::branches::has_pulse(&mut tx, id).await?,
            "this community build's original counts players with Slop Pulse, so the build must \
             too (DESIGN §9): no Pulse handshake from it yet. The author launches it through \
             Slop once and the handshake is recorded."
        );
    }
    if stage != AppStage::ComingSoon {
        ensure!(
            has_approved_release(&mut tx, id).await?,
            "approve a release before listing a {stage:?} app: it has no approved release yet. \
             The maker submits one from Releases & progress, then it waits under Releases for \
             review. A Coming soon page can be listed without one."
        );
    }
    ensure!(
        !crate::media::has_pending(&mut tx, id).await?,
        "review the app's pictures first: a new set is waiting under Pictures to review"
    );
    sqlx::query("update apps set status = 'listed', updated_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
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

/// Moves an app to a new stage. Beta and Released need an approved release.
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
    if stage != AppStage::ComingSoon {
        ensure!(
            has_approved_release(&mut tx, id).await?,
            "approve a release first: Beta and Released apps need an approved release"
        );
    }
    sqlx::query(
        "update apps set stage = $2, stage_entered_at = now(), updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(stage)
    .execute(&mut *tx)
    .await?;
    let told = crate::interested::stage_moved(&mut tx, id, stage).await?;
    audit(
        &mut tx,
        reviewer,
        "app.stage",
        slug,
        json!({ "from": from, "to": stage, "interested_told": told }),
    )
    .await?;
    tx.commit().await?;
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
