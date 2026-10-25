// SPDX-License-Identifier: AGPL-3.0-or-later
//! Where a release is on its way through Slop's checks, file by file, for its maker and their AI
//! agent (`GET /v1/apps/{slug}/releases`, the agent tool `list_releases`).
//!
//! Slop checks one release at a time, oldest first, and its files one at a time (releases.rs,
//! `fetch_release`): download, maker signature, then the scanner (ClamAV, YARA and capa in a
//! container), which takes minutes per file. Each file records its stage and when its check
//! started and finished (migrations/0083_release_file_progress.sql), so the maker sees "checking
//! file 2 of 4: Linux (.deb), scanning for malware" with an estimate from how long recent files
//! took, instead of one step that looks stuck.

use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

/// What one file is doing now, as stored in `release_files.check_stage`.
pub const STAGES: [&str; 3] = ["downloading", "signature", "scanning"];

/// A scanner's longest run on one file (scan.rs).
const LONGEST_SCAN_MINUTES: i64 = 30;
/// Without any history on a scanning server: what a file usually takes there.
const DEFAULT_SECONDS_PER_FILE: i64 = 5 * 60;

/// Plain words for a file's check stage.
pub fn stage_label(stage: &str) -> &'static str {
    match stage {
        "downloading" => "Downloading",
        "signature" => "Checking the maker signature",
        "scanning" => "Scanning for malware",
        _ => "Checking",
    }
}

/// "Windows (.zip)", "Linux (.deb)": the platform, and the kind of file from its name.
pub fn file_label(platform: &str, file_name: &str) -> String {
    let name = file_name.to_ascii_lowercase();
    let ext = [".tar.gz", ".tgz", ".zip", ".deb", ".rpm", ".apk"]
        .into_iter()
        .find(|e| name.ends_with(e));
    let system = match platform {
        "windows" => "Windows",
        "linux" | "linux-deb" | "linux-rpm" => "Linux",
        "mac" => "macOS",
        "android" => "Android",
        "any" => "Mod",
        other => other,
    };
    match ext {
        Some(e) => format!("{system} ({e})"),
        None => system.to_string(),
    }
}

/// One file, as the progress needs it.
pub struct FileState<'a> {
    pub platform: &'a str,
    pub file_name: &'a str,
    pub stage: Option<&'a str>,
    pub started_at: Option<OffsetDateTime>,
    pub checked_at: Option<OffsetDateTime>,
    /// Its size and hash are recorded (releases checked before per-file progress was kept).
    pub hashed: bool,
    pub scan_result: Option<&'a str>,
    pub scan_detail: Option<&'a str>,
}

impl FileState<'_> {
    /// waiting, checking, checked, or not_checked (a release that stopped before reaching it).
    pub fn check(&self, release_status: &str) -> &'static str {
        if self.checked_at.is_some() {
            "checked"
        } else if release_status == "fetching" {
            if self.stage.is_some() {
                "checking"
            } else {
                "waiting"
            }
        } else if self.hashed {
            "checked"
        } else {
            "not_checked"
        }
    }
}

/// The file being checked right now.
#[derive(Debug, Serialize)]
pub struct CurrentFile {
    /// Its place in the order Slop checks them, from 1.
    pub number: usize,
    pub platform: String,
    pub label: String,
    /// downloading, signature or scanning.
    pub step: String,
    pub step_label: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
pub struct ReleaseProgress {
    /// queued, checking_files, waiting_for_review, approved, rejected, failed, revoked or
    /// withdrawn.
    pub stage: &'static str,
    pub files_total: usize,
    pub files_checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentFile>,
    /// When Slop started checking the first file.
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    /// About how long one file takes on this server, from the last files it checked; null when
    /// there's nothing to go on (a server without a scanner checks files in seconds).
    pub seconds_per_file: Option<i64>,
    /// About how long until every file is checked, while they are.
    pub remaining_seconds: Option<i64>,
    /// Releases (of any app) Slop checks before this one starts.
    pub queue_ahead: i64,
    /// One or two plain sentences for the maker, which an agent can pass on as they are.
    pub summary: String,
}

pub struct Inputs<'a> {
    pub status: &'a str,
    pub status_reason: Option<&'a str>,
    pub auto_approved: bool,
    pub files: &'a [FileState<'a>],
    pub now: OffsetDateTime,
    pub seconds_per_file: Option<i64>,
    pub queue_ahead: i64,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "under a minute", "about 1 minute", "about 25 minutes", "about 1 hour 10 minutes".
pub fn about(seconds: i64) -> String {
    if seconds < 60 {
        return "under a minute".into();
    }
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return format!("about {}", plural(minutes as usize, "minute", "minutes"));
    }
    let (h, m) = (minutes / 60, minutes % 60);
    let hours = plural(h as usize, "hour", "hours");
    if m == 0 {
        format!("about {hours}")
    } else {
        format!("about {hours} {}", plural(m as usize, "minute", "minutes"))
    }
}

/// The scan results that keep a release waiting for a person.
fn held(result: Option<&str>) -> bool {
    matches!(result, Some("on_hold" | "rejected" | "blocked"))
}

pub fn build(i: &Inputs) -> ReleaseProgress {
    let total = i.files.len();
    let files_checked = i
        .files
        .iter()
        .filter(|f| f.check(i.status) == "checked")
        .count();
    let started_at = i.files.iter().filter_map(|f| f.started_at).min();
    let current = i
        .files
        .iter()
        .enumerate()
        .find(|(_, f)| f.check(i.status) == "checking")
        .map(|(n, f)| CurrentFile {
            number: n + 1,
            platform: f.platform.to_string(),
            label: file_label(f.platform, f.file_name),
            step: f.stage.unwrap_or("downloading").to_string(),
            step_label: stage_label(f.stage.unwrap_or("downloading")),
            started_at: f.started_at.unwrap_or(i.now),
        });
    let files_word = plural(total, "file", "files");
    let mut remaining_seconds = None;
    let (stage, summary) = match i.status {
        "fetching" if started_at.is_none() && files_checked == 0 => (
            "queued",
            if i.queue_ahead > 0 {
                format!(
                    "Waiting its turn: Slop checks one release at a time, and {} ahead of this one. Then its {files_word} are downloaded and scanned, one at a time.",
                    plural(i.queue_ahead as usize, "release is", "releases are")
                )
            } else {
                format!(
                    "Starting: Slop is about to download and scan its {files_word}, one at a time."
                )
            },
        ),
        "fetching" => {
            let mut s = match &current {
                Some(c) => format!(
                    "Checking file {} of {total}: {}, {}. {files_checked} of {total} done.",
                    c.number,
                    c.label,
                    c.step_label.to_lowercase()
                ),
                None if files_checked == total => {
                    format!("All {files_word} are checked; finishing up.")
                }
                None => format!("{files_checked} of {total} files checked; starting the next."),
            };
            if let Some(per) = i.seconds_per_file.filter(|_| files_checked < total) {
                let elapsed = current
                    .as_ref()
                    .map_or(0, |c| (i.now - c.started_at).whole_seconds().max(0));
                let after_this = (total - files_checked).saturating_sub(1) as i64 * per;
                let this_left = per - elapsed;
                s.push_str(&format!(" Each file takes {} here", about(per)));
                if this_left > 0 || current.is_none() {
                    let left = after_this + this_left.max(0);
                    remaining_seconds = Some(left);
                    s.push_str(&format!("; {} left.", about(left)));
                } else {
                    remaining_seconds = Some(after_this);
                    s.push_str(&format!(
                        ". This one is taking longer than usual: big programs take longest, up to {LONGEST_SCAN_MINUTES} minutes a file."
                    ));
                }
            }
            ("checking_files", s)
        }
        "awaiting_review" => {
            let on_hold: Vec<&FileState> = i.files.iter().filter(|f| held(f.scan_result)).collect();
            let s = if let Some(first) = on_hold.first() {
                format!(
                    "{} on hold: {}. A person at Slop looks at it, and can clear a false positive; you'll get a notification when the release is decided.",
                    if on_hold.len() == 1 {
                        format!("{} is", file_label(first.platform, first.file_name))
                    } else {
                        format!("{} files are", on_hold.len())
                    },
                    first
                        .scan_detail
                        .unwrap_or("the scanner found something to look at")
                        .trim_end_matches('.')
                )
            } else {
                format!(
                    "All {files_word} are checked. Waiting for review by a person at Slop, usually within a day or two; you'll get a notification when it's decided."
                )
            };
            ("waiting_for_review", s)
        }
        "approved" => (
            "approved",
            if i.auto_approved {
                "Approved automatically as an update, and signed by Slop.".to_string()
            } else {
                "Approved by a person at Slop, and signed.".to_string()
            },
        ),
        "rejected" => (
            "rejected",
            format!(
                "Not approved: {}. get_fixes (or \"To fix\" on the Releases page) says what to change; then submit a new version.",
                i.status_reason
                    .unwrap_or("no reason given")
                    .trim_end_matches('.')
            ),
        ),
        "failed" => (
            "failed",
            format!(
                "Slop couldn't check the files: {}. Fix it and submit the release again.",
                i.status_reason
                    .unwrap_or("no reason given")
                    .trim_end_matches('.')
            ),
        ),
        "revoked" => (
            "revoked",
            format!(
                "Withdrawn by Slop: {}. It no longer launches.",
                i.status_reason
                    .unwrap_or("no reason given")
                    .trim_end_matches('.')
            ),
        ),
        _ => (
            "withdrawn",
            "Withdrawn before review; it never reached players.".to_string(),
        ),
    };
    ReleaseProgress {
        stage,
        files_total: total,
        files_checked,
        current,
        started_at,
        seconds_per_file: i.seconds_per_file.filter(|_| i.status == "fetching"),
        remaining_seconds,
        queue_ahead: if stage == "queued" { i.queue_ahead } else { 0 },
        summary,
    }
}

/// How long one file's check takes on this server, from the last 20 it finished (any app's: only
/// the durations are read). Falls back to a typical scan when there's no history yet, and to
/// nothing on a server without a scanner.
pub async fn seconds_per_file(db: &sqlx::PgPool, scanning: bool) -> sqlx::Result<Option<i64>> {
    let avg: Option<f64> = sqlx::query_scalar(
        "select extract(epoch from avg(checked_at - check_started_at))::float8 from (
             select checked_at, check_started_at from release_files
             where checked_at is not null and check_started_at is not null
               and checked_at > now() - interval '30 days'
             order by checked_at desc limit 20) recent",
    )
    .fetch_one(db)
    .await?;
    Ok(match avg {
        Some(s) if scanning || s >= 60.0 => Some(s.round().max(1.0) as i64),
        _ if scanning => Some(DEFAULT_SECONDS_PER_FILE),
        _ => None,
    })
}

/// Releases waiting to be checked that Slop takes before this one (the fetcher goes oldest first).
pub async fn queue_ahead(db: &sqlx::PgPool, release: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "select count(*) from releases o, releases r
         where r.id = $1 and o.status = 'fetching' and o.id <> r.id and o.created_at < r.created_at",
    )
    .bind(release)
    .fetch_one(db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);

    fn file<'a>(platform: &'a str, name: &'a str) -> FileState<'a> {
        FileState {
            platform,
            file_name: name,
            stage: None,
            started_at: None,
            checked_at: None,
            hashed: false,
            scan_result: None,
            scan_detail: None,
        }
    }

    fn done<'a>(mut f: FileState<'a>, started_min_ago: i64, took_min: i64) -> FileState<'a> {
        let start = NOW - time::Duration::minutes(started_min_ago);
        f.started_at = Some(start);
        f.checked_at = Some(start + time::Duration::minutes(took_min));
        f.hashed = true;
        f.scan_result = Some("passed");
        f
    }

    fn inputs<'a>(status: &'a str, files: &'a [FileState<'a>]) -> Inputs<'a> {
        Inputs {
            status,
            status_reason: None,
            auto_approved: false,
            files,
            now: NOW,
            seconds_per_file: Some(300),
            queue_ahead: 0,
        }
    }

    #[test]
    fn labels() {
        assert_eq!(file_label("windows", "Game-1.0.ZIP"), "Windows (.zip)");
        assert_eq!(file_label("linux", "game.tar.gz"), "Linux (.tar.gz)");
        assert_eq!(
            file_label("linux-deb", "game_1.0_amd64.deb"),
            "Linux (.deb)"
        );
        assert_eq!(file_label("linux-rpm", "game.rpm"), "Linux (.rpm)");
        assert_eq!(file_label("any", "mod.zip"), "Mod (.zip)");
        assert_eq!(stage_label("scanning"), "Scanning for malware");
        assert_eq!(about(20), "under a minute");
        assert_eq!(about(300), "about 5 minutes");
        assert_eq!(about(89), "about 1 minute");
        assert_eq!(about(3600), "about 1 hour");
        assert_eq!(about(4500), "about 1 hour 15 minutes");
    }

    #[test]
    fn a_release_waiting_its_turn() {
        let files = [file("linux", "g.tar.gz"), file("windows", "g.zip")];
        let mut i = inputs("fetching", &files);
        i.queue_ahead = 2;
        let p = build(&i);
        assert_eq!(p.stage, "queued");
        assert_eq!((p.files_total, p.files_checked, p.queue_ahead), (2, 0, 2));
        assert!(p.current.is_none());
        assert!(
            p.summary.contains("2 releases are ahead of this one"),
            "{}",
            p.summary
        );
        assert!(p.summary.contains("its 2 files"));
        assert_eq!(files[0].check("fetching"), "waiting");
    }

    #[test]
    fn file_two_of_four_is_being_scanned() {
        let mut deb = file("linux-deb", "g.deb");
        deb.stage = Some("scanning");
        deb.started_at = Some(NOW - time::Duration::minutes(2));
        let files = [
            done(file("linux", "g.tar.gz"), 12, 6),
            deb,
            file("linux-rpm", "g.rpm"),
            file("windows", "g.zip"),
        ];
        let p = build(&inputs("fetching", &files));
        assert_eq!(p.stage, "checking_files");
        assert_eq!((p.files_total, p.files_checked), (4, 1));
        let c = p.current.as_ref().unwrap();
        assert_eq!(
            (c.number, c.label.as_str(), c.step.as_str()),
            (2, "Linux (.deb)", "scanning")
        );
        assert_eq!(p.started_at, Some(NOW - time::Duration::minutes(12)));
        // 3 minutes left of this one, then two more files at 5 minutes each.
        assert_eq!(p.remaining_seconds, Some(13 * 60));
        assert_eq!(
            p.summary,
            "Checking file 2 of 4: Linux (.deb), scanning for malware. 1 of 4 done. Each file takes about 5 minutes here; about 13 minutes left."
        );
        assert_eq!(
            files
                .iter()
                .map(|f| f.check("fetching"))
                .collect::<Vec<_>>(),
            ["checked", "checking", "waiting", "waiting"]
        );
    }

    #[test]
    fn a_slow_file_says_so_instead_of_counting_below_zero() {
        let mut win = file("windows", "g.zip");
        win.stage = Some("scanning");
        win.started_at = Some(NOW - time::Duration::minutes(9));
        let files = [win, file("linux", "g.tar.gz")];
        let p = build(&inputs("fetching", &files));
        assert_eq!(p.remaining_seconds, Some(300));
        assert!(
            p.summary.contains("taking longer than usual"),
            "{}",
            p.summary
        );
    }

    #[test]
    fn checked_files_wait_for_a_person() {
        let files = [
            done(file("linux", "g.tar.gz"), 30, 5),
            done(file("windows", "g.zip"), 25, 5),
        ];
        let p = build(&inputs("awaiting_review", &files));
        assert_eq!(p.stage, "waiting_for_review");
        assert_eq!(p.files_checked, 2);
        assert_eq!((p.seconds_per_file, p.remaining_seconds), (None, None));
        assert!(
            p.summary
                .contains("Waiting for review by a person at Slop, usually within a day or two")
        );
        assert!(p.summary.contains("notification"));

        let mut held = done(file("windows", "g.zip"), 25, 5);
        held.scan_result = Some("on_hold");
        held.scan_detail = Some("YARA: Packed_UPX.");
        let files = [done(file("linux", "g.tar.gz"), 30, 5), held];
        let p = build(&inputs("awaiting_review", &files));
        assert_eq!(
            p.summary,
            "Windows (.zip) is on hold: YARA: Packed_UPX. A person at Slop looks at it, and can clear a false positive; you'll get a notification when the release is decided."
        );
    }

    #[test]
    fn older_releases_and_the_end_states() {
        // Checked before per-file progress was kept: hashed, no times.
        let mut old = file("windows", "g.zip");
        old.hashed = true;
        assert_eq!(old.check("approved"), "checked");
        assert_eq!(file("linux", "g.tar.gz").check("failed"), "not_checked");
        let files = [old];
        let mut i = inputs("rejected", &files);
        i.status_reason = Some("The program doesn't start.");
        let p = build(&i);
        assert_eq!(p.stage, "rejected");
        assert!(
            p.summary
                .starts_with("Not approved: The program doesn't start. get_fixes")
        );
        i.status = "approved";
        i.auto_approved = true;
        assert_eq!(
            build(&i).summary,
            "Approved automatically as an update, and signed by Slop."
        );
        i.status = "withdrawn";
        assert_eq!(build(&i).stage, "withdrawn");
    }
}
