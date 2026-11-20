// SPDX-License-Identifier: AGPL-3.0-or-later
//! Checks, then Publish (decided 2026-11).
//!
//! Every release goes through SLAPorSLOP's automatic checks (releases.rs `fetch_next`): its files
//! are downloaded, hashed, checked for a maker signature and scanned, Windows builds need
//! Microsoft Defender's check, a browser game's play address is checked, and the source is read
//! (source_scan.rs). No person reviews a release any more. The checks end in one of three ways
//! (`finish_checks`):
//!
//! - **ready**: it passed. Nothing is signed yet and players can't get it; its maker presses
//!   Publish (`publish`), signed in themselves: never an AI agent's token or session. Publishing
//!   signs the manifest, gives the store page the release's rating, ends a hold on an older
//!   release and makes it what players get (`releases::SERVING`, `catalog::PLAY_RELEASE`). It
//!   doesn't put a draft in the store, or move a Coming soon page to Beta: players get nothing
//!   until those happen too (manage.rs).
//! - **on hold** (`awaiting_review`, with a `release_holds` row): a scanner or Microsoft Defender
//!   flagged a file (SLAPorSLOP staff look, and can clear a false positive), a Windows build is
//!   still waiting for Defender's check (it goes on by itself once Slop Check reports), or it
//!   lowers the age rating from the last published release (staff check it first, AGE-7; their
//!   acceptance is recorded in `releases.rating_accepted_at`, `rating_accepted_by`).
//! - **failed**: something the maker has to change, said plainly.
//!
//! **A MUST FIX blocks Publish** (owner decision 2026-11-20): a ready release whose checks found a
//! `must` item (fixes.rs `of_release`: a possible secret, a licence problem, purchases, code from
//! outside SLAPorSLOP, going online without the store page saying so…) can't be published until
//! it's fixed. What the store page decides is judged against the page as it is when Publish is
//! pressed, so changing the page is enough for those; the rest need a new release. A false alarm
//! is for SLAPorSLOP staff: the maker asks them about the release (holds.rs), and a staff member
//! signed in can allow publishing that release anyway (`releases.must_fix_waived_at`). `should`
//! items never stop it, and neither does what the source check found that the last published
//! release didn't have (`new_concerns`): both are shown to the maker, and the latter logged.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use slop_manifest::{
    FileKind, Launch, MANIFEST_SCHEMA, ManifestFile, Platform, Purpose, ReleaseManifest,
};
use sqlx::{PgConnection, PgPool};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    AppState,
    admin::{self, ReviewRules},
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    release_progress::file_label,
    releases::{self, ReleaseEvent, ReleaseInfo},
    updates::AUTO_REVIEWER,
};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/apps/{slug}/releases/{release_id}/publish",
        post(publish_route),
    )
}

// ---------------------------------------------------------------------------------------------
// What the checks found

/// What holds a release back from being published.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldKind {
    /// SLAPorSLOP's scanner flagged the file (on hold, rejected or blocked) and it isn't cleared.
    Scan,
    /// Microsoft Defender flagged the Windows build and it isn't cleared.
    Defender,
    /// A Windows build with no Defender check yet, or one for a different file.
    DefenderMissing,
    /// The release lowers the age rating or drops a content label from the last published one,
    /// and SLAPorSLOP staff haven't accepted it yet (AGE-7: a lowered rating waits for a person).
    RatingLowered,
}

impl HoldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Defender => "defender",
            Self::DefenderMissing => "defender_missing",
            Self::RatingLowered => "rating_lowered",
        }
    }

    /// A hold's stored kind (`release_holds.kinds`); none for a question.
    pub fn parse(kind: &str) -> Option<Self> {
        [
            Self::Scan,
            Self::Defender,
            Self::DefenderMissing,
            Self::RatingLowered,
        ]
        .into_iter()
        .find(|k| k.as_str() == kind)
    }

    /// A finding or a lowered rating is for SLAPorSLOP staff to judge; a missing Defender check
    /// is the maker's to fix (it becomes a staff item only if the maker writes to staff).
    pub fn needs_staff(self) -> bool {
        matches!(self, Self::Scan | Self::Defender | Self::RatingLowered)
    }

    /// A hold on a file (cleared per file, re-scannable), rather than on the whole release.
    pub fn is_file(self) -> bool {
        !matches!(self, Self::RatingLowered)
    }
}

/// One hold: a file's, or the release's own (a lowered rating: no platform or file label).
#[derive(Clone, Debug, Serialize)]
pub struct Hold {
    pub kind: HoldKind,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub platform: String,
    /// "Windows (.zip)" (`release_progress::file_label`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file_label: String,
}

impl Hold {
    /// "a scanner flagged the Windows (.zip) build", for a one-line summary.
    pub fn describe(&self) -> String {
        let label = &self.file_label;
        match self.kind {
            HoldKind::Scan => format!("a scanner flagged the {label} build"),
            HoldKind::Defender => format!("Microsoft Defender flagged the {label} build"),
            HoldKind::DefenderMissing => {
                format!("the {label} build needs Microsoft Defender's check")
            }
            HoldKind::RatingLowered => {
                "it lowers the age rating, so SLAPorSLOP staff check it first".into()
            }
        }
    }
}

/// Where a release's checks leave it.
#[derive(Debug)]
pub enum Verdict {
    Ready,
    Held(Vec<Hold>),
    /// Something the maker has to change, in plain words.
    Failed(String),
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
    defender: Option<sqlx::types::Json<crate::scan::DefenderCheck>>,
    /// A reviewer cleared it as a false positive (`release_files.cleared_at`), with or without
    /// a note.
    cleared: bool,
    /// Microsoft Defender's detections in it were cleared with it (`defender_cleared_at`).
    defender_cleared: bool,
    /// Staff waived its missing Defender check (`defender_waived_at`).
    defender_waived: bool,
}

#[derive(Deserialize)]
struct PartRow {
    url: String,
    size: Option<u64>,
    sha256: Option<String>,
    attestation: Option<String>,
}

async fn file_rows(conn: &mut PgConnection, release_id: Uuid) -> sqlx::Result<Vec<FileRow>> {
    sqlx::query_as(
        "select platform, kind, url, file_name, exec, args, size_bytes, sha256, attestation, provenance_problem,
                (maker_signature - 'fingerprint' - 'key_name') as maker_signature, parts,
                scan_result, defender, cleared_at is not null as cleared,
                defender_cleared_at is not null as defender_cleared,
                defender_waived_at is not null as defender_waived
         from release_files where release_id = $1 order by platform",
    )
    .bind(release_id)
    .fetch_all(conn)
    .await
}

/// A file's parts, each fetched and hashed.
fn parts_of(f: &FileRow) -> Result<Vec<slop_manifest::FilePart>, String> {
    let label = file_label(&f.platform, &f.file_name);
    f.parts
        .as_ref()
        .map(|p| p.0.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|p| {
            Ok(slop_manifest::FilePart {
                url: p.url.clone(),
                size: p.size.ok_or_else(|| {
                    format!(
                        "A part of the {label} build wasn't downloaded. Send the release again."
                    )
                })?,
                sha256: p.sha256.clone().ok_or_else(|| {
                    format!("A part of the {label} build wasn't hashed. Send the release again.")
                })?,
                attestation: p.attestation.clone(),
            })
        })
        .collect()
}

/// Runs every gate on release `release_id` without signing anything: the gates staff approval
/// used to run (frozen app, files hashed and scanned, maker signatures, Microsoft Defender, a
/// browser game's address checks and the bad-link lists), then the rating: one lower than what
/// players are shown (`releases::rating_change`) holds it for staff until they accept that
/// change, or one no worse (`releases::rating_accepted`). A `Failed` gate beats a hold: the
/// build has to change anyway.
pub async fn evaluate(
    conn: &mut PgConnection,
    rules: ReviewRules,
    release_id: Uuid,
) -> anyhow::Result<Verdict> {
    let verdict = checks_verdict(conn, rules, release_id).await?;
    if matches!(verdict, Verdict::Failed(_)) {
        return Ok(verdict);
    }
    let Some(change) = releases::rating_change(conn, release_id).await? else {
        return Ok(verdict);
    };
    if releases::rating_accepted(conn, release_id, &change).await? {
        return Ok(verdict);
    }
    let rating = Hold {
        kind: HoldKind::RatingLowered,
        platform: String::new(),
        file_label: String::new(),
    };
    Ok(match verdict {
        Verdict::Held(mut holds) => {
            holds.push(rating);
            Verdict::Held(holds)
        }
        _ => Verdict::Held(vec![rating]),
    })
}

/// `evaluate` without the rating.
async fn checks_verdict(
    conn: &mut PgConnection,
    rules: ReviewRules,
    release_id: Uuid,
) -> anyhow::Result<Verdict> {
    let (app_id, play_address, address_checks): (Uuid, Option<String>, Option<serde_json::Value>) =
        sqlx::query_as("select app_id, play_address, address_checks from releases where id = $1")
            .bind(release_id)
            .fetch_one(&mut *conn)
            .await?;
    if let Some(reason) = crate::frozen::frozen_reason(&mut *conn, app_id).await? {
        return Ok(Verdict::Failed(format!(
            "The app is frozen ({}). Send it again once SLAPorSLOP can read the repository.",
            reason.trim_end_matches('.')
        )));
    }
    if let Some(address) = play_address {
        return address_verdict(conn, &address, address_checks.as_ref()).await;
    }
    let rows = file_rows(conn, release_id).await?;
    if rows.is_empty() {
        return Ok(Verdict::Failed("The release has no files.".into()));
    }
    let mut holds = Vec::new();
    for f in &rows {
        let label = file_label(&f.platform, &f.file_name);
        let Some(sha256) = f.sha256.as_deref().filter(|_| f.size_bytes.is_some()) else {
            return Ok(Verdict::Failed(format!(
                "The {label} build wasn't downloaded and hashed. Send the release again."
            )));
        };
        let scan = f
            .scan_result
            .as_deref()
            .and_then(crate::scan::ScanResult::parse);
        use crate::scan::ScanResult;
        match scan {
            Some(_) if f.cleared => {}
            Some(result) if rules.scanner.approvable(result) => {}
            Some(ScanResult::OnHold | ScanResult::Rejected | ScanResult::Blocked) => {
                holds.push(Hold {
                    kind: HoldKind::Scan,
                    platform: f.platform.clone(),
                    file_label: label.clone(),
                });
            }
            Some(_) | None => {
                return Ok(Verdict::Failed(format!(
                    "The {label} build wasn't scanned, and every file is scanned here. Send the release again."
                )));
            }
        }
        let parts = match parts_of(f) {
            Ok(parts) => parts,
            Err(why) => return Ok(Verdict::Failed(why)),
        };
        let parts_signed = !parts.is_empty() && parts.iter().all(|p| p.attestation.is_some());
        if rules.require_maker_signature
            && f.attestation.is_none()
            && f.maker_signature.is_none()
            && !parts_signed
        {
            return Ok(Verdict::Failed(format!(
                "The {label} build has no maker signature: {}",
                f.provenance_problem.as_deref().unwrap_or("not checked")
            )));
        }
        // Windows builds need Microsoft Defender's all-clear, so players' Windows runs them.
        // Clearing the scanner's finding doesn't cover Defender: its detections count as cleared
        // only when they were there to clear (`defender_cleared_at`), and a missing check only
        // when staff waived it (`defender_waived_at`).
        if rules.scanner.is_on() && f.platform == "windows" {
            let kind = match &f.defender {
                Some(d) if d.sha256.eq_ignore_ascii_case(sha256) => {
                    (!d.detections.is_empty() && !f.defender_cleared).then_some(HoldKind::Defender)
                }
                _ => (!f.defender_waived).then_some(HoldKind::DefenderMissing),
            };
            if let Some(kind) = kind {
                holds.push(Hold {
                    kind,
                    platform: f.platform.clone(),
                    file_label: label,
                });
            }
        }
    }
    Ok(if holds.is_empty() {
        Verdict::Ready
    } else {
        Verdict::Held(holds)
    })
}

/// A browser game's release: its automatic checks passed, and no bad-link list names the
/// address, any address it redirected through or where it ended up, checked again now since days
/// may have passed.
async fn address_verdict(
    conn: &mut PgConnection,
    address: &str,
    checks: Option<&serde_json::Value>,
) -> anyhow::Result<Verdict> {
    let Some(checks) = checks else {
        return Ok(Verdict::Failed(
            "The play address hasn't been checked. Send it again.".into(),
        ));
    };
    if checks["ok"].as_bool() != Some(true) {
        return Ok(Verdict::Failed(format!(
            "The play address didn't pass its checks: {}.",
            checks["problem"]
                .as_str()
                .unwrap_or("no details")
                .trim_end_matches('.')
        )));
    }
    let mut addresses = vec![address.to_string()];
    let hops = checks["hops"].as_array().into_iter().flatten();
    for hop in hops
        .chain([&checks["final_address"]])
        .filter_map(|h| h.as_str())
    {
        if !addresses.iter().any(|a| a == hop) {
            addresses.push(hop.to_string());
        }
    }
    if let Some((on, list)) = crate::bad_links::find(&mut *conn, &addresses)
        .await?
        .into_iter()
        .next()
    {
        return Ok(Verdict::Failed(format!(
            "{on} is on {list}, a list of known-bad links."
        )));
    }
    Ok(Verdict::Ready)
}

/// Release `release_id`'s manifest for downloads, signed with `key`, as approved (published) at
/// `now`. Every gate has passed (`evaluate`).
pub async fn sign_manifest(
    conn: &mut PgConnection,
    key: &slop_manifest::ed25519_dalek::SigningKey,
    release_id: Uuid,
    now: OffsetDateTime,
) -> anyhow::Result<slop_manifest::Signed> {
    #[derive(sqlx::FromRow)]
    struct ReleaseRow {
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
    let release: ReleaseRow = sqlx::query_as(
        "select a.id as app_id, a.slug, a.title, u.display_name as maker, a.repo_url, r.version,
                r.commit_sha, a.branch_kind, a.mods_folder, r.runtimes, g.id as game_id,
                g.slug as game_slug, g.title as game_title
         from releases r join apps a on a.id = r.app_id join users u on u.id = a.maker_id
              left join apps g on g.id = a.parent_app_id
         where r.id = $1",
    )
    .bind(release_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut files = Vec::new();
    for f in file_rows(conn, release_id).await? {
        let (Some(size), Some(sha256)) = (f.size_bytes, f.sha256.clone()) else {
            anyhow::bail!("the {} build hasn't been hashed", f.platform);
        };
        let parts = parts_of(&f).map_err(anyhow::Error::msg)?;
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
    anyhow::ensure!(!files.is_empty(), "release has no files");
    let is_mod = release.branch_kind.as_deref() == Some("addon");
    // A dedicated server installs and runs as an app of its own: neither a mod of its game nor
    // a build of it to play instead (docs/DEDICATED-SERVERS.md §7: the link stays out of the
    // manifest, which installed desktop apps couldn't read yet).
    let is_server = release.branch_kind.as_deref() == Some("server");
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
        app_slug: release.slug,
        app_title: release.title,
        maker: release.maker,
        repo_url: release.repo_url,
        release_id: release_id.to_string(),
        version: release.version,
        commit: release.commit_sha,
        files,
        // When it was published (docs/standard/release-manifest.md).
        approved_at: now.format(&Rfc3339)?,
        mod_for: original.clone().filter(|_| is_mod),
        build_of: original.filter(|_| !is_mod && !is_server),
        mods_folder: release
            .mods_folder
            .and_then(|f| serde_json::from_value(f).ok())
            .filter(|f: &slop_manifest::ModsFolder| !is_mod && f.is_valid()),
        runtimes: release.runtimes,
    };
    Ok(slop_manifest::sign(
        &manifest,
        Purpose::ReleaseManifest,
        key,
    ))
}

// ---------------------------------------------------------------------------------------------
// The end of the checks

/// What `finish_checks` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It passed: it waits for its maker to publish it.
    Ready,
    /// On hold. `opened` is true when the hold is new or holds for other reasons now (the maker
    /// was told); `needs_staff` puts it in the staff queue.
    Held {
        kinds: Vec<HoldKind>,
        needs_staff: bool,
        opened: bool,
    },
    Failed(String),
    /// A newer release was published meanwhile, so this one never can be: it's withdrawn as
    /// replaced by that version.
    Replaced(String),
    /// It wasn't being checked or on hold (withdrawn meanwhile, say): nothing was done.
    NotChecking,
}

/// The release a decision is about.
struct Subject {
    id: Uuid,
    app_id: Uuid,
    slug: String,
    version: String,
    created_at: OffsetDateTime,
    /// Slop Check can run for the app's repository (holds.rs `slop_check_runs`).
    slop_check: bool,
}

/// The end of a release's checks, in one transaction with the release row locked: `evaluate`,
/// then `ready`, a hold, or `failed`, and its maker is told. Called by the fetcher once every
/// file and the source are checked (with `AUTO_REVIEWER`), and again whenever what held it may
/// have changed (a reviewer clearing a finding, with the reviewer as `actor`). Only a release
/// being checked (`fetching`) or on hold (`awaiting_review`) is touched, so a withdrawal made
/// meanwhile stands.
pub async fn finish_checks(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    actor: &str,
) -> anyhow::Result<Outcome> {
    let mut tx = db.begin().await?;
    let outcome = finish_checks_in(&mut tx, rules, release_id, actor).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// `finish_checks` in the caller's transaction.
pub async fn finish_checks_in(
    conn: &mut PgConnection,
    rules: ReviewRules,
    release_id: Uuid,
    actor: &str,
) -> anyhow::Result<Outcome> {
    let row: Option<(String, Uuid, String, String, OffsetDateTime, String)> = sqlx::query_as(
        "select r.status::text, r.app_id, a.slug, r.version, r.created_at, a.repo_url
         from releases r join apps a on a.id = r.app_id where r.id = $1 for update of r",
    )
    .bind(release_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((status, app_id, slug, version, created_at, repo_url)) = row else {
        return Ok(Outcome::NotChecking);
    };
    if !matches!(status.as_str(), "fetching" | "awaiting_review") {
        return Ok(Outcome::NotChecking);
    }
    let subject = Subject {
        id: release_id,
        app_id,
        slug,
        version,
        created_at,
        slop_check: crate::holds::slop_check_runs(&repo_url),
    };
    // A newer release was published while this one was checked or on hold: Publish refuses an
    // older one, so rather than end ready (or held) for nothing, it's replaced now.
    if let Some(newer) = newer_out(conn, &subject).await? {
        let reason = format!("Replaced by {newer} before it was published.");
        sqlx::query(
            "update releases set status = 'withdrawn', reviewed_at = now(), status_reason = $2
             where id = $1",
        )
        .bind(release_id)
        .bind(&reason)
        .execute(&mut *conn)
        .await?;
        closed_as_replaced(conn, &subject, release_id, &subject.version, &newer, actor).await?;
        return Ok(Outcome::Replaced(newer));
    }
    let verdict = evaluate(conn, rules, release_id).await?;
    apply(conn, &subject, verdict, actor).await
}

/// Checks a ready release again, when something its checks rely on changed (newer signatures
/// match a program in it, rescan.rs): if it no longer passes, it goes where the checks leave it,
/// and its maker is told, as Publish would.
pub async fn recheck_ready(
    db: &PgPool,
    rules: ReviewRules,
    release_id: Uuid,
    actor: &str,
) -> anyhow::Result<Outcome> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String, String, OffsetDateTime, String)> = sqlx::query_as(
        "select r.app_id, a.slug, r.version, r.created_at, a.repo_url
         from releases r join apps a on a.id = r.app_id
         where r.id = $1 and r.status = 'ready' for update of r",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((app_id, slug, version, created_at, repo_url)) = row else {
        return Ok(Outcome::NotChecking);
    };
    let subject = Subject {
        id: release_id,
        app_id,
        slug,
        version,
        created_at,
        slop_check: crate::holds::slop_check_runs(&repo_url),
    };
    let outcome = match evaluate(&mut tx, rules, release_id).await? {
        Verdict::Ready => Outcome::Ready,
        other => apply(&mut tx, &subject, other, actor).await?,
    };
    tx.commit().await?;
    Ok(outcome)
}

/// The newest published (or since withdrawn by SLAPorSLOP) release of the app sent after `s`.
async fn newer_out(conn: &mut PgConnection, s: &Subject) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(
        "select version from releases
         where app_id = $1 and id <> $2 and status in ('approved', 'revoked') and created_at > $3
         order by created_at desc limit 1",
    )
    .bind(s.app_id)
    .bind(s.id)
    .bind(s.created_at)
    .fetch_optional(conn)
    .await
}

async fn apply(
    conn: &mut PgConnection,
    s: &Subject,
    verdict: Verdict,
    actor: &str,
) -> anyhow::Result<Outcome> {
    match verdict {
        Verdict::Ready => {
            sqlx::query(
                "update releases set status = 'ready', reviewed_at = now(), reviewed_by = $2,
                        status_reason = null
                 where id = $1",
            )
            .bind(s.id)
            .bind(actor)
            .execute(&mut *conn)
            .await?;
            let automatic = actor == AUTO_REVIEWER;
            if close_hold(
                conn,
                s.id,
                if automatic { "passed" } else { "cleared" },
                actor,
            )
            .await?
            {
                system_line(
                    conn,
                    s.id,
                    if automatic {
                        "Passed SLAPorSLOP's checks: it's ready for its maker to publish."
                    } else {
                        "Cleared by SLAPorSLOP staff: it's ready for its maker to publish."
                    },
                    None,
                )
                .await?;
            }
            let concerns = crate::source_scan::new_concerns(conn, s.id).await?;
            let rating = releases::rating_change(conn, s.id).await?;
            admin::audit(
                conn,
                actor,
                "release.ready",
                &s.id.to_string(),
                json!({ "app": s.slug, "version": s.version }),
            )
            .await?;
            if let Some(concerns) = &concerns {
                admin::audit(
                    conn,
                    actor,
                    "release.source_concerns",
                    &s.id.to_string(),
                    json!({ "release": s.id, "concerns": concerns }),
                )
                .await?;
            }
            // A held older release is replaced: the maker fixed it with this one.
            replace_older(conn, s, &["awaiting_review"], actor).await?;
            let pictures_pending = crate::media::has_pending(conn, s.app_id).await?;
            releases::tell_maker(
                conn,
                s.id,
                ReleaseEvent::Ready {
                    pictures_pending,
                    concerns: concerns.as_deref(),
                    rating: rating.as_ref(),
                },
            )
            .await?;
            Ok(Outcome::Ready)
        }
        Verdict::Held(holds) => {
            let mut kinds: Vec<HoldKind> = Vec::new();
            for h in &holds {
                if !kinds.contains(&h.kind) {
                    kinds.push(h.kind);
                }
            }
            let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
            let summary = format!(
                "On hold: {}.",
                holds
                    .iter()
                    .map(Hold::describe)
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            sqlx::query(
                "update releases set status = 'awaiting_review', status_reason = $2 where id = $1",
            )
            .bind(s.id)
            .bind(&summary)
            .execute(&mut *conn)
            .await?;
            let before: Option<(Vec<String>, String)> =
                sqlx::query_as("select kinds, status from release_holds where release_id = $1")
                    .bind(s.id)
                    .fetch_optional(&mut *conn)
                    .await?;
            // A missing Defender check becomes a staff item only once the maker writes to staff
            // (and until staff answer), unless Slop Check can't run for the repository: then
            // only staff can resolve it.
            let needs_staff: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "insert into release_holds (release_id, app_id, kinds, needs_staff)
                 values ($1, $2, $3, $4)
                 on conflict (release_id) do update set
                     kinds = excluded.kinds,
                     needs_staff = excluded.needs_staff
                         or (release_holds.status = 'open' and {unanswered}),
                     status = 'open', closed_at = null, closed_by = null
                 returning needs_staff",
                unanswered =
                    crate::holds::UNANSWERED.replace("h.release_id", "release_holds.release_id"),
            )))
            .bind(s.id)
            .bind(s.app_id)
            .bind(&names)
            .bind(crate::holds::kinds_need_staff(&kinds, s.slop_check))
            .fetch_one(&mut *conn)
            .await?;
            let opened = match &before {
                Some((was, status)) => status != "open" || *was != names,
                None => true,
            };
            if opened {
                system_line(
                    conn,
                    s.id,
                    &summary,
                    Some(json!({ "kinds": names, "files": holds })),
                )
                .await?;
                admin::audit(
                    conn,
                    actor,
                    "release.hold",
                    &s.id.to_string(),
                    json!({ "app": s.slug, "version": s.version, "kinds": names, "needs_staff": needs_staff }),
                )
                .await?;
                releases::tell_maker(conn, s.id, ReleaseEvent::Held { holds: &holds }).await?;
                if needs_staff {
                    crate::holds::notify_staff(conn, s.id, crate::holds::staff_reason(&kinds))
                        .await?;
                }
            }
            Ok(Outcome::Held {
                kinds,
                needs_staff,
                opened,
            })
        }
        Verdict::Failed(reason) => {
            sqlx::query("update releases set status = 'failed', status_reason = $2 where id = $1")
                .bind(s.id)
                .bind(&reason)
                .execute(&mut *conn)
                .await?;
            if close_hold(conn, s.id, "withdrawn", actor).await? {
                system_line(conn, s.id, &format!("It can't go on: {reason}"), None).await?;
            }
            releases::tell_maker(conn, s.id, ReleaseEvent::Failed { reason: &reason }).await?;
            Ok(Outcome::Failed(reason))
        }
    }
}

/// Closes release `release_id`'s open hold, if it has one, as `status` (passed, cleared,
/// rejected, replaced, withdrawn or answered). Returns whether there was one.
pub(crate) async fn close_hold(
    conn: &mut PgConnection,
    release_id: Uuid,
    status: &str,
    by: &str,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "update release_holds set status = $2, closed_at = now(), closed_by = $3,
                rescan_requested_at = null
         where release_id = $1 and status = 'open'",
    )
    .bind(release_id)
    .bind(status)
    .bind(by)
    .execute(conn)
    .await?
    .rows_affected()
        == 1)
}

/// Adds SLAPorSLOP's own line to a hold's log (the hold's row must exist).
pub(crate) async fn system_line(
    conn: &mut PgConnection,
    release_id: Uuid,
    body: &str,
    detail: Option<serde_json::Value>,
) -> sqlx::Result<()> {
    let body: String = body.chars().take(4000).collect();
    sqlx::query(
        "insert into release_hold_messages (id, release_id, author_id, from_staff, kind, body, detail)
         values ($1, $2, null, false, 'system', $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(release_id)
    .bind(body)
    .bind(detail)
    .execute(conn)
    .await?;
    Ok(())
}

/// Withdraws the app's releases older than `s` (by when they were sent) that are in one of
/// `statuses` and never reached players: "Replaced by {v} before it was published." Their holds
/// close as `replaced`, as do open questions about older releases that failed or were turned
/// down: `s` supersedes them. No notice: the hold log says it.
async fn replace_older(
    conn: &mut PgConnection,
    s: &Subject,
    statuses: &[&str],
    actor: &str,
) -> anyhow::Result<Vec<String>> {
    let reason = format!("Replaced by {} before it was published.", s.version);
    let replaced: Vec<(Uuid, String)> = sqlx::query_as(
        "update releases set status = 'withdrawn', reviewed_at = now(), status_reason = $4
         where app_id = $1 and id <> $2 and created_at < $3 and status::text = any($5)
         returning id, version",
    )
    .bind(s.app_id)
    .bind(s.id)
    .bind(s.created_at)
    .bind(&reason)
    .bind(statuses)
    .fetch_all(&mut *conn)
    .await?;
    for (id, version) in &replaced {
        closed_as_replaced(conn, s, *id, version, &s.version, actor).await?;
    }
    let questions: Vec<Uuid> = sqlx::query_scalar(
        "select h.release_id from release_holds h join releases r on r.id = h.release_id
         where r.app_id = $1 and r.id <> $2 and r.created_at < $3 and h.status = 'open'
           and r.status in ('failed', 'rejected')",
    )
    .bind(s.app_id)
    .bind(s.id)
    .bind(s.created_at)
    .fetch_all(&mut *conn)
    .await?;
    for id in questions {
        if close_hold(conn, id, "replaced", actor).await? {
            system_line(
                conn,
                id,
                &format!("Closed: version {} replaced it.", s.version),
                None,
            )
            .await?;
        }
    }
    Ok(replaced.into_iter().map(|(_, v)| v).collect())
}

/// The rest of withdrawing release `id` (version `version`) of `s`'s app as replaced by `by`:
/// its hold closes as `replaced`, with the reason in its log, and it's audited.
async fn closed_as_replaced(
    conn: &mut PgConnection,
    s: &Subject,
    id: Uuid,
    version: &str,
    by: &str,
    actor: &str,
) -> anyhow::Result<()> {
    if close_hold(conn, id, "replaced", actor).await? {
        system_line(
            conn,
            id,
            &format!("Replaced by {by} before it was published."),
            None,
        )
        .await?;
    }
    admin::audit(
        conn,
        actor,
        "release.replace",
        &id.to_string(),
        json!({ "app": s.slug, "version": version, "by": by }),
    )
    .await
}

/// Releases left waiting when this version was deployed (SPEC "checks, then Publish" §9), run
/// once at startup and safe to run again: every release on hold without a `release_holds` row
/// (waiting for a person's review under the old flow) has its checks end now (`finish_checks`),
/// so it's ready, on hold, failed or replaced by a newer published one, and its maker is told;
/// and every pending request to hold players at a published release (or to follow the newest
/// again) is applied, as such requests now are at once, unless a newer release was published
/// after it was made: holding players back then isn't what the maker asked for, so it's closed
/// and they're told. Repository moves still wait for staff.
pub async fn settle_after_upgrade(state: &crate::AppState) -> anyhow::Result<()> {
    let rules = ReviewRules::from_config(&state.config);
    let waiting: Vec<Uuid> = sqlx::query_scalar(
        "select r.id from releases r
         where r.status = 'awaiting_review'
           and not exists (select 1 from release_holds h where h.release_id = r.id)
         order by r.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    for id in waiting {
        if let Err(err) = finish_checks(&state.db, rules, id, AUTO_REVIEWER).await {
            tracing::error!(release = %id, error = ?err, "settling a waiting release");
        }
    }
    let stale: Vec<(Uuid, Uuid, String, String, String, String)> = sqlx::query_as(
        "select c.id, a.maker_id, a.slug, a.title, r.version, n.version
         from app_changes c join apps a on a.id = c.app_id join releases r on r.id = c.release_id
         join lateral (select version, published_at from releases n
                       where n.app_id = c.app_id and n.status = 'approved'
                       order by n.published_at desc limit 1) n on true
         where c.status = 'pending' and c.kind = 'release' and n.published_at > c.created_at
         order by c.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, maker, slug, title, asked, newest) in stale {
        let mut tx = state.db.begin().await?;
        let closed = sqlx::query(
            "update app_changes set status = 'cancelled', reviewed_at = now(), status_reason = $2
             where id = $1 and status = 'pending'",
        )
        .bind(id)
        .bind(format!(
            "Version {newest} was published after you asked, so players get it instead."
        ))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if closed == 1 {
            admin::audit(
                &mut tx,
                AUTO_REVIEWER,
                "app.release_change_superseded",
                &slug,
                json!({ "change": id, "hold_at": asked, "published": newest }),
            )
            .await?;
            crate::notify::send(
                &mut tx,
                crate::notify::Notice {
                    user_id: maker,
                    category: "apps",
                    title: format!("{title}: players aren't held at {asked}"),
                    body: format!(
                        "You asked to hold players at {asked}, then published {newest}, so players get {newest}. To hold them at an earlier version, ask again on the Manage page."
                    ),
                    link: format!("/publish/{slug}/manage"),
                    collapse_key: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
    }
    let changes: Vec<Uuid> = sqlx::query_scalar(
        "select c.id from app_changes c left join releases r on r.id = c.release_id
         where c.status = 'pending' and c.kind = 'release'
           and (c.release_id is null or r.status = 'approved')
         order by c.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    for id in changes {
        if let Err(err) = crate::manage::approve_change(&state.db, id, AUTO_REVIEWER).await {
            tracing::warn!(change = %id, error = %err, "settling a pending release change");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Publish

/// "Publish": the maker gives players a release that passed its checks. A person signed in on
/// SLAPorSLOP only: never an AI agent's token or session (it's not in `maker_agents::ALLOWED`
/// either), so neither a stolen token nor a compromised repository can put a build in front of
/// players. While the beta's invite code is required, they need to have entered it (invites.rs).
async fn publish_route(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ReleaseInfo>> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let found: bool =
        sqlx::query_scalar("select exists (select 1 from releases where id = $1 and app_id = $2)")
            .bind(release_id)
            .bind(app_id)
            .fetch_one(&state.db)
            .await?;
    if !found {
        return Err(ApiError::NotFound);
    }
    if user.agent.is_some() {
        return Err(ApiError::Refused(
            "Publishing is for the app's maker, signed in on SLAPorSLOP: an AI agent can't publish. Tell your maker to press Publish on the Releases page.",
        ));
    }
    crate::invites::require(&state.db, user.id).await?;
    let cfg = &state.config;
    publish(
        &state.db,
        cfg.releases.signing_key.as_ref(),
        ReviewRules::from_config(cfg),
        release_id,
        user.id,
    )
    .await?;
    Ok(Json(releases::load_release(&state, release_id).await?))
}

/// Publishes release `release_id` for its app's maker `by`, in one transaction with the app and
/// the release locked. Refuses (409) unless it's `ready`, the app isn't frozen, its new pictures
/// are reviewed, its source check has a result, no newer release is out, it has no `must` fix
/// (`must_fix_refusal`) unless staff allowed it anyway, and every gate still passes, a lowered
/// rating included (staff must have accepted it: defence in depth, since the
/// published release it's compared with may have changed since its checks). A gate that fails
/// now marks it failed; a hold puts it back on hold. Then it signs
/// the manifest (a browser game's address has none), makes it what players get, gives the store
/// page its rating, ends a hold on an older release, withdraws older releases that never reached
/// players, and tells the maker.
pub async fn publish(
    db: &PgPool,
    key: Option<&slop_manifest::ed25519_dalek::SigningKey>,
    rules: ReviewRules,
    release_id: Uuid,
    by: Uuid,
) -> ApiResult<()> {
    let mut tx = db.begin().await?;
    let app: Option<(Uuid, Uuid, String, Option<Uuid>, String)> = sqlx::query_as(
        "select a.id, a.maker_id, a.slug, a.pinned_release_id, a.repo_url from apps a
         where a.id = (select app_id from releases where id = $1) for update",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((app_id, _, slug, pinned, repo_url)) = app.filter(|a| a.1 == by) else {
        return Err(ApiError::NotFound);
    };
    let (status, version, created_at, play_address, source): (
        String,
        String,
        OffsetDateTime,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "select status::text, version, created_at, play_address, repo_scan ->> 'status'
         from releases where id = $1 for update",
    )
    .bind(release_id)
    .fetch_one(&mut *tx)
    .await?;
    if status != "ready" {
        return Err(ApiError::Conflict(
            "only a release that passed its checks can be published".into(),
        ));
    }
    if let Some(reason) = crate::frozen::frozen_reason(&mut *tx, app_id).await? {
        return Err(ApiError::Conflict(format!(
            "the app is frozen ({}), so it can't be published until SLAPorSLOP can read the repository again",
            reason.trim_end_matches('.')
        )));
    }
    if crate::media::has_pending(&mut tx, app_id).await? {
        return Err(ApiError::Conflict(
            "Its new pictures are waiting for review. You can publish once they're reviewed."
                .into(),
        ));
    }
    // So what the source check found is always shown before publishing. "Couldn't read it"
    // (not_checked) is a result too, as is a week of failed tries (source_scan.rs `run_once`).
    if !matches!(source.as_deref(), Some("checked" | "not_checked")) {
        return Err(ApiError::Conflict(
            "SLAPorSLOP hasn't read this release's source yet. It tries again every hour, and you can publish once it has a result."
                .into(),
        ));
    }
    let newer: Option<String> = sqlx::query_scalar(
        "select version from releases
         where app_id = $1 and id <> $2 and status in ('approved', 'revoked') and created_at > $3
         order by created_at desc limit 1",
    )
    .bind(app_id)
    .bind(release_id)
    .bind(created_at)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(newer) = newer {
        return Err(ApiError::Conflict(format!(
            "Version {newer} is newer and already out"
        )));
    }
    let found = crate::fixes::of_release(&mut tx, release_id).await?;
    let blocking = found.blocking();
    if !blocking.is_empty() {
        return Err(ApiError::Conflict(must_fix_refusal(&version, &blocking)));
    }
    let rating = releases::rating_change(&mut tx, release_id).await?;
    let subject = Subject {
        id: release_id,
        app_id,
        slug: slug.clone(),
        version: version.clone(),
        created_at,
        slop_check: crate::holds::slop_check_runs(&repo_url),
    };
    let actor = by.to_string();
    match evaluate(&mut tx, rules, release_id).await? {
        Verdict::Ready => {}
        other => {
            // Something changed since the checks (a bad-link list names the address now, the
            // app froze): it goes back to where the checks leave it.
            let outcome = apply(&mut tx, &subject, other, &actor).await?;
            tx.commit().await?;
            return Err(ApiError::Conflict(match outcome {
                Outcome::Failed(reason) => format!("It can't be published: {reason}"),
                Outcome::Held { kinds, .. } if kinds.contains(&HoldKind::RatingLowered) => {
                    "It lowers the age rating, so a SLAPorSLOP staff member checks it before it can be published"
                        .into()
                }
                _ => "It's on hold again: see the release".into(),
            }));
        }
    }
    // The manifest's approval time, to the second (published_at is the database's own clock,
    // finer, so two releases published in one second still have an order).
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .map_err(|e| ApiError::Internal(e.into()))?;
    let mut detail = json!({ "app": slug, "version": version });
    let signed = match &play_address {
        Some(address) => {
            detail["play_address"] = json!(address);
            None
        }
        None => {
            let key = key.ok_or(ApiError::Unavailable(
                "SLAPorSLOP can't publish releases right now",
            ))?;
            let signed = sign_manifest(&mut tx, key, release_id, now).await?;
            detail["key_id"] = json!(signed.key_id);
            Some(signed)
        }
    };
    sqlx::query(
        "update releases set status = 'approved', signed_manifest = $2, published_at = now(),
                published_by = $3
         where id = $1",
    )
    .bind(release_id)
    .bind(signed.as_ref().map(sqlx::types::Json))
    .bind(&actor)
    .execute(&mut *tx)
    .await?;
    // The store page shows the rating of the release players get (AGE-7), never below what
    // SLAPorSLOP staff set it to.
    sqlx::query(
        "update apps a set min_age = greatest(r.min_age, coalesce(a.min_age_floor, 0)),
                content_labels = r.content_labels
                    || array(select l from unnest(a.content_labels_floor) l where l <> all (r.content_labels)),
                updated_at = now()
         from releases r where r.id = $1 and a.id = r.app_id and r.min_age is not null",
    )
    .bind(release_id)
    .execute(&mut *tx)
    .await?;
    // Publishing ends a hold at an older release: players get this one.
    if let Some(pinned) = pinned {
        let held_at: Option<String> =
            sqlx::query_scalar("select version from releases where id = $1")
                .bind(pinned)
                .fetch_optional(&mut *tx)
                .await?;
        sqlx::query("update apps set pinned_release_id = null, updated_at = now() where id = $1")
            .bind(app_id)
            .execute(&mut *tx)
            .await?;
        detail["ended_hold"] = json!(held_at);
    }
    // Older releases still being checked go too: the fetcher leaves a withdrawn one alone.
    let replaced = replace_older(
        &mut tx,
        &subject,
        &["ready", "awaiting_review", "fetching"],
        &actor,
    )
    .await?;
    if !replaced.is_empty() {
        detail["replaced"] = json!(replaced);
    }
    // A question its maker asked staff about what had to be fixed ends with publishing it.
    if close_hold(&mut tx, release_id, "passed", &actor).await? {
        system_line(&mut tx, release_id, "Closed: its maker published it.", None).await?;
    }
    // Published with what must be fixed in it, as SLAPorSLOP staff allowed.
    if let Some(at) = found.waived_at {
        detail["must_fix_waived_at"] = json!(at.format(&Rfc3339).ok());
        detail["must_fix"] = json!(found.with("must"));
    }
    detail["rating"] = json!(rating);
    detail["concerns"] = json!(crate::source_scan::new_concerns(&mut tx, release_id).await?);
    admin::audit(
        &mut tx,
        &actor,
        "release.publish",
        &release_id.to_string(),
        detail,
    )
    .await?;
    releases::tell_maker(&mut tx, release_id, ReleaseEvent::Published).await?;
    tx.commit().await?;
    Ok(())
}

/// Why Publish refuses a release with `must` items: each one, and the ways on (no dead end).
pub fn must_fix_refusal(version: &str, blocking: &[crate::fixes::FixBrief]) -> String {
    let problems: Vec<&str> = blocking.iter().map(|f| f.problem.trim()).collect();
    format!(
        "Version {version} can't be published until {} {} fixed: {} To fix says how. If one is wrong, ask SLAPorSLOP staff about this release on the Releases page.",
        crate::fixes::things(blocking.len()),
        if blocking.len() == 1 { "is" } else { "are" },
        problems.join(" ")
    )
}
