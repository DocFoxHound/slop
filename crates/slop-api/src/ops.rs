// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alerts for Slop's admins (SECURITY §13, ARCHITECTURE §7). Every five minutes the API checks
//! what needs a person and tells every admin under the 🔔, by phone push, and by email once SMTP
//! is set up. Each alert is sent once, then repeated at most daily while it lasts.
//!
//! What it watches:
//! - backups (`SLOP_OPS_BACKUP_DIR`): the nightly backup and the weekly restore check;
//! - deploys (`SLOP_OPS_DEPLOY_DIR`): a commit whose tests failed, and known-vulnerable
//!   dependencies in the running build (OSV, checked on every deploy);
//! - free disk space;
//! - the scanner failing on a release;
//! - urgent or serious reports as they arrive, and any report past its deadline;
//! - releases waiting more than a day for review;
//! - revocations, admin rights granted or removed, payment records not matching Stripe, and a
//!   re-scan flagging a live release (rescan.rs).
//!
//! If the whole server is down nothing here can say so: an outside uptime check on `/healthz`
//! covers that (deploy/README.md).

use std::path::Path;

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, admin_web::AdminUser, error::ApiResult};

const EVERY: std::time::Duration = std::time::Duration::from_secs(300);
const BACKUP_MAX_AGE: time::Duration = time::Duration::hours(26);
const RESTORE_CHECK_MAX_AGE: time::Duration = time::Duration::days(8);
const REVIEW_WAIT: time::Duration = time::Duration::hours(24);
/// Under this much free space (bytes) or this share, the disk is nearly full.
const MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const MIN_FREE_SHARE: f64 = 0.10;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/ops", get(current))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    /// Stable for the same problem, so it's sent once.
    pub key: String,
    pub title: String,
    pub detail: String,
    pub link: String,
    /// An event (sent once, then done) rather than a condition (repeated daily while true).
    pub once: bool,
}

fn alert(
    key: impl Into<String>,
    title: impl Into<String>,
    detail: impl Into<String>,
    link: &str,
    once: bool,
) -> Alert {
    Alert {
        key: key.into(),
        title: title.into(),
        detail: detail.into(),
        link: link.into(),
        once,
    }
}

/// A file's timestamp (written by the backup scripts), as a time.
pub fn stamp_in(path: &Path) -> Option<OffsetDateTime> {
    let text = std::fs::read_to_string(path).ok()?;
    OffsetDateTime::parse(text.trim(), &time::format_description::well_known::Rfc3339).ok()
}

/// Backups: the nightly one and the weekly restore check.
pub fn backup_alerts(dir: &Path, now: OffsetDateTime) -> Vec<Alert> {
    let mut out = Vec::new();
    match stamp_in(&dir.join("last-success")) {
        Some(at) if now - at <= BACKUP_MAX_AGE => {}
        Some(at) => out.push(alert(
            "backup-stale",
            "The nightly backup hasn't run",
            format!("The last good backup was at {at}. Check slop-backup.service (journalctl --user -u slop-backup)."),
            "/admin",
            false,
        )),
        None => out.push(alert(
            "backup-missing",
            "No database backup yet",
            format!("Nothing in {}. Check slop-backup.timer is enabled.", dir.display()),
            "/admin",
            false,
        )),
    }
    match stamp_in(&dir.join("last-restore-check")) {
        Some(at) if now - at <= RESTORE_CHECK_MAX_AGE => {}
        _ => out.push(alert(
            "restore-check-stale",
            "Backups haven't been test-restored this week",
            "Check slop-restore-check.service (journalctl --user -u slop-restore-check).",
            "/admin",
            false,
        )),
    }
    out
}

/// Deploys whose tests failed in the last two days (update.sh marks them `failed-<sha>`).
fn deploy_alerts(dir: &Path) -> Vec<Alert> {
    let Ok(entries) = std::fs::read_dir(dir.join("builds")) else {
        return Vec::new();
    };
    let recent = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86_400);
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let sha = name.strip_prefix("failed-")?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            (modified > recent).then(|| {
                alert(
                    format!("deploy-failed-{sha}"),
                    format!("Commit {} didn't deploy", &sha[..sha.len().min(7)]),
                    "Its tests failed, so the server kept running the previous build. See ~/slop-deploy/out/ (test.log, lint.log, e2e.log, container.log).",
                    "/admin",
                    true,
                )
            })
        })
        .collect()
}

/// Known-vulnerable dependencies in the running build (update.sh writes `audit.json`).
pub fn audit_alerts(dir: &Path) -> Vec<Alert> {
    let Ok(text) = std::fs::read_to_string(dir.join("audit.json")) else {
        return Vec::new();
    };
    let Ok(audit) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let vulns = audit["vulnerabilities"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if vulns.is_empty() {
        return Vec::new();
    }
    let mut names: Vec<String> = vulns
        .iter()
        .filter_map(|v| {
            Some(format!(
                "{} {} ({})",
                v["package"].as_str()?,
                v["version"].as_str()?,
                v["id"].as_str()?
            ))
        })
        .collect();
    names.sort();
    names.dedup();
    let shown = names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    vec![alert(
        "dependencies-vulnerable",
        format!(
            "{} known-vulnerable dependenc{}",
            names.len(),
            if names.len() == 1 { "y" } else { "ies" }
        ),
        format!(
            "{shown}{}. Update them (cargo update, npm update) and deploy. Full list: ~/slop-deploy/audit.json.",
            if names.len() > 5 { " and more" } else { "" }
        ),
        "/admin",
        false,
    )]
}

/// Free space where backups and downloads being scanned are written.
fn disk_alerts(paths: &[&Path]) -> Vec<Alert> {
    let mut out = Vec::new();
    for path in paths {
        let Ok(o) = std::process::Command::new("df")
            .args(["-Pk"])
            .arg(path)
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&o.stdout);
        let Some(line) = text.lines().nth(1) else {
            continue;
        };
        let cols: Vec<&str> = line.split_whitespace().collect();
        let (Some(total), Some(avail)) = (
            cols.get(1).and_then(|v| v.parse::<u64>().ok()),
            cols.get(3).and_then(|v| v.parse::<u64>().ok()),
        ) else {
            continue;
        };
        let (total, avail) = (total * 1024, avail * 1024);
        if avail < MIN_FREE_BYTES || (total > 0 && (avail as f64) / (total as f64) < MIN_FREE_SHARE)
        {
            let mount = cols.last().unwrap_or(&"?").to_string();
            out.push(alert(
                format!("disk-{mount}"),
                format!("The disk at {mount} is nearly full"),
                format!(
                    "{:.1} GB free of {:.1} GB.",
                    avail as f64 / 1e9,
                    total as f64 / 1e9
                ),
                "/admin",
                false,
            ));
        }
    }
    out.dedup_by(|a, b| a.key == b.key);
    out
}

async fn cursor(db: &sqlx::PgPool, name: &str, initial: &str) -> sqlx::Result<i64> {
    let existing: Option<i64> = sqlx::query_scalar("select value from ops_cursor where name = $1")
        .bind(name)
        .fetch_optional(db)
        .await?;
    match existing {
        Some(v) => Ok(v),
        None => {
            // First run: start from now, so history isn't alerted on.
            let start: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(initial.to_string()))
                .fetch_one(db)
                .await?;
            set_cursor(db, name, start).await?;
            Ok(start)
        }
    }
}

async fn set_cursor(db: &sqlx::PgPool, name: &str, value: i64) -> sqlx::Result<()> {
    sqlx::query(
        "insert into ops_cursor (name, value) values ($1, $2) on conflict (name) do update set value = $2",
    )
    .bind(name)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

/// Everything that needs an admin right now.
/// Settings that are wrong for a public deployment.
pub fn config_alerts(config: &crate::config::Config) -> Vec<Alert> {
    let mut out = Vec::new();
    // Behind a reverse proxy on the same machine, every request comes from the proxy: without
    // the header that carries the visitor's address, the rate limits (ratelimit.rs) count the
    // whole internet as one visitor, and one script can lock everyone out of signing in.
    if config.secure_cookies()
        && config.bind.ip().is_loopback()
        && config.client_ip_header.is_none()
    {
        out.push(alert(
            "client-ip-header-missing",
            "Rate limits can't tell visitors apart",
            "Slop listens on localhost behind a proxy but SLOP_CLIENT_IP_HEADER isn't set, so every \
             visitor shares one rate limit. Set SLOP_CLIENT_IP_HEADER=x-forwarded-for in slop.env \
             (Caddy overwrites that header) and restart slop-api.",
            "/admin",
            false,
        ));
    }
    out
}

pub async fn collect(state: &AppState) -> anyhow::Result<Vec<Alert>> {
    let db = &state.db;
    let now = OffsetDateTime::now_utc();
    let mut out = Vec::new();
    let ops = &state.config.ops;
    if let Some(dir) = &ops.backup_dir {
        out.extend(backup_alerts(dir, now));
    }
    if let Some(dir) = &ops.deploy_dir {
        out.extend(deploy_alerts(dir));
        out.extend(audit_alerts(dir));
    }
    let tmp = std::env::temp_dir();
    let mut disks: Vec<&Path> = vec![tmp.as_path()];
    if let Some(dir) = &ops.backup_dir {
        disks.push(dir.as_path());
    }
    out.extend(disk_alerts(&disks));
    out.extend(config_alerts(&state.config));

    // The scanner failing (not finding something: that's the review queue's job).
    let failed: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select r.id, a.slug, r.version from release_files f join releases r on r.id = f.release_id
         join apps a on a.id = r.app_id
         where f.scan_detail like 'the scanner couldn''t finish%' and r.created_at > now() - interval '2 days'",
    )
    .fetch_all(db)
    .await?;
    for (id, slug, version) in failed {
        out.push(alert(
            format!("scanner-failed-{id}"),
            format!("The scanner failed on {slug} {version}"),
            "The release is held for review. Check the scanner (journalctl --user -u slop-api, and ~/slop-deploy/bin/slop-scan).",
            "/admin",
            true,
        ));
    }

    // Reports: urgent and serious ones as they arrive, and any past its deadline.
    let after = cursor(
        db,
        "reports",
        "select coalesce(extract(epoch from max(created_at))::int8, 0) from reports",
    )
    .await?;
    let new: Vec<(String, i16, OffsetDateTime)> = sqlx::query_as(
        "select reason, priority, created_at from reports
         where status = 'open' and priority <= 1 and created_at > to_timestamp($1) order by created_at",
    )
    .bind(after as f64)
    .fetch_all(db)
    .await?;
    let mut newest = after;
    for (reason, priority, at) in new {
        newest = newest.max(at.unix_timestamp());
        out.push(alert(
            format!("report-{}-{reason}", at.unix_timestamp_nanos()),
            if priority == 0 {
                "Urgent report: act within the hour"
            } else {
                "Serious report: act within 24 hours"
            },
            format!("A new {reason} report is waiting."),
            "/admin",
            true,
        ));
    }
    set_cursor(db, "reports", newest).await?;
    let overdue: i64 =
        sqlx::query_scalar("select count(*) from reports where status = 'open' and due_at < now()")
            .fetch_one(db)
            .await?;
    if overdue > 0 {
        out.push(alert(
            "reports-overdue",
            format!(
                "{overdue} report{} past {} deadline",
                if overdue == 1 { "" } else { "s" },
                if overdue == 1 { "its" } else { "their" }
            ),
            "Reports have legal deadlines (24 hours for serious ones). Open the admin queue.",
            "/admin",
            false,
        ));
    }

    // Law-enforcement requests with a deadline within a day (legal_requests.rs).
    for (reference, kind, deadline) in crate::legal_requests::due_soon(db).await? {
        out.push(alert(
            format!("legal-request-{reference}"),
            if kind == "removal_order" {
                format!("Removal order {reference}: act within the hour")
            } else {
                format!("Legal request {reference} is due {}", deadline.date())
            },
            format!(
                "A {kind} request has its deadline at {deadline}. Open Admin → Legal requests."
            ),
            "/admin?tab=legal",
            false,
        ));
    }

    let signals: i64 =
        sqlx::query_scalar("select count(*) from abuse_signals where status = 'open'")
            .fetch_one(db)
            .await?;
    if signals > 0 {
        out.push(alert(
            "abuse-open",
            format!("{signals} possible abuse pattern{} to look at", if signals == 1 { "" } else { "s" }),
            "Vote bursts, review bombs, lookalike app names or accounts hitting rate limits. See the admin queue.",
            "/admin",
            false,
        ));
    }

    let waiting: i64 = sqlx::query_scalar(
        "select count(*) from releases where status = 'awaiting_review' and created_at < $1",
    )
    .bind(now - REVIEW_WAIT)
    .fetch_one(db)
    .await?;
    if waiting > 0 {
        out.push(alert(
            "reviews-waiting",
            format!(
                "{waiting} release{} waiting over a day for review",
                if waiting == 1 { "" } else { "s" }
            ),
            "Makers are waiting. Open the admin queue.",
            "/admin",
            false,
        ));
    }

    // Events from the audit log.
    let after = cursor(db, "audit", "select coalesce(max(id), 0) from audit_log").await?;
    let events: Vec<(i64, String, String, String)> = sqlx::query_as(
        "select id, actor, action, subject from audit_log
         where id > $1 and action in ('release.revoke', 'admin.grant', 'admin.revoke', 'payments.mismatch', 'release.rescan_flagged')
         order by id",
    )
    .bind(after)
    .fetch_all(db)
    .await?;
    let last: Option<i64> = sqlx::query_scalar("select max(id) from audit_log")
        .fetch_one(db)
        .await?;
    for (id, actor, action, subject) in events {
        let (title, detail) = match action.as_str() {
            "release.revoke" => (
                "A release was withdrawn",
                format!("{actor} withdrew {subject}. It stops launching for everyone."),
            ),
            "admin.grant" => (
                "Someone was made an admin",
                format!("{actor} gave admin rights to {subject}."),
            ),
            "admin.revoke" => (
                "Admin rights were removed",
                format!("{actor} removed admin rights from {subject}."),
            ),
            _ => (
                "Payments don't match Stripe",
                format!(
                    "Slop's records and Stripe's fees differ for {subject}. See the audit log."
                ),
            ),
        };
        out.push(alert(
            format!("audit-{id}"),
            title,
            detail,
            "/admin?tab=audit",
            true,
        ));
    }
    if let Some(last) = last {
        set_cursor(db, "audit", last.max(after)).await?;
    }
    Ok(out)
}

/// One run: send new alerts, repeat lasting ones daily, and close the ones that cleared.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let alerts = collect(state).await?;
    let admins: Vec<Uuid> =
        sqlx::query_scalar("select id from users where is_admin and closed_at is null")
            .fetch_all(&state.db)
            .await?;
    let mut sent = 0;
    let mut tx = state.db.begin().await?;
    for a in &alerts {
        let due: bool = sqlx::query_scalar(
            "insert into ops_alerts (key, title, detail, link) values ($1, $2, $3, $4)
             on conflict (key) do update set title = $2, detail = $3, link = $4,
                 resolved_at = null,
                 first_seen = case when ops_alerts.resolved_at is not null then now() else ops_alerts.first_seen end,
                 last_sent = case when ops_alerts.resolved_at is not null then null else ops_alerts.last_sent end
             returning last_sent is null or last_sent < now() - interval '1 day'",
        )
        .bind(&a.key)
        .bind(&a.title)
        .bind(&a.detail)
        .bind(&a.link)
        .fetch_one(&mut *tx)
        .await?;
        if !due {
            continue;
        }
        for admin in &admins {
            crate::notify::send(
                &mut tx,
                crate::notify::Notice {
                    user_id: *admin,
                    category: "ops",
                    title: a.title.clone(),
                    body: a.detail.clone(),
                    link: a.link.clone(),
                    collapse_key: Some(format!("ops:{}", a.key)),
                },
            )
            .await?;
        }
        sqlx::query(
            "update ops_alerts set last_sent = now(), resolved_at = case when $2 then now() else null end where key = $1",
        )
        .bind(&a.key)
        .bind(a.once)
        .execute(&mut *tx)
        .await?;
        sent += 1;
    }
    // Conditions that no longer hold are resolved.
    let keys: Vec<String> = alerts.iter().map(|a| a.key.clone()).collect();
    sqlx::query("update ops_alerts set resolved_at = now() where resolved_at is null and not (key = any($1))")
        .bind(&keys)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(sent)
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "ops checks");
        }
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Current {
    key: String,
    title: String,
    detail: String,
    link: String,
    #[serde(with = "time::serde::rfc3339")]
    first_seen: OffsetDateTime,
}

/// What's wrong right now, for the admin page.
async fn current(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<Current>>> {
    Ok(Json(
        sqlx::query_as(
            "select key, title, detail, link, first_seen from ops_alerts
             where resolved_at is null order by first_seen desc limit 50",
        )
        .fetch_all(&state.db)
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vulnerable_dependencies_alert() {
        let dir = std::env::temp_dir().join(format!("slop-audit-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(audit_alerts(&dir).is_empty(), "no audit yet");
        std::fs::write(dir.join("audit.json"), r#"{"vulnerabilities": []}"#).unwrap();
        assert!(audit_alerts(&dir).is_empty());
        std::fs::write(
            dir.join("audit.json"),
            r#"{"vulnerabilities": [{"package": "zip", "version": "4.6.1", "id": "RUSTSEC-1"}, {"package": "zip", "version": "4.6.1", "id": "RUSTSEC-1"}]}"#,
        )
        .unwrap();
        let a = audit_alerts(&dir);
        assert_eq!(a[0].title, "1 known-vulnerable dependency");
        assert!(a[0].detail.starts_with("zip 4.6.1 (RUSTSEC-1)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backups_alert_when_stale_or_missing() {
        let dir = std::env::temp_dir().join(format!("slop-ops-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = OffsetDateTime::now_utc();
        let keys = |v: Vec<Alert>| v.into_iter().map(|a| a.key).collect::<Vec<_>>();
        assert_eq!(
            keys(backup_alerts(&dir, now)),
            ["backup-missing", "restore-check-stale"]
        );
        let fmt = |t: OffsetDateTime| {
            t.format(&time::format_description::well_known::Rfc3339)
                .unwrap()
        };
        std::fs::write(
            dir.join("last-success"),
            fmt(now - time::Duration::hours(2)),
        )
        .unwrap();
        std::fs::write(
            dir.join("last-restore-check"),
            fmt(now - time::Duration::days(3)),
        )
        .unwrap();
        assert!(backup_alerts(&dir, now).is_empty());
        std::fs::write(
            dir.join("last-success"),
            fmt(now - time::Duration::hours(30)),
        )
        .unwrap();
        assert_eq!(keys(backup_alerts(&dir, now)), ["backup-stale"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
