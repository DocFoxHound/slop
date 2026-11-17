// SPDX-License-Identifier: AGPL-3.0-or-later
//! Releases on hold, and failed releases a maker asks about (decided 2026-11; publish.rs ends a
//! release's checks).
//!
//! - **What holds it, in plain words** (`problems`): each scanner finding, Microsoft Defender's
//!   result or missing check, and a lowered age rating, with what the maker can do. Built here so
//!   the website and the maker's AI agent say the same thing.
//! - **Run the scans again** (`POST …/rescan`): SLAPorSLOP keeps no copy of a release's files
//!   (publisher.md), so a re-run downloads the same files again from the maker's release page,
//!   checks their SHA-256 hasn't changed, scans them with today's signatures and deletes them,
//!   as a re-scan of a live release does (rescan.rs). Up to 3 per release, 6 hours apart
//!   (30 minutes after the scanner couldn't finish), 10 a day per account. It only ever clears a
//!   scanner's hold: Defender can't be run here, and a lowered rating waits for staff. The fetcher
//!   runs re-runs when no release is waiting (`rescan_next`), one at a time, sharing the scanner.
//! - **A conversation with SLAPorSLOP staff** (`release_hold_messages`): the maker (or their
//!   agent, which staff see) says why a finding is a false positive, or asks about a failed
//!   release (CON-16: a person looks at an automated refusal on request); staff answer, accept
//!   or reject. Plain text, as admin_messages.rs: no control characters but new lines, at most 3
//!   links, shown as text and handed to staff agents as `untrusted_text`. 5 a day per release,
//!   20 a day per account. Staff notices never quote the maker.
//! - **The staff queue** is the open holds that need staff: a scanner or Defender finding, a
//!   lowered age rating, or a maker who wrote to staff (a missing Defender check is the maker's
//!   to fix until then). Makers waiting for an answer come first, then the oldest.
//!
//! The conversation is kept with its release and deleted with the app; when the maker's account
//! closes, their own messages are blanked (account.rs).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_messages::{self, UNTRUSTED},
    admin_web::AdminUser,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    publish::{self, HoldKind, Outcome},
    release_progress::file_label,
    releases::{self, RatingChange, ReleaseEvent},
    updates::AUTO_REVIEWER,
};

/// Re-runs of the scanner per release (the table's check says the same).
pub const MAX_RESCANS: i16 = 3;
/// Between a held file's last scan and the next re-run: the scanner's signatures refresh daily
/// (deploy/scanner/update.sh), so sooner rarely finds anything new.
const RESCAN_SPACING: Duration = Duration::hours(6);
/// When the last scan didn't finish (scan.rs), which usually passes with a second try.
const RESCAN_SPACING_UNFINISHED: Duration = Duration::minutes(30);
/// A re-run that hasn't moved on for this long is presumed dead and tried again (as the
/// fetcher's own lease, releases.rs). Each file renews it, and each has `RESCAN_FILE_DEADLINE`.
const RESCAN_LEASE: Duration = Duration::minutes(15);
/// One file's re-run, download and scan: shorter than a first check's (scan.rs allows 30 minutes
/// a scan), so a slow release page or a file built to be slow can't keep the scanner from
/// everyone else's releases for long.
const RESCAN_FILE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// Re-runs across SLAPorSLOP in a day (`release_rescan_runs`): each downloads files again and
/// keeps the scanner busy. Past it, re-runs wait their turn for the next day.
const RESCANS_PER_DAY: i64 = 200;
/// What a maker may write about one release in a day (counted in the database).
const MAKER_MESSAGES_PER_RELEASE: i64 = 5;
const MAX_MAKER_MESSAGE: usize = 2000;
const MAX_STAFF_MESSAGE: usize = 4000;
/// A closed account's own messages, kept so the conversation still reads (account.rs).
pub const DELETED_MESSAGE: &str = "(message deleted when the account closed)";
/// The scanner's own words when it couldn't finish (scan.rs).
const UNFINISHED: &str = "the scanner couldn't finish";
/// How the scanner's summary (deploy/scanner/scan.py) says who looks next, before and since
/// checks-then-Publish: the conversation and the actions say what happens now, so it's cut off.
const WHO_LOOKS: [&str; 2] = [" A person checks it", " SLAPorSLOP staff look at it"];

/// Whether Slop Check, which gets Microsoft Defender's check of a Windows build, can run for a
/// repository: it runs on GitHub Actions, so only for a GitHub repository. Elsewhere a missing
/// Defender check is for SLAPorSLOP staff to check by hand (and waive) from the start.
pub(crate) fn slop_check_runs(repo_url: &str) -> bool {
    slop_provenance::github_repo_path(repo_url).is_some()
}

/// Whether a hold of `kinds` is for SLAPorSLOP staff without the maker writing to them: a finding
/// or a lowered rating, or a missing Defender check where Slop Check can't run.
pub(crate) fn kinds_need_staff(kinds: &[HoldKind], slop_check: bool) -> bool {
    kinds.iter().any(|k| k.needs_staff())
        || (!slop_check && kinds.contains(&HoldKind::DefenderMissing))
}

/// `kinds_need_staff` for a hold's stored kinds.
fn stored_kinds_need_staff(kinds: &[String], slop_check: bool) -> bool {
    let kinds: Vec<HoldKind> = kinds.iter().filter_map(|k| HoldKind::parse(k)).collect();
    kinds_need_staff(&kinds, slop_check)
}

/// The scanner's summary without its old "who looks next" ending.
fn without_who_looks(detail: &str) -> &str {
    WHO_LOOKS
        .iter()
        .find_map(|w| detail.split_once(w).map(|(before, _)| before))
        .unwrap_or(detail)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/releases/{release_id}/hold", get(maker_hold))
        .route(
            "/apps/{slug}/releases/{release_id}/hold/messages",
            post(maker_write),
        )
        .route(
            "/apps/{slug}/releases/{release_id}/rescan",
            post(request_rescan),
        )
        .route("/admin/releases/{id}/hold", get(staff_hold))
        .route("/admin/releases/{id}/hold/messages", post(staff_answer))
}

// ---------------------------------------------------------------------------------------------
// What holds it

/// One thing holding a release (or why its checks failed), for its maker and their agent.
#[derive(Debug, Serialize)]
pub struct Problem {
    /// scan, defender, defender_missing, rating_lowered, failed or rejected.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// "Windows (.zip)".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_label: Option<String>,
    /// One line: what holds it.
    pub what: String,
    /// What was found and what it means, in plain sentences.
    pub explained: String,
    /// What the maker can do about it: rescan, write_to_staff, run_slop_check, new_release.
    pub you_can: Vec<&'static str>,
}

/// A release file, as the problems need it.
#[derive(sqlx::FromRow)]
struct FileFacts {
    platform: String,
    file_name: String,
    sha256: Option<String>,
    scan_result: Option<String>,
    scan_detail: Option<String>,
    scan_report: Option<Value>,
    defender: Option<sqlx::types::Json<crate::scan::DefenderCheck>>,
    cleared: bool,
    defender_cleared: bool,
    defender_waived: bool,
    checked_at: Option<OffsetDateTime>,
}

impl FileFacts {
    /// SLAPorSLOP's scanner holds it (on hold, rejected or blocked) and staff haven't cleared it.
    fn scan_held(&self) -> bool {
        !self.cleared
            && matches!(
                self.scan_result.as_deref(),
                Some("on_hold" | "rejected" | "blocked")
            )
    }

    /// Its last scan didn't finish (scan.rs): a second try usually works.
    fn unfinished(&self) -> bool {
        self.scan_detail
            .as_deref()
            .is_some_and(|d| d.starts_with(UNFINISHED))
    }
}

async fn file_facts(conn: &mut PgConnection, release_id: Uuid) -> sqlx::Result<Vec<FileFacts>> {
    sqlx::query_as(
        "select platform, file_name, sha256, scan_result, scan_detail, scan_report, defender,
                cleared_at is not null as cleared, defender_cleared_at is not null as defender_cleared,
                defender_waived_at is not null as defender_waived, checked_at
         from release_files where release_id = $1 order by platform",
    )
    .bind(release_id)
    .fetch_all(conn)
    .await
}

/// `text` in backticks, as the scanner's names and file paths are shown.
fn quoted(text: &str) -> String {
    format!("`{}`", text.replace('`', "'"))
}

/// A scanner hold, explained from its report's findings, or from what the scanner said.
fn scan_problem(f: &FileFacts) -> Problem {
    let label = file_label(&f.platform, &f.file_name);
    let findings: Vec<&Value> = f
        .scan_report
        .as_ref()
        .and_then(|r| r["findings"].as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut said: Vec<String> = Vec::new();
    for finding in findings.iter().take(5) {
        let engine = finding["engine"].as_str().unwrap_or("A scanner");
        let name = quoted(finding["name"].as_str().unwrap_or("something"));
        let file = quoted(finding["file"].as_str().unwrap_or(&f.file_name));
        said.push(match engine {
            "ClamAV" => format!(
                "ClamAV, a virus scanner, matched the signature {name} in {file}. Scanners sometimes match harmless programs."
            ),
            "YARA" => format!(
                "A YARA rule ({name}) matched {file}. These rules look for patterns seen in malware. They also match packed programs, for example ones compressed with UPX."
            ),
            other => format!("{other} flagged {name} in {file}."),
        });
    }
    if findings.len() > 5 {
        said.push(format!("And {} more findings.", findings.len() - 5));
    }
    let detail = f.scan_detail.as_deref().unwrap_or("").trim();
    let mut you_can = vec!["rescan", "write_to_staff", "new_release"];
    let (what, explained) = if !said.is_empty() {
        (
            format!("A scanner flagged the {label} build"),
            said.join(" "),
        )
    } else if let Some(rest) = detail.strip_prefix(UNFINISHED) {
        // "the scanner couldn't finish (it took too long); …"
        let why = rest
            .trim_start()
            .strip_prefix('(')
            .and_then(|r| r.split_once(')'))
            .map(|(why, _)| why)
            .unwrap_or("no reason given");
        (
            format!("SLAPorSLOP's scanner couldn't finish the {label} build"),
            format!(
                "SLAPorSLOP's scanner couldn't finish ({why}). Running the scans again usually fixes this."
            ),
        )
    } else if let Some(rest) = detail.strip_prefix("Not fully scanned:") {
        let why = without_who_looks(rest).trim().trim_end_matches('.');
        let mut explained = format!("SLAPorSLOP couldn't scan all of it: {why}.");
        // The scanner's archive limits (deploy/scanner/scan.py): running it again won't help.
        if why.contains("unpack") || why.contains("nested") {
            explained.push_str(" Make the archive smaller or less nested and send a new version.");
            you_can.retain(|w| *w != "rescan");
        }
        (
            format!("SLAPorSLOP couldn't scan all of the {label} build"),
            explained,
        )
    } else {
        (
            format!("A scanner flagged the {label} build"),
            if detail.is_empty() {
                "SLAPorSLOP's scanner held it without saying why.".to_string()
            } else {
                without_who_looks(detail).to_string()
            },
        )
    };
    Problem {
        kind: "scan",
        platform: Some(f.platform.clone()),
        file_label: Some(label),
        what,
        explained,
        you_can,
    }
}

/// Microsoft Defender flagged it, or its check is missing (`slop_check`: Slop Check can run for
/// the repository, so the maker can get the check themselves).
fn defender_problem(f: &FileFacts, kind: HoldKind, slop_check: bool) -> Problem {
    let label = file_label(&f.platform, &f.file_name);
    if kind == HoldKind::DefenderMissing && !slop_check {
        return Problem {
            kind: "defender_missing",
            platform: Some(f.platform.clone()),
            file_label: Some(label),
            what: "The Windows build needs Microsoft Defender's check".into(),
            explained: "Slop Check runs on GitHub Actions, so it can't run for a repository outside GitHub yet. SLAPorSLOP staff check the Windows build by hand; you can write to them below.".into(),
            you_can: vec!["write_to_staff", "new_release"],
        };
    }
    if kind == HoldKind::DefenderMissing {
        return Problem {
            kind: "defender_missing",
            platform: Some(f.platform.clone()),
            file_label: Some(label.clone()),
            what: format!("The {label} build is waiting for Microsoft Defender's check"),
            explained: "Windows builds need Microsoft Defender's check. Run Slop Check in your repository (see /publish/guide#slop-check). This release goes on by itself as soon as the result arrives.".into(),
            you_can: vec!["run_slop_check", "write_to_staff"],
        };
    }
    let flagged: Vec<String> = f
        .defender
        .as_ref()
        .map(|d| {
            d.0.detections
                .iter()
                .take(5)
                .map(|x| format!("{} in {}", quoted(&x.threat), quoted(&x.file)))
                .collect()
        })
        .unwrap_or_default();
    Problem {
        kind: "defender",
        platform: Some(f.platform.clone()),
        file_label: Some(label.clone()),
        what: format!("Microsoft Defender flagged the {label} build"),
        explained: format!(
            "Microsoft Defender flagged {}. SLAPorSLOP can't run Defender again on the same file. Submit the file to Microsoft, then build again, or tell SLAPorSLOP staff why it's a false positive.",
            if flagged.is_empty() {
                "it".to_string()
            } else {
                flagged.join(", ")
            }
        ),
        you_can: vec!["write_to_staff", "new_release"],
    }
}

/// A lowered age rating (or a content label dropped) from the last published release (AGE-7).
fn rating_problem(r: &RatingChange) -> Problem {
    let dropped = if r.dropped_labels.is_empty() {
        String::new()
    } else {
        let words: Vec<&str> = r
            .dropped_labels
            .iter()
            .map(|l| crate::catalog::content_label_words(l))
            .collect();
        format!(" (drops: {})", words.join(", "))
    };
    let change = if r.to_age < r.from_age {
        format!(
            "Lowers the age rating from {} to {}{dropped}.",
            RatingChange::age(r.from_age),
            RatingChange::age(r.to_age)
        )
    } else {
        format!(
            "Lowers the rating of the last published release{dropped}; the age stays {}.",
            RatingChange::age(r.to_age)
        )
    };
    Problem {
        kind: "rating_lowered",
        platform: None,
        file_label: None,
        what: "It lowers the age rating".into(),
        explained: format!(
            "{change} A SLAPorSLOP staff member checks a lowered rating before you can publish."
        ),
        you_can: vec!["write_to_staff", "new_release"],
    }
}

/// Everything holding release `release_id` (as its hold's `kinds` last found it), or why its
/// checks failed or staff turned it down, in plain words. Empty for a release that's none of
/// these.
pub async fn problems(conn: &mut PgConnection, release_id: Uuid) -> anyhow::Result<Vec<Problem>> {
    let (status, reason, repo_url): (String, Option<String>, String) = sqlx::query_as(
        "select r.status::text, r.status_reason, a.repo_url
         from releases r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(release_id)
    .fetch_one(&mut *conn)
    .await?;
    let ended = match status.as_str() {
        "failed" => Some(("failed", "Its checks failed")),
        "rejected" => Some(("rejected", "SLAPorSLOP staff turned it down")),
        _ => None,
    };
    if let Some((kind, what)) = ended {
        return Ok(vec![Problem {
            kind,
            platform: None,
            file_label: None,
            what: what.into(),
            explained: reason.unwrap_or_else(|| "No reason was recorded.".into()),
            you_can: vec!["new_release", "write_to_staff"],
        }]);
    }
    if status != "awaiting_review" {
        return Ok(Vec::new());
    }
    let slop_check = slop_check_runs(&repo_url);
    let kinds: Vec<String> =
        sqlx::query_scalar("select kinds from release_holds where release_id = $1")
            .bind(release_id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or_default();
    let has = |k: HoldKind| kinds.iter().any(|x| x == k.as_str());
    let mut out = Vec::new();
    for f in file_facts(conn, release_id).await? {
        if has(HoldKind::Scan) && f.scan_held() {
            out.push(scan_problem(&f));
        }
        if f.platform != "windows" {
            continue;
        }
        let same_file = match (&f.defender, &f.sha256) {
            (Some(d), Some(sha)) => d.0.sha256.eq_ignore_ascii_case(sha),
            _ => false,
        };
        // As publish.rs `evaluate` judges it: a scanner finding cleared doesn't clear Defender.
        if has(HoldKind::Defender)
            && same_file
            && !f.defender_cleared
            && f.defender
                .as_ref()
                .is_some_and(|d| !d.0.detections.is_empty())
        {
            out.push(defender_problem(&f, HoldKind::Defender, slop_check));
        } else if has(HoldKind::DefenderMissing) && !same_file && !f.defender_waived {
            out.push(defender_problem(&f, HoldKind::DefenderMissing, slop_check));
        }
    }
    if has(HoldKind::RatingLowered)
        && let Some(r) = releases::rating_change(conn, release_id).await?
    {
        out.push(rating_problem(&r));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// The hold as its maker and staff see it

#[derive(sqlx::FromRow)]
struct HoldRow {
    status: String,
    kinds: Vec<String>,
    needs_staff: bool,
    maker_asked_at: Option<OffsetDateTime>,
    rescans_used: i16,
    rescan_requested_at: Option<OffsetDateTime>,
    rescan_started_at: Option<OffsetDateTime>,
    last_rescan_at: Option<OffsetDateTime>,
    unanswered: bool,
}

/// The holds in SLAPorSLOP staff's queue, as SQL on `release_holds h` joined to `releases r`:
/// open and needing staff, on a release on hold or a failed or turned-down one its maker asked
/// about.
pub(crate) const NEEDS_STAFF: &str =
    "h.status = 'open' and h.needs_staff and r.status in ('awaiting_review', 'failed', 'rejected')";

/// The staff queue's order, with `UNANSWERED` selected as `unanswered`: makers waiting for an
/// answer first (the newest question first), then the oldest hold.
pub(crate) const QUEUE_ORDER: &str =
    "unanswered desc, h.maker_asked_at desc nulls last, h.opened_at";

/// Whether the maker's last message came after staff last wrote, as SQL on `release_holds h`.
pub(crate) const UNANSWERED: &str = "coalesce((select max(m.created_at) from release_hold_messages m
                    where m.release_id = h.release_id and m.kind = 'message' and not m.from_staff), '-infinity')
          > coalesce((select max(m.created_at) from release_hold_messages m
                       where m.release_id = h.release_id and m.from_staff), '-infinity')";

async fn hold_row(conn: &mut PgConnection, release_id: Uuid) -> sqlx::Result<Option<HoldRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select h.status, h.kinds, h.needs_staff, h.maker_asked_at, h.rescans_used,
                h.rescan_requested_at, h.rescan_started_at, h.last_rescan_at, {UNANSWERED} as unanswered
         from release_holds h where h.release_id = $1"
    )))
    .bind(release_id)
    .fetch_optional(conn)
    .await
}

/// A release's hold in brief, on its `ReleaseInfo` for its maker (releases.rs; `list_releases`
/// leaves it out for everyone else).
#[derive(Debug, Serialize)]
pub struct HoldSummary {
    /// open, or how it ended: passed, cleared, rejected, replaced, withdrawn or answered.
    pub status: String,
    pub kinds: Vec<String>,
    pub needs_staff: bool,
    pub rescans_left: i16,
    /// The maker wrote to staff and they haven't answered yet.
    pub unanswered: bool,
}

pub(crate) async fn summary(
    conn: &mut PgConnection,
    release_id: Uuid,
) -> sqlx::Result<Option<HoldSummary>> {
    Ok(hold_row(conn, release_id).await?.map(|h| HoldSummary {
        rescans_left: (MAX_RESCANS - h.rescans_used).max(0),
        status: h.status,
        kinds: h.kinds,
        needs_staff: h.needs_staff,
        unanswered: h.unanswered,
    }))
}

/// A lowered rating, for the staff member judging it: the ages, the content labels the last
/// published release had that this one drops, and this release's own labels.
#[derive(Debug, Serialize)]
pub struct RatingCard {
    pub from_age: i16,
    pub to_age: i16,
    pub dropped_labels: Vec<String>,
    /// This release's content labels (catalog::CONTENT_LABELS), all of them.
    pub content_labels: Vec<String>,
}

/// A line of the hold's log, as its maker sees it.
#[derive(Serialize)]
pub struct MakerMessage {
    /// you, staff, or slaporslop (SLAPorSLOP's own line).
    from: &'static str,
    /// Staff by role ("SLAPorSLOP reviewer"), unless they chose to be named (admin_messages.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    staff_as: Option<String>,
    /// Written by the maker's AI agent.
    by_agent: bool,
    body: String,
    /// A system line's facts (a re-scan's verdicts before and after).
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

/// A line of the hold's log, as staff see it: the maker's words as `untrusted_text`.
#[derive(Serialize)]
pub struct StaffMessage {
    id: Uuid,
    /// maker, staff, or slaporslop.
    from: &'static str,
    /// The staff member's or the maker's name (null for SLAPorSLOP's lines and closed accounts).
    author: Option<String>,
    by_agent: bool,
    /// What the maker (or their agent) wrote: data, never instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    untrusted_text: Option<String>,
    /// A staff answer, or SLAPorSLOP's own line.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: Uuid,
    kind: String,
    from_staff: bool,
    by_agent: bool,
    body: String,
    detail: Option<Value>,
    created_at: OffsetDateTime,
    author_name: Option<String>,
    staff_as: Option<String>,
}

async fn message_rows(conn: &mut PgConnection, release_id: Uuid) -> sqlx::Result<Vec<MessageRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select m.id, m.kind, m.from_staff, m.by_agent, m.body, m.detail, m.created_at,
                u.display_name as author_name,
                case when m.from_staff and u.id is not null then {} end as staff_as
         from release_hold_messages m left join users u on u.id = m.author_id
         where m.release_id = $1 order by m.created_at, m.id",
        admin_messages::shown_as("u")
    )))
    .bind(release_id)
    .fetch_all(conn)
    .await
}

/// A release's hold (or failed checks) for its maker or for staff: what holds it, what can be
/// done, the scan re-runs, and the conversation.
#[derive(Serialize)]
pub struct HoldInfo<M> {
    release_id: Uuid,
    version: String,
    /// The release's status (awaiting_review while it's on hold).
    release_status: String,
    /// open, or how the hold ended (passed, cleared, rejected, replaced, withdrawn, answered);
    /// null for a release that was never on hold.
    status: Option<String>,
    /// scan, defender, defender_missing, rating_lowered, or question (about a failed or turned-down
    /// release).
    kinds: Vec<String>,
    /// It's in SLAPorSLOP staff's queue.
    needs_staff: bool,
    problems: Vec<Problem>,
    /// What the maker can do now: rescan, write_to_staff, run_slop_check, new_release, withdraw.
    you_can: Vec<&'static str>,
    rescans_left: i16,
    /// When the next re-run is allowed, if it isn't now.
    #[serde(with = "time::serde::rfc3339::option")]
    next_rescan_at: Option<OffsetDateTime>,
    /// A re-run is waiting for the scanner, or running.
    rescan_requested: bool,
    rescan_running: bool,
    /// When the maker last wrote to staff.
    #[serde(with = "time::serde::rfc3339::option")]
    asked_staff_at: Option<OffsetDateTime>,
    /// The maker wrote and staff haven't answered since.
    unanswered: bool,
    /// For a lowered rating: what it lowers.
    #[serde(skip_serializing_if = "Option::is_none")]
    rating: Option<RatingCard>,
    /// For staff and their agents: how to treat the maker's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    untrusted: Option<&'static str>,
    messages: Vec<M>,
}

pub type MakerHold = HoldInfo<MakerMessage>;
pub type StaffHold = HoldInfo<StaffMessage>;

/// When the held files were last scanned, and whether the last scan didn't finish.
fn last_scan(files: &[FileFacts], hold: &HoldRow) -> (Option<OffsetDateTime>, bool) {
    let held: Vec<&FileFacts> = files.iter().filter(|f| f.scan_held()).collect();
    let last = held
        .iter()
        .filter_map(|f| f.checked_at)
        .chain(hold.last_rescan_at)
        .max();
    (last, held.iter().any(|f| f.unfinished()))
}

/// When a re-run is allowed: `None` now, or the time; `Err` when it can't be at all.
fn rescan_allowed_at(
    files: &[FileFacts],
    hold: &HoldRow,
    now: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, &'static str> {
    if hold.status != "open" || !hold.kinds.iter().any(|k| k == "scan") {
        return Err("Only a release on hold because of SLAPorSLOP's scanner can be scanned again.");
    }
    if hold.rescan_requested_at.is_some() {
        return Err("The scans are already running again on this release.");
    }
    if hold.rescans_used >= MAX_RESCANS {
        return Err(
            "No re-runs left: SLAPorSLOP runs the scans again up to 3 times per release. Write to SLAPorSLOP staff, or send a new version.",
        );
    }
    let (last, unfinished) = last_scan(files, hold);
    let spacing = if unfinished {
        RESCAN_SPACING_UNFINISHED
    } else {
        RESCAN_SPACING
    };
    Ok(last.map(|t| t + spacing).filter(|next| *next > now))
}

/// Builds the hold of release `release_id` (which must exist) for its maker or staff.
async fn hold_info<M>(
    conn: &mut PgConnection,
    scanning: bool,
    release_id: Uuid,
    message: impl Fn(MessageRow) -> M,
    for_staff: bool,
) -> anyhow::Result<HoldInfo<M>> {
    let (version, release_status): (String, String) =
        sqlx::query_as("select version, status::text from releases where id = $1")
            .bind(release_id)
            .fetch_one(&mut *conn)
            .await?;
    let hold = hold_row(conn, release_id).await?;
    let files = file_facts(conn, release_id).await?;
    let mut problems = problems(conn, release_id).await?;
    let now = OffsetDateTime::now_utc();
    let open = hold.as_ref().is_some_and(|h| h.status == "open");
    let rescan = match &hold {
        Some(h) if scanning => rescan_allowed_at(&files, h, now),
        _ => Err(""),
    };
    let mut you_can: Vec<&'static str> = Vec::new();
    for p in &mut problems {
        if rescan.is_err() {
            p.you_can.retain(|w| *w != "rescan");
        }
        for w in &p.you_can {
            if !you_can.contains(w) {
                you_can.push(w);
            }
        }
    }
    if (open || matches!(release_status.as_str(), "failed" | "rejected"))
        && !you_can.contains(&"write_to_staff")
    {
        you_can.push("write_to_staff");
    }
    if matches!(
        release_status.as_str(),
        "fetching" | "awaiting_review" | "ready"
    ) {
        you_can.push("withdraw");
    }
    let rating = match &hold {
        Some(h) if open && h.kinds.iter().any(|k| k == "rating_lowered") => {
            let labels: Option<Vec<String>> =
                sqlx::query_scalar("select content_labels from releases where id = $1")
                    .bind(release_id)
                    .fetch_one(&mut *conn)
                    .await?;
            releases::rating_change(conn, release_id)
                .await?
                .map(|r| RatingCard {
                    from_age: r.from_age,
                    to_age: r.to_age,
                    dropped_labels: r.dropped_labels,
                    content_labels: labels.unwrap_or_default(),
                })
        }
        _ => None,
    };
    let messages = message_rows(conn, release_id)
        .await?
        .into_iter()
        .map(message)
        .collect();
    Ok(HoldInfo {
        release_id,
        version,
        release_status,
        status: hold.as_ref().map(|h| h.status.clone()),
        kinds: hold.as_ref().map(|h| h.kinds.clone()).unwrap_or_default(),
        needs_staff: open && hold.as_ref().is_some_and(|h| h.needs_staff),
        problems,
        you_can,
        rescans_left: hold
            .as_ref()
            .map_or(MAX_RESCANS, |h| (MAX_RESCANS - h.rescans_used).max(0)),
        next_rescan_at: rescan.ok().flatten(),
        rescan_requested: hold
            .as_ref()
            .is_some_and(|h| h.rescan_requested_at.is_some()),
        rescan_running: hold.as_ref().is_some_and(|h| {
            h.rescan_requested_at.is_some()
                && h.rescan_started_at.is_some_and(|t| t > now - RESCAN_LEASE)
        }),
        asked_staff_at: hold.as_ref().and_then(|h| h.maker_asked_at),
        unanswered: hold.as_ref().is_some_and(|h| h.unanswered),
        rating,
        untrusted: for_staff.then_some(UNTRUSTED),
        messages,
    })
}

fn maker_message(m: MessageRow) -> MakerMessage {
    let (from, staff_as) = if m.kind == "system" {
        ("slaporslop", None)
    } else if m.from_staff {
        (
            "staff",
            Some(m.staff_as.unwrap_or_else(|| "SLAPorSLOP staff".into())),
        )
    } else {
        ("you", None)
    };
    MakerMessage {
        from,
        staff_as,
        by_agent: m.by_agent,
        body: m.body,
        detail: m.detail,
        at: m.created_at,
    }
}

fn staff_message(m: MessageRow) -> StaffMessage {
    let (from, untrusted_text, text) = if m.kind == "system" {
        ("slaporslop", None, Some(m.body))
    } else if m.from_staff {
        ("staff", None, Some(m.body))
    } else {
        ("maker", Some(m.body), None)
    };
    StaffMessage {
        id: m.id,
        from,
        author: if from == "slaporslop" {
            None
        } else {
            m.author_name
        },
        by_agent: m.by_agent,
        untrusted_text,
        text,
        detail: m.detail,
        at: m.created_at,
    }
}

pub async fn maker_view(
    conn: &mut PgConnection,
    scanning: bool,
    release_id: Uuid,
) -> anyhow::Result<MakerHold> {
    hold_info(conn, scanning, release_id, maker_message, false).await
}

pub async fn staff_view(
    conn: &mut PgConnection,
    scanning: bool,
    release_id: Uuid,
) -> anyhow::Result<StaffHold> {
    hold_info(conn, scanning, release_id, staff_message, true).await
}

/// The owner's release `release_id` of app `slug`, or 404.
async fn owned_release(
    state: &AppState,
    slug: &str,
    user: &CurrentUser,
    release_id: Uuid,
) -> ApiResult<Uuid> {
    let (app_id, _) = crate::updates::owned(state, slug, user).await?;
    let found: bool =
        sqlx::query_scalar("select exists (select 1 from releases where id = $1 and app_id = $2)")
            .bind(release_id)
            .bind(app_id)
            .fetch_one(&state.db)
            .await?;
    if found {
        Ok(app_id)
    } else {
        Err(ApiError::NotFound)
    }
}

async fn maker_hold(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<MakerHold>> {
    owned_release(&state, &slug, &user, release_id).await?;
    let mut conn = state.db.acquire().await?;
    let scanning = state.config.releases.scanner.is_on();
    Ok(Json(maker_view(&mut conn, scanning, release_id).await?))
}

// ---------------------------------------------------------------------------------------------
// The conversation

/// Adds a message to a hold's log (nothing, for a release that has no hold).
pub(crate) async fn add_message(
    conn: &mut PgConnection,
    release_id: Uuid,
    author: Option<Uuid>,
    from_staff: bool,
    by_agent: bool,
    body: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into release_hold_messages (id, release_id, author_id, from_staff, by_agent, kind, body)
         select $1, $2, $3, $4, $5, 'message', $6
         where exists (select 1 from release_holds where release_id = $2)",
    )
    .bind(Uuid::now_v7())
    .bind(release_id)
    .bind(author)
    .bind(from_staff)
    .bind(by_agent)
    .bind(body)
    .execute(conn)
    .await?;
    Ok(())
}

/// What staff are told a hold is about, in a few words (never the maker's).
pub(crate) fn staff_reason(kinds: &[HoldKind]) -> &'static str {
    if kinds.contains(&HoldKind::Scan) {
        "a scanner finding"
    } else if kinds.contains(&HoldKind::Defender) {
        "a Microsoft Defender finding"
    } else if kinds.contains(&HoldKind::RatingLowered) {
        "a lowered age rating"
    } else {
        "waiting for Microsoft Defender's check"
    }
}

/// Tells SLAPorSLOP staff who review (the reviewer and owner roles) that something about a held
/// release needs them: one unread notice each, merged, never with the maker's messages (they may
/// be junk, and notices are emailed). It names the app by its title, cleaned as staff-facing text
/// is (admin_messages.rs: one line, no control or reordering characters), else by its address,
/// and the release by its version (checked when it was sent).
pub(crate) async fn notify_staff(
    conn: &mut PgConnection,
    release_id: Uuid,
    why: &str,
) -> sqlx::Result<()> {
    let Some((title, slug, version)): Option<(String, String, String)> = sqlx::query_as(
        "select a.title, a.slug, r.version from releases r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(release_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    let title = admin_messages::clean("title", &title, 80, true).unwrap_or(slug);
    let staff: Vec<Uuid> = sqlx::query_scalar(
        "select u.id from users u
         where u.is_admin and u.closed_at is null and u.suspended_at is null
           and exists (select 1 from staff_roles r where r.user_id = u.id and r.role in ('reviewer', 'owner'))",
    )
    .fetch_all(&mut *conn)
    .await?;
    for user_id in staff {
        crate::notify::send(
            &mut *conn,
            crate::notify::Notice {
                user_id,
                category: "ops",
                title: "Releases on hold".into(),
                body: format!("Newest: {title} {version} ({why})."),
                link: "/admin?tab=releases".into(),
                collapse_key: Some("release-holds".into()),
            },
        )
        .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    body: String,
}

/// The maker (or their agent with a publish token) writes to SLAPorSLOP staff about a release on
/// hold, saying why a finding is a false positive, or asks about a release whose checks failed or
/// that staff turned down (CON-16: a person looks again on request). That puts it in staff's
/// queue; a missing Defender check becomes a staff item this way.
async fn maker_write(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
    Json(req): Json<Write>,
) -> ApiResult<Json<MakerHold>> {
    let app_id = owned_release(&state, &slug, &user, release_id).await?;
    let body = admin_messages::clean_agent_text("body", &req.body, MAX_MAKER_MESSAGE)?;
    let mut tx = state.db.begin().await?;
    let status: String =
        sqlx::query_scalar("select status::text from releases where id = $1 for update")
            .bind(release_id)
            .fetch_one(&mut *tx)
            .await?;
    let hold: Option<String> =
        sqlx::query_scalar("select status from release_holds where release_id = $1 for update")
            .bind(release_id)
            .fetch_optional(&mut *tx)
            .await?;
    let question = match (hold.as_deref(), status.as_str()) {
        (Some("open"), _) => false,
        (_, "failed" | "rejected") => true,
        _ => {
            return Err(ApiError::Conflict(
                "You can write to SLAPorSLOP staff about a release on hold, one whose checks failed, or one they turned down."
                    .into(),
            ));
        }
    };
    // Five a day per release, counted here so a restart doesn't reset it.
    let today: Vec<OffsetDateTime> = sqlx::query_scalar(
        "select created_at from release_hold_messages
         where release_id = $1 and kind = 'message' and not from_staff and created_at > now() - interval '1 day'
         order by created_at",
    )
    .bind(release_id)
    .fetch_all(&mut *tx)
    .await?;
    if today.len() as i64 >= MAKER_MESSAGES_PER_RELEASE {
        let next = today[0] + Duration::DAY;
        let now = OffsetDateTime::now_utc();
        let seconds = u64::try_from((next - now).whole_seconds().max(1)).unwrap_or(1);
        return Err(ApiError::NotYet {
            message: format!(
                "You've written 5 messages about this release today. The next is allowed at {} (in {}).",
                admin_messages::stamp(next),
                admin_messages::in_words(seconds)
            ),
            retry_after: seconds,
        });
    }
    crate::limits::check(&state, user.id, crate::limits::Action::HoldMessage)?;
    if question {
        // A question about a failed release: a thread of its own, for staff to answer.
        sqlx::query(
            "insert into release_holds (release_id, app_id, kinds, needs_staff, maker_asked_at)
             values ($1, $2, array['question'], true, now())
             on conflict (release_id) do update set
                 kinds = array['question'], status = 'open', needs_staff = true,
                 maker_asked_at = now(), closed_at = null, closed_by = null,
                 rescan_requested_at = null, rescan_started_at = null",
        )
        .bind(release_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "update release_holds set needs_staff = true, maker_asked_at = now() where release_id = $1",
        )
        .bind(release_id)
        .execute(&mut *tx)
        .await?;
    }
    add_message(
        &mut tx,
        release_id,
        Some(user.id),
        false,
        user.agent.is_some(),
        &body,
    )
    .await?;
    admin::audit(
        &mut tx,
        &user.id.to_string(),
        "release.hold_message",
        &release_id.to_string(),
        json!({ "app": slug, "question": question, "by_agent": user.agent.is_some() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    notify_staff(
        &mut tx,
        release_id,
        match (question, status.as_str()) {
            (true, "rejected") => "the maker asks about a release staff turned down",
            (true, _) => "the maker asks about a failed release",
            _ => "the maker wrote",
        },
    )
    .await?;
    tx.commit().await?;
    let scanning = state.config.releases.scanner.is_on();
    let mut conn = state.db.acquire().await?;
    Ok(Json(maker_view(&mut conn, scanning, release_id).await?))
}

async fn staff_hold(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<StaffHold>> {
    let mut conn = state.db.acquire().await?;
    let found: bool = sqlx::query_scalar("select exists (select 1 from releases where id = $1)")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if !found {
        return Err(ApiError::NotFound);
    }
    let scanning = state.config.releases.scanner.is_on();
    Ok(Json(staff_view(&mut conn, scanning, id).await?))
}

/// Staff answer the maker in a hold's conversation (the maker is told). An answer to a question
/// about a failed or turned-down release closes it; the maker may ask again. Answering a hold
/// that's in staff's queue only because the maker wrote (a missing Defender check Slop Check can
/// get) takes it out of the queue: it's the maker's again, until they write back.
async fn staff_answer(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(req): Json<Write>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    let body = admin_messages::clean("body", &req.body, MAX_STAFF_MESSAGE, false)?;
    let mut tx = state.db.begin().await?;
    // The release first, then its hold, as everywhere else (no deadlock with finish_checks).
    let repo_url: Option<String> = sqlx::query_scalar(
        "select a.repo_url from releases r join apps a on a.id = r.app_id where r.id = $1 for update of r",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let hold: Option<(String, Vec<String>)> =
        sqlx::query_as("select status, kinds from release_holds where release_id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (Some(repo_url), Some((status, kinds))) = (repo_url, hold) else {
        return Err(ApiError::NotFound);
    };
    if status != "open" {
        return Err(ApiError::Conflict(
            "this release has no open hold or question to answer".into(),
        ));
    }
    add_message(
        &mut tx,
        id,
        Some(admin.user_id),
        true,
        admin.token.is_some(),
        &body,
    )
    .await?;
    let question = kinds == ["question"];
    if question {
        publish::close_hold(&mut tx, id, "answered", &admin.actor).await?;
    } else if !stored_kinds_need_staff(&kinds, slop_check_runs(&repo_url)) {
        sqlx::query("update release_holds set needs_staff = false where release_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        "release.hold_answer",
        &id.to_string(),
        json!({ "release": id, "question": question }),
    )
    .await
    .map_err(ApiError::Internal)?;
    releases::tell_maker(&mut tx, id, ReleaseEvent::StaffReplied).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Running the scans again

/// The maker asks SLAPorSLOP to run its scanner again on a release its scanner held (a publish
/// token may too): it waits for the fetcher (`rescan_next`).
async fn request_rescan(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<MakerHold>> {
    owned_release(&state, &slug, &user, release_id).await?;
    let scanning = state.config.releases.scanner.is_on();
    if !scanning {
        return Err(ApiError::Conflict(
            "SLAPorSLOP's scanner is off right now, so it can't scan again.".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    let hold: Option<HoldRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select h.status, h.kinds, h.needs_staff, h.maker_asked_at, h.rescans_used,
                h.rescan_requested_at, h.rescan_started_at, h.last_rescan_at, {UNANSWERED} as unanswered
         from release_holds h where h.release_id = $1 for update"
    )))
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(hold) = hold else {
        return Err(ApiError::Conflict(
            "Only a release on hold because of SLAPorSLOP's scanner can be scanned again.".into(),
        ));
    };
    let files = file_facts(&mut tx, release_id).await?;
    match rescan_allowed_at(&files, &hold, OffsetDateTime::now_utc()) {
        Err(why) => return Err(ApiError::Conflict(why.into())),
        Ok(Some(next)) => {
            return Err(ApiError::Conflict(format!(
                "Next re-run allowed at {}.",
                admin_messages::stamp(next)
            )));
        }
        Ok(None) => {}
    }
    crate::limits::check(&state, user.id, crate::limits::Action::Rescan)?;
    sqlx::query(
        "update release_holds set rescans_used = rescans_used + 1, rescan_requested_at = now(),
                rescan_started_at = null
         where release_id = $1",
    )
    .bind(release_id)
    .execute(&mut *tx)
    .await?;
    publish::system_line(
        &mut tx,
        release_id,
        "Running SLAPorSLOP's scanner again on the held files, as the maker asked.",
        None,
    )
    .await?;
    admin::audit(
        &mut tx,
        &user.id.to_string(),
        "release.rescan_requested",
        &release_id.to_string(),
        json!({ "app": slug, "by_agent": user.agent.is_some() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    state.fetch_wakeup.notify_one();
    let mut conn = state.db.acquire().await?;
    Ok(Json(maker_view(&mut conn, scanning, release_id).await?))
}

#[derive(sqlx::FromRow)]
struct HeldFile {
    platform: String,
    url: String,
    file_name: String,
    parts: Option<Value>,
    sha256: Option<String>,
    scan_result: Option<String>,
}

/// What became of one file's re-run.
enum Rerun {
    /// Scanned again: its result before and after.
    Scanned { before: String, after: String },
    /// The file on the release page isn't the one checked: the hold stays.
    Changed,
    /// It couldn't be downloaded.
    Unreachable(String),
    /// It took longer than `RESCAN_FILE_DEADLINE`.
    TooSlow,
    /// SLAPorSLOP's shared GitHub allowance is low: try again later.
    Later,
    /// The hold ended meanwhile (the release was withdrawn, accepted or replaced): the result
    /// isn't recorded.
    HoldEnded,
}

/// Whether release `release_id` is still on hold with its hold open, as SQL bound to $1.
const STILL_HELD: &str =
    "exists (select 1 from release_holds h join releases r on r.id = h.release_id
                where h.release_id = $1 and h.status = 'open' and r.status = 'awaiting_review')";

/// Runs one re-run the maker asked for, if one is waiting and SLAPorSLOP hasn't run its day's
/// share (`RESCANS_PER_DAY`). Returns whether it did any work. Called by the fetcher when no
/// release is waiting to be checked (releases.rs `run_fetcher`).
pub async fn rescan_next(state: &AppState) -> anyhow::Result<bool> {
    let scanner = state.config.releases.scanner;
    if !scanner.is_on() {
        return Ok(false);
    }
    let today: i64 = sqlx::query_scalar(
        "select count(*) from release_rescan_runs where started_at > now() - interval '1 day'",
    )
    .fetch_one(&state.db)
    .await?;
    if today >= RESCANS_PER_DAY {
        return Ok(false);
    }
    // The attempt is recorded as it starts: the next re-run is spaced from it, whatever happens.
    let claimed: Option<(Uuid, Uuid, bool)> = sqlx::query_as(
        "update release_holds set rescan_started_at = now(), last_rescan_at = now()
         where release_id = (
             select release_id from release_holds
             where status = 'open' and rescan_requested_at is not null
               and (rescan_started_at is null or rescan_started_at < now() - $1)
             order by rescan_requested_at limit 1 for update skip locked)
         returning release_id, app_id, rescan_given_back",
    )
    .bind(RESCAN_LEASE)
    .fetch_optional(&state.db)
    .await?;
    let Some((release_id, app_id, given_back_before)) = claimed else {
        return Ok(false);
    };
    sqlx::query("insert into release_rescan_runs (release_id) values ($1)")
        .bind(release_id)
        .execute(&state.db)
        .await?;
    let files: Vec<HeldFile> = sqlx::query_as(
        "select platform, url, file_name, parts, sha256, scan_result from release_files
         where release_id = $1 and scan_result in ('on_hold', 'rejected', 'blocked') and cleared_at is null
         order by platform",
    )
    .bind(release_id)
    .fetch_all(&state.db)
    .await?;
    let mut lines: Vec<String> = Vec::new();
    let mut stopped = None;
    let mut give_back = false;
    let mut ended = false;
    for f in &files {
        let label = file_label(&f.platform, &f.file_name);
        // Each file renews the lease, so a re-run of several files isn't taken for a dead one.
        sqlx::query("update release_holds set rescan_started_at = now() where release_id = $1")
            .bind(release_id)
            .execute(&state.db)
            .await?;
        let path = std::env::temp_dir().join(format!("slop-rescan-{}", Uuid::now_v7()));
        let rerun = tokio::time::timeout(
            RESCAN_FILE_DEADLINE,
            rerun_file(state, app_id, release_id, f, &path),
        )
        .await
        .unwrap_or(Rerun::TooSlow);
        // Whatever happened, SLAPorSLOP keeps no copy (publisher.md).
        let _ = tokio::fs::remove_file(&path).await;
        match rerun {
            Rerun::Later => {
                // Left claimed: it's tried again once the lease runs out.
                tracing::info!(release = %release_id, "re-scan waits for GitHub's allowance");
                return Ok(false);
            }
            Rerun::HoldEnded => {
                ended = true;
                break;
            }
            Rerun::Scanned { before, after } => {
                let mut tx = state.db.begin().await?;
                let line = format!(
                    "{label}: scanned again with today's signatures: {}.",
                    match after.as_str() {
                        "passed" | "note" => "nothing found",
                        _ => "still flagged",
                    }
                );
                publish::system_line(
                    &mut tx,
                    release_id,
                    &line,
                    Some(json!({ "platform": f.platform, "before": before, "after": after })),
                )
                .await?;
                admin::audit(
                    &mut tx,
                    AUTO_REVIEWER,
                    "release.rescan_held",
                    &release_id.to_string(),
                    json!({ "release": release_id, "platform": f.platform, "before": before, "after": after }),
                )
                .await?;
                tx.commit().await?;
                lines.push(line);
            }
            Rerun::Changed => {
                stopped = Some(format!(
                    "{label}: the file on your release page isn't the one SLAPorSLOP checked (its SHA-256 changed). A changed build is a new version: submit it."
                ));
                break;
            }
            Rerun::TooSlow => {
                stopped = Some(format!(
                    "{label}: running the scans again took longer than {} minutes, so SLAPorSLOP stopped. This re-run counts.",
                    RESCAN_FILE_DEADLINE.as_secs() / 60
                ));
                break;
            }
            Rerun::Unreachable(why) => {
                // Given back once per hold, and only when nothing was scanned: a maker can't
                // make downloads fail to re-run for free.
                give_back = !given_back_before && lines.is_empty();
                stopped = Some(format!(
                    "{label}: SLAPorSLOP couldn't download it from your release page again ({why}). {}",
                    if give_back {
                        "This re-run doesn't count: try again later."
                    } else if !lines.is_empty() {
                        "The files before it were scanned, so this re-run counts."
                    } else {
                        "This re-run counts: one that couldn't download was given back already."
                    }
                ));
                break;
            }
        }
    }
    let mut tx = state.db.begin().await?;
    // The release first, then its hold, as finish_checks and the routes lock them.
    sqlx::query("select 1 from releases where id = $1 for update")
        .bind(release_id)
        .execute(&mut *tx)
        .await?;
    let held: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select {STILL_HELD}")))
        .bind(release_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "update release_holds set rescan_requested_at = null, rescan_started_at = null,
                rescans_used = case when $2 then greatest(rescans_used - 1, 0) else rescans_used end,
                rescan_given_back = rescan_given_back or $3
         where release_id = $1",
    )
    .bind(release_id)
    // Nothing was held any more (cleared meanwhile): nothing ran, so no run is spent.
    .bind(give_back || files.is_empty())
    .bind(give_back)
    .execute(&mut *tx)
    .await?;
    // Withdrawn, accepted or replaced meanwhile: nothing more to say, to the maker or staff.
    if ended || !held {
        tx.commit().await?;
        return Ok(true);
    }
    if let Some(line) = &stopped {
        publish::system_line(&mut tx, release_id, line, None).await?;
    }
    let outcome = publish::finish_checks_in(
        &mut tx,
        admin::ReviewRules::from_config(&state.config),
        release_id,
        AUTO_REVIEWER,
    )
    .await?;
    // Passed (or failed): the hold is closed and its maker told (publish.rs). Still held for
    // the same reasons: the log and the maker say so here, and staff hear it if it's theirs (a
    // hold that changed was told to both by publish.rs).
    if let Outcome::Held {
        opened: false,
        needs_staff,
        ..
    } = &outcome
        && (stopped.is_some() || !lines.is_empty())
    {
        publish::system_line(&mut tx, release_id, "Still on hold.", None).await?;
        let said = match &stopped {
            Some(line) => line.clone(),
            None => format!("{} It's still on hold.", lines.join(" ")),
        };
        releases::tell_maker(
            &mut tx,
            release_id,
            ReleaseEvent::Rescanned {
                ran: stopped.is_none(),
                line: &said,
            },
        )
        .await?;
        if *needs_staff {
            notify_staff(&mut tx, release_id, "a scan re-run finished").await?;
        }
    }
    tx.commit().await?;
    Ok(true)
}

/// Downloads one held file again to `path` (as releases.rs fetched it: the release page's own
/// link, or the maker's token for a private repository elsewhere; never an attestation or commit
/// lookup, which cost GitHub API requests and can't have changed), checks it's the same file,
/// scans it and records the new result, while the release is still on hold. The caller deletes
/// `path`.
async fn rerun_file(
    state: &AppState,
    app_id: Uuid,
    release_id: Uuid,
    f: &HeldFile,
    path: &std::path::Path,
) -> Rerun {
    let part_urls: Vec<String> = f
        .parts
        .as_ref()
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p["url"].as_str().map(String::from))
        .collect();
    let first = part_urls.first().unwrap_or(&f.url);
    if releases::shared_github_budget_low(state, first).await {
        return Rerun::Later;
    }
    let auth = match crate::repo_tokens::auth_for(state, app_id, &f.url).await {
        Ok(auth) => auth,
        Err(e) => return Rerun::Unreachable(format!("{e:#}")),
    };
    let downloaded = if part_urls.is_empty() {
        let source = match &auth {
            Some(_) => Ok(f.url.clone()),
            None => releases::download_link(state, &f.url).await,
        };
        match source {
            Ok(source) => {
                releases::download_full(
                    &state.fetch_http,
                    &source,
                    auth.as_deref(),
                    state.config.releases.max_file_bytes,
                    Some(path),
                )
                .await
            }
            Err(e) => Err(e),
        }
    } else {
        releases::download_parts(state, &part_urls, auth.as_deref(), Some(path))
            .await
            .map(|(joined, _)| joined)
    };
    let file = match downloaded {
        Ok(d) => d,
        Err(e) => return Rerun::Unreachable(format!("{e:#}")),
    };
    if f.sha256.as_deref() != Some(file.sha256.as_str()) {
        return Rerun::Changed;
    }
    let verdict = state
        .config
        .releases
        .scanner
        .scan(&crate::scan::Target {
            path,
            file_name: &f.file_name,
            sha256: &file.sha256,
        })
        .await;
    let after = verdict.result.as_str().to_string();
    if let Err(e) =
        crate::scan::save_fingerprints(&state.db, release_id, &f.platform, &verdict.fingerprints)
            .await
    {
        return Rerun::Unreachable(format!("{e:#}"));
    }
    match sqlx::query(sqlx::AssertSqlSafe(format!(
        "update release_files set scan_result = $3, scan_detail = $4, scan_report = $5, checked_at = now()
         where release_id = $1 and platform = $2 and {STILL_HELD}"
    )))
    .bind(release_id)
    .bind(&f.platform)
    .bind(&after)
    .bind(&verdict.detail)
    .bind(verdict.report.map(sqlx::types::Json))
    .execute(&state.db)
    .await
    {
        Ok(done) if done.rows_affected() == 0 => Rerun::HoldEnded,
        Ok(_) => Rerun::Scanned {
            before: f.scan_result.clone().unwrap_or_default(),
            after,
        },
        Err(e) => Rerun::Unreachable(format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scanners_who_looks_next_ending_is_cut_old_or_new() {
        // Rows stored before checks-then-Publish, and the scanner's wording since.
        for detail in [
            "Flagged by ClamAV: Win.Test in game.exe. A person checks it before it can be approved; it may be a false positive.",
            "Flagged by ClamAV: Win.Test in game.exe. SLAPorSLOP staff look at it; it may be a false positive.",
        ] {
            assert_eq!(
                without_who_looks(detail),
                "Flagged by ClamAV: Win.Test in game.exe."
            );
        }
        for rest in [
            " ClamAV has no signatures yet. A person checks it before it can be approved.",
            " ClamAV has no signatures yet. SLAPorSLOP staff look at it.",
        ] {
            assert_eq!(
                without_who_looks(rest).trim().trim_end_matches('.'),
                "ClamAV has no signatures yet"
            );
        }
        assert_eq!(without_who_looks("Scanned clean."), "Scanned clean.");
    }

    #[test]
    fn a_missing_defender_check_is_staffs_only_where_slop_check_cant_run() {
        let missing = [HoldKind::DefenderMissing];
        assert!(!kinds_need_staff(&missing, true));
        assert!(kinds_need_staff(&missing, false));
        assert!(kinds_need_staff(&[HoldKind::Scan], true));
        assert!(slop_check_runs("https://github.com/octo/game"));
        assert!(!slop_check_runs("https://gitlab.com/octo/game"));
        assert!(!stored_kinds_need_staff(&["question".to_string()], false));
        assert!(stored_kinds_need_staff(
            &["defender_missing".to_string()],
            false
        ));
    }
}
