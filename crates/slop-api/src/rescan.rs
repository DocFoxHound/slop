// SPDX-License-Identifier: AGPL-3.0-or-later
//! Re-scanning live releases (SECURITY §5.6, DESIGN §16).
//!
//! - **Newer signatures** (`check_fingerprints`): every six hours, the fingerprints the scanner
//!   recorded of every program in the release players get for each app, and in releases waiting
//!   to be published or on hold (scan.rs), are checked against ClamAV's latest signatures,
//!   without downloading anything. A match on a live release re-scans it in full (below); one on
//!   a release not published yet holds it for SLAPorSLOP staff, and its maker is told.
//! - **Re-scans** (`run_once`): straight away when someone reports the app for malware or for not
//!   matching its source, or newer signatures match it. Every live release used to be scanned
//!   again weekly; with thousands of apps that costs too much (decided 2026-11-17), so it's off
//!   unless `SLOP_RESCAN_EVERY_DAYS` sets a number of days.
//!
//! A re-scan downloads the file again from the maker's page (as when it was first fetched),
//! checks it's still the approved file, scans it and deletes it. A new finding doesn't take
//! anything down by itself: it puts the release in the review queue, and the admins get an alert
//! (ops.rs). A reviewer withdraws it (the kill switch) or keeps it, with a reason. A finding a
//! reviewer already cleared as a false positive isn't raised again.

use std::collections::BTreeSet;

use futures_util::TryStreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    AppState,
    scan::{Query, Target},
};

/// Files due a re-scan each round, so a round never runs long.
const PER_ROUND: i64 = 5;
const EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// Who acts on a newer signature's match, in the audit log and a hold's log.
const ACTOR: &str = "rescan";

/// The release players get for each app (releases::SERVING's order).
const SERVING: &str = "select distinct on (r.app_id) r.id, r.app_id, r.published_at
     from releases r join apps a on a.id = r.app_id
     where r.status = 'approved' and r.signed_manifest is not null
     order by r.app_id, coalesce(r.id = a.pinned_release_id, false) desc, r.published_at desc";

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
    /// Staff cleared the first scan's findings as false positives (`cleared_at`).
    cleared: bool,
    rescan_report: Option<Value>,
    /// Staff kept the release after an earlier re-scan flagged it (`rescan_cleared_at`).
    rescan_cleared: bool,
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
    // The release players get, per app, whose files were asked for (or, with
    // SLOP_RESCAN_EVERY_DAYS, are due).
    let due: Vec<Due> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "with serving as ({SERVING})
         select s.app_id, s.id as release_id, f.platform, f.url, f.file_name, f.sha256, f.scan_report,
                f.cleared_at is not null as cleared, f.rescan_report,
                f.rescan_cleared_at is not null as rescan_cleared
         from serving s join release_files f on f.release_id = s.id
         left join rescan_requests q on q.app_id = s.app_id and q.requested_at <= now()
         where q.app_id is not null
            or ($2 > 0 and coalesce(f.rescanned_at, s.published_at, now() - interval '30 days')
                           < now() - make_interval(days => $2))
         order by q.requested_at nulls last, coalesce(f.rescanned_at, s.published_at) nulls first
         limit $1"
    )))
    .bind(PER_ROUND)
    .bind(state.config.releases.rescan_every_days as i32)
    .fetch_all(&state.db)
    .await?;
    let mut done = 0;
    let (mut apps, mut retry) = (BTreeSet::new(), BTreeSet::new());
    for f in due {
        apps.insert(f.app_id);
        let (result, report, flag, retry_soon) = rescan_file(state, &f).await;
        if retry_soon {
            retry.insert(f.app_id);
        }
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
    // A re-scan that couldn't download the file is asked for again a day later (the repository
    // check, frozen.rs, deals with files that stay out of reach).
    for app in apps {
        let query = if retry.contains(&app) {
            "update rescan_requests set requested_at = now() + interval '1 day' where app_id = $1"
        } else {
            "delete from rescan_requests where app_id = $1"
        };
        sqlx::query(query).bind(app).execute(&state.db).await?;
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
    if let Err(err) =
        crate::scan::save_fingerprints(&state.db, f.release_id, &f.platform, &verdict.fingerprints)
            .await
    {
        tracing::error!(error = %err, release = %f.release_id, "keeping fingerprints");
    }
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
    if f.cleared {
        before.extend(finding_names(f.scan_report.as_ref()));
    }
    if f.rescan_cleared {
        before.extend(finding_names(f.rescan_report.as_ref()));
    }
    let cleared = f.cleared || f.rescan_cleared;
    let flag = is_new_finding(cleared, &before, &now);
    (
        verdict.result.as_str().to_string(),
        Some(report),
        flag,
        false,
    )
}

/// Checks the fingerprints of live releases' programs, and of releases waiting to be published
/// or on hold, against ClamAV's latest signatures, if they're due (every six hours, as the
/// signatures are refreshed). Each signature that matches a file acts once: a live release is
/// re-scanned in full, one not published yet is held for staff. Returns how many acted.
pub async fn check_fingerprints(state: &AppState) -> anyhow::Result<usize> {
    let scanner = state.config.releases.scanner;
    if !scanner.is_on() {
        return Ok(0);
    }
    let due: bool = sqlx::query_scalar(
        "select not exists (select 1 from fingerprint_checks where checked_at > now() - interval '6 hours')",
    )
    .fetch_one(&state.db)
    .await?;
    if !due {
        return Ok(0);
    }
    // Releases that can never reach players again aren't checked: their fingerprints go.
    sqlx::query(
        "delete from release_file_fingerprints p using releases r
         where r.id = p.release_id and r.status in ('rejected', 'withdrawn', 'failed', 'revoked')",
    )
    .execute(&state.db)
    .await?;
    let db = state.db.clone();
    let Some(matches) = scanner
        .match_fingerprints(|stdin| async move {
            let mut out = tokio::io::BufWriter::new(stdin);
            let mut rows = sqlx::query_as::<_, (Uuid, String, String, Option<i16>, i64, String, String, String)>(
                sqlx::AssertSqlSafe(format!(
                    "with serving as ({SERVING}),
                          checked as (select id from serving
                                      union select id from releases where status in ('ready', 'awaiting_review'))
                     select p.release_id, p.platform, p.file, p.section, p.size, p.md5, p.sha1, p.sha256
                     from release_file_fingerprints p where p.release_id in (select id from checked)"
                )),
            )
            .fetch(&db);
            while let Some((release, platform, file, section, size, md5, sha1, sha256)) = rows.try_next().await? {
                // The release, platform and file come back with a match: neither a UUID nor a
                // platform has a "/" in it.
                let id = format!("{release}/{platform}/{file}");
                let line = serde_json::to_string(&Query {
                    id: &id,
                    kind: if section.is_some() { "section" } else { "file" },
                    size,
                    md5: &md5,
                    sha1: &sha1,
                    sha256: &sha256,
                })?;
                out.write_all(line.as_bytes()).await?;
                out.write_all(b"\n").await?;
            }
            out.shutdown().await?;
            Ok(())
        })
        .await?
    else {
        return Ok(0);
    };
    let mut acted = 0;
    for (id, signature) in matches.found {
        let mut parts = id.splitn(3, '/');
        let (Some(release_id), Some(platform), Some(file)) = (
            parts.next().and_then(|s| s.parse::<Uuid>().ok()),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let signature: String = signature.chars().take(300).collect();
        let new = sqlx::query(
            "insert into fingerprint_matches (release_id, platform, signature, file)
             select $1, $2, $3, $4 where exists (select 1 from release_files where release_id = $1 and platform = $2)
             on conflict do nothing",
        )
        .bind(release_id)
        .bind(platform)
        .bind(&signature)
        .bind(file)
        .execute(&state.db)
        .await?
        .rows_affected()
            == 1;
        if new && act_on_match(state, release_id, platform, file, &signature).await? {
            acted += 1;
        }
    }
    sqlx::query(
        "insert into fingerprint_checks (signatures) values ($1)
         on conflict (id) do update set signatures = excluded.signatures, checked_at = now()",
    )
    .bind(&matches.signatures)
    .execute(&state.db)
    .await?;
    Ok(acted)
}

/// What a newer signature's match on one file does. Returns whether it did anything.
async fn act_on_match(
    state: &AppState,
    release_id: Uuid,
    platform: &str,
    file: &str,
    signature: &str,
) -> anyhow::Result<bool> {
    let row: Option<(String, Uuid, Option<Value>, bool)> = sqlx::query_as(
        "select r.status::text, r.app_id, f.scan_report, f.cleared_at is not null
         from releases r join release_files f on f.release_id = r.id
         where r.id = $1 and f.platform = $2",
    )
    .bind(release_id)
    .bind(platform)
    .fetch_optional(&state.db)
    .await?;
    let Some((status, app_id, scan_report, cleared)) = row else {
        return Ok(false);
    };
    // Already judged a false positive by that name (ClamAV adds ".UNOFFICIAL" to signatures
    // that aren't its own).
    let known = finding_names(scan_report.as_ref());
    if cleared && (known.contains(signature) || known.contains(&format!("{signature}.UNOFFICIAL")))
    {
        return Ok(false);
    }
    let mut tx = state.db.begin().await?;
    crate::admin::audit(
        &mut tx,
        ACTOR,
        "release.fingerprint_matched",
        &format!("{release_id} {platform}"),
        json!({ "signature": signature, "file": file, "status": status }),
    )
    .await?;
    tx.commit().await?;
    tracing::warn!(release = %release_id, %platform, %signature, %file, "newer signatures match a release");
    match status.as_str() {
        // Live: a full re-scan, which flags it for staff if it finds something new.
        "approved" => {
            request(
                &state.db,
                app_id,
                &format!("newer signatures match {signature} in {file}"),
            )
            .await?;
        }
        // Not published yet: held for staff, like a finding at its first scan.
        "ready" | "awaiting_review" => {
            let detail = format!(
                "Flagged by ClamAV: {signature} in {file}, by signatures newer than its scan. \
                 SLAPorSLOP staff look at it; it may be a false positive."
            );
            sqlx::query(
                "update release_files set scan_result = 'on_hold', scan_detail = $3,
                        scan_report = jsonb_set(coalesce(scan_report, '{}'::jsonb), '{findings}',
                                                coalesce(scan_report -> 'findings', '[]'::jsonb) || $4),
                        cleared_at = null, cleared_by = null, cleared_reason = null
                 where release_id = $1 and platform = $2",
            )
            .bind(release_id)
            .bind(platform)
            .bind(&detail)
            .bind(json!([{ "engine": "ClamAV", "file": file, "name": signature }]))
            .execute(&state.db)
            .await?;
            let rules = crate::admin::ReviewRules::from_config(&state.config);
            if status == "ready" {
                crate::publish::recheck_ready(&state.db, rules, release_id, ACTOR).await?;
            } else {
                crate::publish::finish_checks(&state.db, rules, release_id, ACTOR).await?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = check_fingerprints(&state).await {
            tracing::error!(error = %err, "checking fingerprints");
        }
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
