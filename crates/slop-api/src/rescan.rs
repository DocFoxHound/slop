// SPDX-License-Identifier: AGPL-3.0-or-later
//! Re-scanning live releases (SECURITY §5.6, DESIGN §16). The release players get for each app is
//! scanned again at least weekly, with the scanner's latest signatures, and straight away when
//! someone reports the app for malware or for not matching its source.
//!
//! SLAPorSLOP downloads the file again from the maker's page (as when it was first fetched), checks it's
//! still the approved file, scans it and deletes it. A new finding doesn't take anything down by
//! itself: it puts the release in the review queue, and the admins get an alert (ops.rs). A
//! reviewer withdraws it (the kill switch) or keeps it, with a reason. A finding a reviewer
//! already cleared as a false positive isn't raised again.

use std::collections::BTreeSet;

use serde_json::Value;
use uuid::Uuid;

use crate::{AppState, scan::Target};

/// Files due a re-scan each round, so a round never runs long.
const PER_ROUND: i64 = 5;
const EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Asks for an app's live release to be re-scanned soon (e.g. on a malware report).
pub async fn request<'c>(
    db: impl sqlx::PgExecutor<'c>,
    app_id: Uuid,
    reason: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into rescan_requests (app_id, reason) values ($1, $2) on conflict (app_id) do nothing",
    )
    .bind(app_id)
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Due {
    app_id: Uuid,
    release_id: Uuid,
    platform: String,
    url: String,
    file_name: String,
    sha256: Option<String>,
    scan_report: Option<Value>,
    cleared_reason: Option<String>,
    rescan_report: Option<Value>,
    rescan_cleared_reason: Option<String>,
}

/// The findings' names in a scan report.
fn finding_names(report: Option<&Value>) -> BTreeSet<String> {
    report
        .and_then(|r| r["findings"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|f| f["name"].as_str().map(String::from))
        .collect()
}

/// Whether a re-scan's findings are new: anything when nothing was cleared, otherwise only names
/// the cleared scan didn't have.
pub fn is_new_finding(cleared: bool, before: &BTreeSet<String>, now: &BTreeSet<String>) -> bool {
    !now.is_empty() && (!cleared || !now.is_subset(before))
}

/// One round. Returns how many files were re-scanned.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let scanner = state.config.releases.scanner;
    if !scanner.is_on() {
        return Ok(0);
    }
    // The release players get, per app (releases::SERVING's order), whose files are due.
    let due: Vec<Due> = sqlx::query_as(
        "with serving as (
             select distinct on (r.app_id) r.id, r.app_id, r.reviewed_at
             from releases r join apps a on a.id = r.app_id
             where r.status = 'approved' and r.signed_manifest is not null
             order by r.app_id, coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc
         )
         select s.app_id, s.id as release_id, f.platform, f.url, f.file_name, f.sha256, f.scan_report, f.cleared_reason,
                f.rescan_report, f.rescan_cleared_reason
         from serving s join release_files f on f.release_id = s.id
         left join rescan_requests q on q.app_id = s.app_id
         where q.app_id is not null
            or coalesce(f.rescanned_at, s.reviewed_at, now() - interval '30 days') < now() - interval '7 days'
         order by q.requested_at nulls last, coalesce(f.rescanned_at, s.reviewed_at) nulls first
         limit $1",
    )
    .bind(PER_ROUND)
    .fetch_all(&state.db)
    .await?;
    let mut done = 0;
    let mut apps = BTreeSet::new();
    for f in due {
        apps.insert(f.app_id);
        let (result, report, flag, retry_soon) = rescan_file(state, &f).await;
        sqlx::query(
            "update release_files set
                 rescanned_at = case when $5 then now() - interval '6 days' else now() end,
                 rescan_result = $3, rescan_report = $4,
                 rescan_flagged_at = case when $6 then coalesce(rescan_flagged_at, now()) else rescan_flagged_at end
             where release_id = $1 and platform = $2",
        )
        .bind(f.release_id)
        .bind(&f.platform)
        .bind(&result)
        .bind(&report)
        .bind(retry_soon)
        .bind(flag)
        .execute(&state.db)
        .await?;
        if flag {
            let mut tx = state.db.begin().await?;
            crate::admin::audit(
                &mut tx,
                "rescan",
                "release.rescan_flagged",
                &format!("{} {}", f.release_id, f.platform),
                serde_json::json!({ "result": result, "report": report }),
            )
            .await?;
            tx.commit().await?;
            tracing::warn!(release = %f.release_id, platform = %f.platform, %result, "re-scan flagged a live release");
        }
        done += 1;
    }
    for app in apps {
        sqlx::query("delete from rescan_requests where app_id = $1")
            .bind(app)
            .execute(&state.db)
            .await?;
    }
    Ok(done)
}

/// Downloads, checks and scans one file. Returns (result, report, flag for review, retry soon).
async fn rescan_file(state: &AppState, f: &Due) -> (String, Option<Value>, bool, bool) {
    let path = std::env::temp_dir().join(format!("slop-rescan-{}", Uuid::now_v7()));
    let auth = crate::repo_tokens::auth_for(state, f.app_id, &f.url)
        .await
        .ok()
        .flatten();
    let source = if auth.is_some() {
        Ok(f.url.clone())
    } else {
        crate::releases::download_link(state, &f.url).await
    };
    let downloaded = match source {
        Ok(source) => {
            crate::releases::download_with(
                &state.fetch_http,
                &source,
                auth.as_deref(),
                state.config.releases.max_file_bytes,
                Some(&path),
            )
            .await
        }
        Err(e) => Err(e),
    };
    let (_, sha) = match downloaded {
        Ok(d) => d,
        Err(err) => {
            let _ = tokio::fs::remove_file(&path).await;
            // The repository check (frozen.rs) deals with files that can't be reached.
            return (
                "unreachable".into(),
                Some(serde_json::json!({ "detail": format!("{err:#}") })),
                false,
                true,
            );
        }
    };
    if f.sha256.as_deref() != Some(sha.as_str()) {
        let _ = tokio::fs::remove_file(&path).await;
        // Players' apps refuse it (the hash is in the signed manifest), but someone should look.
        return (
            "changed".into(),
            Some(serde_json::json!({
                "detail": "The file on the maker's page isn't the approved one any more: players' apps refuse it.",
                "sha256": sha,
            })),
            true,
            false,
        );
    }
    let verdict = state
        .config
        .releases
        .scanner
        .scan(&Target {
            path: &path,
            file_name: &f.file_name,
            sha256: &sha,
        })
        .await;
    let _ = tokio::fs::remove_file(&path).await;
    let detail = verdict.detail.clone().unwrap_or_default();
    let report = verdict
        .report
        .clone()
        .unwrap_or_else(|| serde_json::json!({ "detail": detail }));
    if detail.starts_with("the scanner couldn't finish") {
        return ("unreachable".into(), Some(report), false, true);
    }
    let now = finding_names(verdict.report.as_ref());
    // What reviewers already judged false positives: the first scan's findings when it was
    // cleared, and an earlier re-scan's when that was kept.
    let mut before = BTreeSet::new();
    if f.cleared_reason.is_some() {
        before.extend(finding_names(f.scan_report.as_ref()));
    }
    if f.rescan_cleared_reason.is_some() {
        before.extend(finding_names(f.rescan_report.as_ref()));
    }
    let cleared = f.cleared_reason.is_some() || f.rescan_cleared_reason.is_some();
    let flag = is_new_finding(cleared, &before, &now);
    (
        verdict.result.as_str().to_string(),
        Some(report),
        flag,
        false,
    )
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "re-scans");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cleared_findings_are_not_raised_again() {
        assert!(!is_new_finding(false, &set(&[]), &set(&[])));
        assert!(is_new_finding(false, &set(&[]), &set(&["Eicar"])));
        assert!(!is_new_finding(
            true,
            &set(&["Packer.UPX"]),
            &set(&["Packer.UPX"])
        ));
        assert!(is_new_finding(
            true,
            &set(&["Packer.UPX"]),
            &set(&["Packer.UPX", "Trojan.X"])
        ));
    }
}
