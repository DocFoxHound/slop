// SPDX-License-Identifier: AGPL-3.0-or-later
//! SLAPorSLOP's scanning server (scan_server.rs, owner decision 2026-11-18): production sends the
//! files to a separate machine to be scanned. When it can't be reached, releases wait for it
//! instead of failing or going on hold.

use crate::*;

/// A stand-in for slop-scan that notes each run (a file's, or `--source` for a source check) and
/// passes every file.
fn counting_scanner(dir: &std::path::Path) -> (String, std::path::PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    let runs = dir.join("runs");
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in --*) echo \"$1\" ;; *) echo file ;; esac >> {runs}\necho '{{\"result\":\"passed\",\"detail\":\"nothing found\",\"findings\":[],\"capabilities\":[],\"engines\":{{\"clamav\":\"there\"}}}}'\n",
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

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_wait_for_the_scanning_server(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-scan-server-{}", Uuid::now_v7()));
    let (command, runs) = counting_scanner(&dir);
    // A free port, with nothing listening on it yet.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let token = "s".repeat(48);
    let url = format!("http://{addr}/");
    let scanner = slop_api::scan::Scanner::remote_with_pauses(&url, &token, &[]).unwrap();
    let app = test_app_with(db, move |c| c.releases.scanner = scanner).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    sqlx::query("update apps set platforms = '{linux}'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(json!({
                    "version": "1.0.0",
                    "commit_sha": "a".repeat(40),
                    "files": [{
                        "platform": "linux", "kind": "archive", "exec": "bin/game",
                        "url": format!("{}octo/game/releases/download/v1/game.tar.gz", app.fake_base),
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let later = || async {
        // As if the few minutes it waits had passed.
        sqlx::query("update releases set fetch_started_at = now() - interval '41 minutes' where id = $1::uuid")
            .bind(&id)
            .execute(&app.db)
            .await
            .unwrap();
    };

    // Down for longer than a release's three tries: it waits, and isn't failed or held.
    for _ in 0..4 {
        assert!(releases::fetch_next(&app.state).await.unwrap());
        let (status, attempts, retry_in): (String, i16, f64) = sqlx::query_as(
            "select status::text, fetch_attempts,
                    extract(epoch from fetch_started_at + interval '40 minutes' - now())::float8
             from releases where id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .unwrap();
        assert_eq!((status.as_str(), attempts), ("fetching", 0));
        assert!(
            (200.0..=300.0).contains(&retry_in),
            "tried again in five minutes: {retry_in}"
        );
        let scanned: Option<String> =
            sqlx::query_scalar("select scan_result from release_files where release_id = $1::uuid")
                .bind(&id)
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(scanned, None);
        later().await;
    }
    assert!(
        slop_api::scan::remote_down_since().is_some(),
        "admins are alerted after an hour"
    );
    assert_eq!(std::fs::read_to_string(&runs).unwrap_or_default(), "");

    // Back: the release is scanned there and goes on.
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let server = slop_api::scan_server::router(slop_api::scan_server::Settings {
        listen: addr,
        token: token.clone(),
        command,
        jobs: 1,
        max_bytes: 1 << 20,
        dir: dir.clone(),
    });
    tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &id).await.0, "ready");
    let (result, report): (String, Value) = sqlx::query_as(
        "select scan_result, scan_report from release_files where release_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(result, "passed");
    assert_eq!(report["engines"]["clamav"], "there");
    // The source check ran there too (this stand-in's answer isn't one, so it's tried again later).
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "file\n--source\n");
    assert_eq!(slop_api::scan::remote_down_since(), None);
    let _ = std::fs::remove_dir_all(&dir);
}
