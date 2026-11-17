// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scans run on makers' GitHub runners (decided 2026-11-17): Slop Build and Slop Check scan the
//! files with SLAPorSLOP's scanner image and send the reports (slop_build.rs); SLAPorSLOP uses them
//! instead of scanning the files itself, waits for them, and scans here only when they don't come
//! (releases.rs).

use crate::*;

/// A stand-in for slop-scan that counts its runs and passes every file.
fn counting_scanner(dir: &std::path::Path) -> (String, std::path::PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    let runs = dir.join("runs");
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho run >> {runs}\necho '{{\"result\":\"passed\",\"detail\":\"nothing found\",\"findings\":[],\"capabilities\":[],\"engines\":{{\"clamav\":\"here\"}}}}'\n",
            runs = runs.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (script.to_str().unwrap().to_string(), runs)
}

fn runs(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .count()
}

/// What SLAPorSLOP's scanner image reports on a maker's runner.
fn github_report(found: &[&str]) -> Value {
    let today = OffsetDateTime::now_utc();
    json!({
        "result": if found.is_empty() { "passed" } else { "on_hold" },
        "detail": if found.is_empty() { "Scanned with ClamAV, YARA and capa: nothing found." } else { "Flagged by ClamAV." },
        "findings": found.iter().map(|n| json!({ "engine": "ClamAV", "file": "bin/game", "name": n })).collect::<Vec<_>>(),
        "noticed": [], "capabilities": [],
        "engines": { "clamav": format!("ClamAV 1.4.3/28186/Mon {} {} 06:30:00 {}", &today.month().to_string()[..3], today.day(), today.year()) },
        "hashes": [{ "file": "bin/game", "size": 3, "md5": "a".repeat(32), "sha1": "a".repeat(40), "sha256": "b".repeat(64) }],
    })
}

async fn send_release(app: &TestApp, maker: &str, version: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(maker)
                .json(json!({
                    "version": version,
                    "commit_sha": "a".repeat(40),
                    "files": [{
                        "platform": "linux", "kind": "archive", "exec": "bin/game",
                        "url": format!("{}octo/game/releases/download/v1/game.tar.gz", app.fake_base),
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    body_json(res).await["id"].as_str().unwrap().to_string()
}

async fn file_row(app: &TestApp, id: &str) -> (Option<String>, Option<String>, Option<Value>) {
    sqlx::query_as(
        "select check_stage, scan_result, scan_report from release_files where release_id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A scan from the maker's GitHub runner is used instead of scanning the file here. A release
/// fetched before its scan came waits for it, and goes on when it does; one whose scan never
/// comes is scanned here after the wait, so nobody is left waiting.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn scans_from_github_are_used_waited_for_and_backed_up(db: PgPool) {
    use sha2::Digest;
    let dir = std::env::temp_dir().join(format!("slop-ci-scans-{}", Uuid::now_v7()));
    let (command, counted) = counting_scanner(&dir);
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    sqlx::query("update apps set platforms = '{linux}'")
        .execute(&app.db)
        .await
        .unwrap();
    let app_id = app_id(&app, "game").await;
    let sha = hex::encode(sha2::Sha256::digest(BUILD_BYTES));
    let keep = |sha: String, report: Value| {
        let db = app.db.clone();
        async move {
            sqlx::query("insert into ci_scans (app_id, sha256, report) values ($1, $2, $3)")
                .bind(app_id)
                .bind(sha)
                .bind(report)
                .execute(&db)
                .await
                .unwrap();
        }
    };

    // Its scan came with the release: nothing is scanned here.
    keep(sha.clone(), github_report(&[])).await;
    let first = send_release(&app, &maker, "1.0.0").await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (stage, result, report) = file_row(&app, &first).await;
    assert_eq!((stage, result.as_deref()), (None, Some("passed")));
    let report = report.unwrap();
    assert!(
        report.get("hashes").is_none(),
        "fingerprints are kept apart"
    );
    assert_eq!(runs(&counted), 0);
    let prints: i64 = sqlx::query_scalar(
        "select count(*) from release_file_fingerprints where release_id = $1::uuid",
    )
    .bind(&first)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(prints, 1);
    assert_eq!(release_status(&app, &first).await.0, "ready");

    // Only a GitHub repository's releases wait (Slop Check runs on GitHub Actions); this one's
    // repository is on the test's own host, so it's scanned here at once.
    sqlx::query("delete from ci_scans where sha256 = $1")
        .bind(&sha)
        .execute(&app.db)
        .await
        .unwrap();
    keep("c".repeat(64), github_report(&[])).await;
    let here = send_release(&app, &maker, "1.0.1").await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(file_row(&app, &here).await.1.as_deref(), Some("passed"));
    assert_eq!(runs(&counted), 1);

    // A GitHub repository that has sent scans before: its release waits for the scan, the
    // maker sees why, and the fetcher has nothing to do meanwhile.
    // (Sent from the test's host, whose files the test serves, then checked as GitHub's.)
    let on_github = |github: bool| {
        let (db, base) = (app.db.clone(), app.fake_base.clone());
        async move {
            sqlx::query("update apps set repo_url = $1")
                .bind(if github {
                    "https://github.com/octo/game".to_string()
                } else {
                    format!("{base}octo/game")
                })
                .execute(&db)
                .await
                .unwrap();
        }
    };
    let waiting = send_release(&app, &maker, "1.1.0").await;
    on_github(true).await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (stage, result, _) = file_row(&app, &waiting).await;
    assert_eq!((stage.as_deref(), result), (Some("waiting_for_scan"), None));
    assert_eq!(release_status(&app, &waiting).await.0, "fetching");
    let list = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    let progress = &list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == waiting.as_str())
        .unwrap()["progress"];
    assert_eq!(
        progress["current"]["step_label"], "Waiting for its scan from GitHub",
        "{progress}"
    );
    assert!(
        progress["summary"].as_str().unwrap().starts_with(
            "Checking file 1 of 1: Linux (.tar.gz), waiting for its scan from GitHub."
        ),
        "{progress}"
    );
    assert!(!releases::fetch_next(&app.state).await.unwrap());

    // It comes (Slop Check's report, slop_build.rs): the release goes on, scanned on GitHub.
    keep(sha.clone(), github_report(&[])).await;
    assert_eq!(
        releases::give_waiting_scans(&app.state, app_id, &sha)
            .await
            .unwrap(),
        1
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &waiting).await.0, "ready");
    assert_eq!(file_row(&app, &waiting).await.1.as_deref(), Some("passed"));
    assert_eq!(runs(&counted), 1);

    // A scan that never comes: after the wait, SLAPorSLOP scans the file itself.
    sqlx::query("delete from ci_scans where sha256 = $1")
        .bind(&sha)
        .execute(&app.db)
        .await
        .unwrap();
    on_github(false).await;
    let late = send_release(&app, &maker, "1.2.0").await;
    on_github(true).await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(
        file_row(&app, &late).await.0.as_deref(),
        Some("waiting_for_scan")
    );
    sqlx::query(
        "update releases set scans_waiting_since = now() - interval '31 minutes' where id = $1::uuid",
    )
    .bind(&late)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (stage, result, report) = file_row(&app, &late).await;
    assert_eq!((stage, result.as_deref()), (None, Some("passed")));
    assert_eq!(report.unwrap()["engines"]["clamav"], "here");
    assert_eq!(runs(&counted), 2);
    assert_eq!(release_status(&app, &late).await.0, "ready");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A finding in a scan from GitHub holds the release for staff, like one found here.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_finding_from_github_holds_the_release(db: PgPool) {
    use sha2::Digest;
    let dir = std::env::temp_dir().join(format!("slop-ci-scans-{}", Uuid::now_v7()));
    let (command, counted) = counting_scanner(&dir);
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    sqlx::query("update apps set platforms = '{linux}'")
        .execute(&app.db)
        .await
        .unwrap();
    let app_id = app_id(&app, "game").await;
    let sha = hex::encode(sha2::Sha256::digest(BUILD_BYTES));
    sqlx::query("insert into ci_scans (app_id, sha256, report) values ($1, $2, $3)")
        .bind(app_id)
        .bind(&sha)
        .bind(github_report(&["Win.Trojan.Test"]))
        .execute(&app.db)
        .await
        .unwrap();
    let id = send_release(&app, &maker, "1.0.0").await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &id).await.0, "awaiting_review");
    let h = body_json(
        app.send(req(Method::GET, format!("/v1/apps/game/releases/{id}/hold")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(h["kinds"], json!(["scan"]));
    assert_eq!(runs(&counted), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What slop-source-check prints on a maker's runner for a tree with a payment library in it.
pub(crate) fn runner_source_check() -> Value {
    json!({
        "raw": {
            "files": 2, "truncated": false,
            "licence_files": [{ "path": "LICENSE", "spdx": "MIT" }],
            "headers": [], "notices": [], "closed": [], "lockfiles": [], "packages": [],
            "monetization": [{ "rule": "unity_iap", "label": "Unity In-App Purchasing", "count": 1, "examples": ["src/Shop.cs:2"] }],
            "code_loading": [], "network": [], "saves": [], "save_folder": "none",
            "code_files": [{ "rule": "cs", "label": "C#", "count": 1, "examples": ["src/Shop.cs"] }],
            "design": [], "pulse": "none"
        },
        "tool": "slop-source-check 0.1.0"
    })
}

/// The source check run on the maker's GitHub runner is used instead of downloading the
/// repository: the store page's licence is applied here.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_source_check_from_github_is_used_instead_of_downloading(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    sqlx::query("update apps set platforms = '{linux}'")
        .execute(&app.db)
        .await
        .unwrap();
    let app_id = app_id(&app, "game").await;
    sqlx::query(
        "insert into ci_source_scans (app_id, commit_sha, raw, tool) values ($1, repeat('a', 40), $2, 'slop-source-check 0.1.0')",
    )
    .bind(app_id)
    .bind(&runner_source_check()["raw"])
    .execute(&app.db)
    .await
    .unwrap();
    let id = send_release(&app, &maker, "1.0.0").await;
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let scan: Value = sqlx::query_scalar("select repo_scan from releases where id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(scan["status"], "checked", "{scan}");
    assert_eq!(scan["checked_on"], "github");
    assert_eq!(scan["licence"]["declared"], "MIT");
    assert_eq!(scan["monetization"][0]["rule"], "unity_iap");
}
