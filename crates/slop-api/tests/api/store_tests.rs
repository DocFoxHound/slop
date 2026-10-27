// SPDX-License-Identifier: AGPL-3.0-or-later
//! The store and catalog, front page and shelves, For You, reviews, votes, labels, playtime, cloud saves, demos, media, notifications and push.

use crate::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------------------------
// Helpers for this area

/// A small GitHub API for licence lookups that counts what it was asked. octo/secret is private:
/// its licence is there only with SLAPorSLOP's token. octo/flaky answers 502. Every repository is
/// owned by GitHub account 42.
async fn counting_github() -> (String, Arc<Mutex<HashMap<String, usize>>>) {
    let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
    let counted = hits.clone();
    let app = Router::new()
        .route(
            "/repos/{owner}/{repo}/license",
            get(
                move |axum::extract::Path((owner, repo)): axum::extract::Path<(String, String)>,
                      headers: axum::http::HeaderMap| {
                    let counted = counted.clone();
                    async move {
                        *counted
                            .lock()
                            .unwrap()
                            .entry(format!("{owner}/{repo}"))
                            .or_default() += 1;
                        let token = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        let mit = json!({ "license": { "spdx_id": "MIT", "name": "MIT License" } });
                        match repo.as_str() {
                            "secret" if token == "Bearer gh-shared" => axum::Json(mit).into_response(),
                            "secret" | "none" => StatusCode::NOT_FOUND.into_response(),
                            "flaky" => StatusCode::BAD_GATEWAY.into_response(),
                            _ => axum::Json(mit).into_response(),
                        }
                    }
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}",
            get(|axum::extract::Path((owner, _)): axum::extract::Path<(String, String)>| async move {
                axum::Json(json!({ "private": true, "id": 5042, "owner": { "login": owner, "id": 42, "type": "User" } }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), hits)
}

fn license_lookup(url: &str) -> Req {
    req(
        Method::GET,
        format!(
            "/v1/repos/license?url={}",
            url::form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>()
        ),
    )
}

async fn count(db: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(db)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Licence detection (catalog.rs)

/// Only the repository's owner asks GitHub with SLAPorSLOP's credentials, and what they see isn't
/// cached for anyone else; public answers are cached for a day, lookups are limited per account,
/// and GitHub's other answers become a note.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn licence_lookups_use_slops_token_only_for_the_owner_and_are_cached_and_limited(db: PgPool) {
    let (github, hits) = counting_github().await;
    let app = test_app_with(db, |c| {
        c.releases.github_api_base = github.parse().unwrap();
        c.releases.github_token = Some("gh-shared".into());
    })
    .await;
    let (owner_id, owner) = app.user("Owner", "US").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', '42', $1)",
    )
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    let hits_for = |repo: &str| hits.lock().unwrap().get(repo).copied().unwrap_or(0);

    // A stranger asking about the private repository asks as nobody: no licence.
    let found = body_json(
        app.send(license_lookup("https://github.com/octo/secret").cookie(&stranger))
            .await,
    )
    .await;
    assert!(found["spdx"].is_null(), "{found}");
    assert!(found["note"].as_str().unwrap().contains("no licence file"));
    // The owner gets it with SLAPorSLOP's token...
    let found = body_json(
        app.send(license_lookup("https://github.com/octo/secret").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(found["spdx"], "MIT");
    // ...and that answer isn't cached for the stranger.
    let found = body_json(
        app.send(license_lookup("https://github.com/octo/secret").cookie(&stranger))
            .await,
    )
    .await;
    assert!(found["spdx"].is_null(), "{found}");
    assert_eq!(hits_for("octo/secret"), 3);

    // A public answer is asked for once a day, whoever asks (and in whatever case).
    for (cookie, url) in [
        (&stranger, "https://github.com/octo/open"),
        (&stranger, "https://github.com/octo/open"),
        (&owner, "https://github.com/Octo/Open"),
    ] {
        let found = body_json(app.send(license_lookup(url).cookie(cookie)).await).await;
        assert_eq!(found["spdx"], "MIT");
    }
    assert_eq!(hits_for("octo/open"), 1);

    // Other answers from GitHub are passed on as a note, not an error.
    let res = app
        .send(license_lookup("https://github.com/octo/flaky").cookie(&stranger))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let found = body_json(res).await;
    assert!(found["spdx"].is_null());
    assert!(
        found["note"]
            .as_str()
            .unwrap()
            .contains("GitHub answered 502"),
        "{found}"
    );

    // Each account gets 30 uncached lookups an hour. The stranger has used 4 (secret twice,
    // open once, flaky once).
    for _ in 0..26 {
        let res = app
            .send(license_lookup("https://github.com/octo/none").cookie(&stranger))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    let res = app
        .send(license_lookup("https://github.com/octo/none").cookie(&stranger))
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(hits_for("octo/none"), 26);
    // Cached answers still come back, and other accounts aren't affected.
    let res = app
        .send(license_lookup("https://github.com/octo/open").cookie(&stranger))
        .await;
    assert_eq!(body_json(res).await["spdx"], "MIT");
    let res = app
        .send(license_lookup("https://github.com/octo/none").cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}

/// A maker agent token for the signed-in maker, as an Authorization header value.
async fn maker_token(app: &TestApp, cookie: &str, body: Value) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(cookie)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    )
}

/// Uploads a synced save with raw headers (None leaves one out).
async fn put_save(
    app: &TestApp,
    auth: (&str, &str),
    slug: &str,
    base: Option<&str>,
    device: Option<&str>,
    body: Vec<u8>,
) -> Response {
    let mut r = Request::builder()
        .method(Method::PUT)
        .uri(format!("/v1/me/saves/{slug}"))
        .header(header::HOST, "localhost:8080")
        .header(auth.0, auth.1)
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(base) = base {
        r = r.header("x-slop-base", base);
    }
    if let Some(device) = device {
        r = r.header("x-slop-device", device);
    }
    app.router
        .clone()
        .oneshot(r.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

/// A stored save written straight to the database, `mb` megabytes long.
async fn stored_save(app: &TestApp, user: Uuid, slug: &str, generation: i32, mb: i32) {
    sqlx::query(
        "insert into save_blobs (user_id, app_id, generation, body, device)
         select $1, id, $2, convert_to(repeat('x', $3 * 1024 * 1024), 'UTF8'), 'd-old' from apps where slug = $4",
    )
    .bind(user)
    .bind(generation)
    .bind(mb)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------------------------
// Save sync (save_sync.rs)

/// Uploads need the generation they build on and a device name, refuse empty saves and apps the
/// player can't see, and aren't for makers' agents; of two uploads on the same generation one
/// wins.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn save_uploads_check_their_headers_the_app_and_who_sends_them(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    listed_with(
        &app,
        maker_id,
        "hidden",
        "released",
        "visibility = 'private'",
    )
    .await;
    listed_with(&app, maker_id, "grown-up", "released", "min_age = 18").await;
    let (_, pat) = app.user("Pat", "US").await;
    let (_, kid) = app.user("Kid", "GB").await;
    let pat = ("cookie", pat.as_str());

    // Signed out.
    let res = app.send(req(Method::GET, "/v1/me/saves")).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = put_save(
        &app,
        ("x-none", "1"),
        "game",
        Some("0"),
        Some("d-1"),
        b"s".to_vec(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Bad headers and bodies.
    for (base, device, body, field) in [
        (None, Some("d-1"), b"s".to_vec(), "x-slop-base"),
        (Some("-1"), Some("d-1"), b"s".to_vec(), "x-slop-base"),
        (Some("one"), Some("d-1"), b"s".to_vec(), "x-slop-base"),
        (Some("0"), None, b"s".to_vec(), "x-slop-device"),
        (Some("0"), Some(""), b"s".to_vec(), "x-slop-device"),
        (Some("0"), Some("my laptop"), b"s".to_vec(), "x-slop-device"),
        (
            Some("0"),
            Some(&*"d".repeat(65)),
            b"s".to_vec(),
            "x-slop-device",
        ),
        (Some("0"), Some("d-1"), vec![], "body"),
    ] {
        let res = put_save(&app, pat, "game", base, device, body).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        let body = body_json(res).await;
        assert_eq!(body["error"]["field"], field, "{body}");
    }
    // Apps the player can't see answer like missing ones.
    for slug in ["hidden", "no-such-app"] {
        let res = put_save(&app, pat, slug, Some("0"), Some("d-1"), b"s".to_vec()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{slug}");
    }
    let res = put_save(
        &app,
        ("cookie", &kid),
        "grown-up",
        Some("0"),
        Some("d-1"),
        b"s".to_vec(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(count(&app.db, "select count(*) from save_blobs").await, 0);

    // A maker's agent can't touch the maker's saves.
    let agent = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let res = put_save(
        &app,
        ("authorization", &agent),
        "game",
        Some("0"),
        Some("d-1"),
        b"s".to_vec(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(req(Method::GET, "/v1/me/saves").header("authorization", agent.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Two computers upload on the same generation at once: one wins, the other is told.
    let (a, b) = tokio::join!(
        put_save(
            &app,
            pat,
            "game",
            Some("0"),
            Some("d-home"),
            b"home".to_vec()
        ),
        put_save(
            &app,
            pat,
            "game",
            Some("0"),
            Some("d-work"),
            b"work".to_vec()
        ),
    );
    let mut statuses = [a.status(), b.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(
        count(&app.db, "select count(*) from save_blobs").await,
        1,
        "only the winner is kept"
    );
}

/// The 200 MB allowance counts the copy kept before each newest save, and the list shows one row
/// per app (its newest) with what the player uses of it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_save_allowance_counts_the_copies_kept_before_each_save(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    // Five apps with a 20 MB save and the 19 MB one before it: 195 MB kept, 95 MB of it newest.
    for i in 1..=5 {
        listed_with(&app, maker_id, &format!("app-{i}"), "released", "").await;
        stored_save(&app, pat_id, &format!("app-{i}"), 1, 20).await;
        stored_save(&app, pat_id, &format!("app-{i}"), 2, 19).await;
    }
    listed_with(&app, maker_id, "new", "released", "").await;
    let pat = ("cookie", pat.as_str());

    let list = body_json(
        app.send(req(Method::GET, "/v1/me/saves").cookie(pat.1))
            .await,
    )
    .await;
    let saves = list["saves"].as_array().unwrap();
    assert_eq!(saves.len(), 5, "one row per app: {saves:?}");
    assert!(
        saves
            .iter()
            .all(|s| s["generation"] == 2 && s["size"] == 19 * 1024 * 1024)
    );
    assert_eq!(saves[0]["device"], "d-old");
    assert_eq!(list["allowance"], 200 * 1024 * 1024);
    assert_eq!(
        list["used"].as_i64(),
        Some(195 * 1024 * 1024),
        "what counts against the allowance includes the kept copies"
    );

    // 6 MB more on a new app goes over (195 + 6 > 200), though the newest saves are only 101.
    let six = || vec![1u8; 6 * 1024 * 1024];
    let res = put_save(&app, pat, "new", Some("0"), Some("d-1"), six()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let body = body_json(res).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("200 MB"),
        "{body}"
    );
    // Replacing a save drops the copy before it (20 MB), so then it fits.
    let res = put_save(&app, pat, "app-1", Some("2"), Some("d-1"), six()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = put_save(&app, pat, "new", Some("0"), Some("d-1"), six()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/saves").cookie(pat.1))
            .await,
    )
    .await;
    assert_eq!(list["used"].as_i64(), Some(187 * 1024 * 1024));
}

/// Stopping sync for an app no one has (or that doesn't exist) does nothing, and never touches
/// another player's saves for the same app.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stopping_save_sync_only_deletes_the_players_own_saves(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    stored_save(&app, pat_id, "tower", 1, 0).await;
    stored_save(&app, sam_id, "tower", 1, 0).await;
    stored_save(&app, sam_id, "tower", 2, 0).await;

    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/saves/tower"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/saves/no-such-app").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(count(&app.db, "select count(*) from save_blobs").await, 3);
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/saves/tower").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let left: Vec<(Uuid, i32)> =
        sqlx::query_as("select user_id, generation from save_blobs order by generation")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(left, [(sam_id, 1), (sam_id, 2)]);
    let res = app
        .send(req(Method::GET, "/v1/me/saves/tower").cookie(&sam))
        .await;
    assert_eq!(res.headers()["x-slop-generation"], "2");
    // Again: still nothing to do.
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/saves/tower").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

// ---------------------------------------------------------------------------------------------
// Browser demos (demos.rs)

/// A zip of these files, stored (not compressed) unless `deflate`.
fn zip_of(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(if deflate {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    });
    for (name, data) in files {
        w.start_file(*name, options).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A release page for web builds: a good one, one without index.html, one too big zipped, one
/// too big unpacked (with the test limits below), one the scanner holds, and a slow good one.
async fn demo_file_host() -> String {
    let app = Router::new().route(
        "/octo/game/releases/download/{tag}/{file}",
        get(
            |axum::extract::Path((_, file)): axum::extract::Path<(String, String)>| async move {
                let index: &[u8] = b"<!doctype html><script src=game.js></script>";
                match file.as_str() {
                    "web.zip" => zip_of(&[("index.html", index), ("game.js", b"go()")], false),
                    "noindex.zip" => zip_of(&[("game/main.html", index)], false),
                    "big.zip" => {
                        zip_of(&[("index.html", index), ("pad.bin", &[7u8; 20_000])], false)
                    }
                    "bomb.zip" => zip_of(
                        &[("index.html", index), ("zeros.bin", &[0u8; 50_000])],
                        true,
                    ),
                    "held.zip" => {
                        zip_of(&[("index.html", index), ("evil.js", b"HELD-MARKER")], false)
                    }
                    "slow.zip" => {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        zip_of(&[("index.html", index)], false)
                    }
                    _ => return StatusCode::NOT_FOUND.into_response(),
                }
                .into_response()
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

/// A scanner stand-in that holds files containing HELD-MARKER and passes the rest.
fn marker_scanner() -> String {
    let dir = std::env::temp_dir().join(format!("slop-scan-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        "#!/bin/sh\nif grep -q HELD-MARKER \"$1\"; then echo '{\"result\":\"on_hold\",\"detail\":\"Flagged by ClamAV: Js.Test\"}'; else echo '{\"result\":\"passed\",\"detail\":\"nothing found\"}'; fi\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    script.to_str().unwrap().to_string()
}

struct DemoSetup {
    app: TestApp,
    dir: std::path::PathBuf,
    host: String,
    maker: String,
    maker_id: Uuid,
}

/// Demos on, with small size limits, the marker scanner, and a newly released listed "game"
/// whose repository is the demo file host.
async fn demo_setup(db: PgPool, max_live: i64) -> DemoSetup {
    let dir = std::env::temp_dir().join(format!("slop-demos-test-{}", Uuid::now_v7()));
    let demo_dir = dir.clone();
    let scanner = marker_scanner();
    let app = test_app_with(db, move |c| {
        c.demos.play_url = Some("http://play.localhost:8080".parse().unwrap());
        c.demos.dir = demo_dir;
        c.demos.max_live = max_live;
        c.demos.max_zip_bytes = 10_000;
        c.demos.max_unpacked_bytes = 20_000;
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &scanner).unwrap();
    })
    .await;
    let host = demo_file_host().await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    sqlx::query(
        "update apps set status = 'listed', stage_entered_at = now(), repo_url = $1 where slug = 'game'",
    )
    .bind(format!("{host}octo/game"))
    .execute(&app.db)
    .await
    .unwrap();
    DemoSetup {
        app,
        dir,
        host,
        maker,
        maker_id,
    }
}

impl DemoSetup {
    fn file(&self, name: &str) -> String {
        format!("{}octo/game/releases/download/v1/{name}", self.host)
    }

    async fn start(&self, auth: Req, name: &str) -> Response {
        self.app
            .send(auth.json(json!({ "url": self.file(name) })))
            .await
    }

    /// The demo's status once it's no longer fetching.
    async fn settled(&self) -> Value {
        for _ in 0..200 {
            let status = body_json(
                self.app
                    .send(req(Method::GET, "/v1/apps/game/demo").cookie(&self.maker))
                    .await,
            )
            .await;
            if status["demo"]["status"] != "fetching" {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the demo never finished fetching");
    }

    async fn play(&self, path: &str) -> StatusCode {
        self.app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::HOST, "play.localhost:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
}

/// Starting a demo: only the maker, only when demos are on, only from the app's repository,
/// only while a slot is free; builds that fail the scan, have no index.html or are too big end
/// up failed with the reason, and nothing is served for them.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn browser_demos_refuse_strangers_bad_builds_and_full_slots(db: PgPool) {
    let s = demo_setup(db, 2).await;
    let app = &s.app;
    let post = || req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker);
    let (_, stranger) = app.user("Stranger", "US").await;

    // Not the maker: as if the app weren't there. Signed out: 401.
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&stranger),
            "web.zip",
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    for m in [Method::GET, Method::DELETE] {
        let res = app
            .send(req(m.clone(), "/v1/apps/game/demo").cookie(&stranger))
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{m}");
    }
    let res = s
        .start(req(Method::POST, "/v1/apps/game/demo"), "web.zip")
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // A file somewhere other than the repository's releases.
    let res = app
        .send(
            post().json(
                json!({ "url": "https://example.com/octo/game/releases/download/v1/web.zip" }),
            ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(&app.db, "select count(*) from demos").await, 0);

    // Every slot taken by other apps' demos (live or still fetching).
    for (slug, status) in [("other-1", "live"), ("other-2", "fetching")] {
        listed_with(app, s.maker_id, slug, "released", "").await;
        sqlx::query(
            "insert into demos (app_id, url, status, ends_at) select id, 'x', $2, now() + interval '1 day' from apps where slug = $1",
        )
        .bind(slug)
        .bind(status)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let res = s.start(post(), "web.zip").await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // Ended and failed ones don't hold a slot.
    sqlx::query("update demos set status = 'ended' where url = 'x' and status = 'live'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update demos set status = 'failed' where url = 'x'")
        .execute(&app.db)
        .await
        .unwrap();

    // Builds that fail: each says why, and nothing is served.
    for (file, why) in [
        ("held.zip", "the scan didn't pass (on_hold)"),
        ("noindex.zip", "needs an index.html"),
        ("big.zip", "over 100 MB"),
        ("bomb.zip", "over 250 MB unpacked"),
    ] {
        let res = s.start(post(), file).await;
        assert_eq!(res.status(), StatusCode::ACCEPTED, "{file}");
        let status = s.settled().await;
        assert_eq!(status["demo"]["status"], "failed", "{file}: {status}");
        assert!(
            status["demo"]["detail"].as_str().unwrap().contains(why),
            "{file}: {status}"
        );
        assert!(status["play_url"].is_null());
        assert!(!s.dir.join("game").exists(), "{file}");
        assert_eq!(s.play("/play/game/").await, StatusCode::NOT_FOUND);
    }
    let held: (Option<String>, Option<i64>) =
        sqlx::query_as("select scan_result, size_bytes from demos d join apps a on a.id = d.app_id where a.slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(held.1.is_some());

    // A good build afterwards goes live.
    assert_eq!(
        s.start(post(), "web.zip").await.status(),
        StatusCode::ACCEPTED
    );
    let status = s.settled().await;
    assert_eq!(status["demo"]["status"], "live", "{status}");
    assert!(status["demo"]["detail"].is_null(), "{status}");
    assert_eq!(status["demo"]["scan_result"], "passed");
    assert_eq!(s.play("/play/game/game.js").await, StatusCode::OK);
}

/// The status page says whether demos exist here and whether the app can have one; a private,
/// delisted or long-released app can't, and the play address shows only while one is live.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn demo_status_says_when_an_app_can_have_one(db: PgPool) {
    let s = demo_setup(db, 10).await;
    let app = &s.app;
    // The same app on a SLAPorSLOP without a play origin: not available, and starting one is refused.
    let off = test_app(app.db.clone()).await;
    let status = body_json(
        off.send(req(Method::GET, "/v1/apps/game/demo").cookie(&s.maker))
            .await,
    )
    .await;
    assert_eq!(status["available"], false);
    assert_eq!(status["eligible"], true);
    assert!(status["demo"].is_null() && status["play_url"].is_null());
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
            "web.zip",
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "but the same request works where demos are on"
    );
    sqlx::query("delete from demos")
        .execute(&app.db)
        .await
        .unwrap();
    let res = off
        .send(
            req(Method::POST, "/v1/apps/game/demo")
                .cookie(&s.maker)
                .json(json!({ "url": s.file("web.zip") })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(count(&app.db, "select count(*) from demos").await, 0);

    let status = || async {
        body_json(
            app.send(req(Method::GET, "/v1/apps/game/demo").cookie(&s.maker))
                .await,
        )
        .await
    };
    let fresh = status().await;
    assert_eq!(
        (fresh["available"].clone(), fresh["eligible"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(fresh["days"], 14);
    assert!(fresh["eligible_until"].is_string());
    for (set, why) in [
        ("visibility = 'private'", "private"),
        ("status = 'delisted'", "delisted"),
        ("stage = 'beta'", "not released"),
        (
            "stage_entered_at = now() - interval '15 days'",
            "released too long ago",
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update apps set {set} where slug = 'game'"
        )))
        .execute(&app.db)
        .await
        .unwrap();
        let st = status().await;
        assert_eq!(st["eligible"], false, "{why}");
        assert!(st["eligible_until"].is_null(), "{why}");
        let res = s
            .start(
                req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
                "web.zip",
            )
            .await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "{why}");
        sqlx::query(
            "update apps set visibility = 'public', status = 'listed', stage = 'released', stage_entered_at = now() where slug = 'game'",
        )
        .execute(&app.db)
        .await
        .unwrap();
    }

    // The play address only while it's live.
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
            "web.zip",
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let live = s.settled().await;
    assert_eq!(live["play_url"], "http://play.localhost:8080/play/game/");
    sqlx::query("update demos set status = 'failed'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(status().await["play_url"].is_null());
}

/// Ending a demo: ended at once, its files deleted, the play address gone; only the maker can.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_end_their_browser_demo(db: PgPool) {
    let s = demo_setup(db, 10).await;
    let app = &s.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
            "web.zip",
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    assert_eq!(s.settled().await["demo"]["status"], "live");
    assert_eq!(s.play("/play/game/").await, StatusCode::OK);

    let res = app
        .send(req(Method::DELETE, "/v1/apps/game/demo").cookie(&stranger))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(s.play("/play/game/").await, StatusCode::OK);
    assert!(s.dir.join("game").exists());

    let res = app
        .send(req(Method::DELETE, "/v1/apps/game/demo").cookie(&s.maker))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let status = s.settled().await;
    assert_eq!(status["demo"]["status"], "ended");
    assert!(status["play_url"].is_null());
    let ended: bool = sqlx::query_scalar("select ends_at <= now() from demos")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(ended);
    assert!(!s.dir.join("game").exists());
    assert_eq!(s.play("/play/game/").await, StatusCode::NOT_FOUND);
    assert_eq!(s.play("/play/game/game.js").await, StatusCode::NOT_FOUND);
}

/// Ending a demo while its build is still being fetched leaves no files behind.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_demo_ended_while_fetching_leaves_no_files(db: PgPool) {
    let s = demo_setup(db, 10).await;
    let app = &s.app;
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
            "slow.zip",
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let res = app
        .send(req(Method::DELETE, "/v1/apps/game/demo").cookie(&s.maker))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    // The slow download finishes after the end.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let status = s.settled().await;
    assert_eq!(status["demo"]["status"], "ended");
    assert!(
        !s.dir.join("game").exists(),
        "an ended demo's files are deleted, and expire() never looks at ended demos again"
    );
    assert_eq!(slop_api::demos::expire(&app.state).await.unwrap(), 0);
}

/// The hourly job ends the demos of apps that left the store or went private, and failed ones
/// past their days.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn demos_of_apps_that_left_the_store_are_ended(db: PgPool) {
    let s = demo_setup(db, 10).await;
    let app = &s.app;
    let res = s
        .start(
            req(Method::POST, "/v1/apps/game/demo").cookie(&s.maker),
            "web.zip",
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    assert_eq!(s.settled().await["demo"]["status"], "live");
    for slug in ["private-one", "failed-old", "failed-new", "fine"] {
        listed_with(app, s.maker_id, slug, "released", "").await;
    }
    sqlx::query("update apps set visibility = 'private' where slug = 'private-one'")
        .execute(&app.db)
        .await
        .unwrap();
    for (slug, status, ends) in [
        ("private-one", "live", "1 day"),
        ("failed-old", "failed", "-1 day"),
        ("failed-new", "failed", "1 day"),
        ("fine", "live", "1 day"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into demos (app_id, url, status, ends_at) select id, 'x', '{status}', now() + interval '{ends}' from apps where slug = $1"
        )))
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Nothing has left yet but the private one and the old failure.
    assert_eq!(slop_api::demos::expire(&app.state).await.unwrap(), 2);
    assert_eq!(s.play("/play/game/").await, StatusCode::OK);

    // The game is delisted: its demo ends and its files go.
    sqlx::query("update apps set status = 'delisted' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::demos::expire(&app.state).await.unwrap(), 1);
    assert!(!s.dir.join("game").exists());
    let statuses: Vec<(String, String)> = sqlx::query_as(
        "select a.slug, d.status from demos d join apps a on a.id = d.app_id order by a.slug",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        statuses,
        [
            ("failed-new".to_string(), "failed".to_string()),
            ("failed-old".into(), "ended".into()),
            ("fine".into(), "live".into()),
            ("game".into(), "ended".into()),
            ("private-one".into(), "ended".into()),
        ]
    );
    assert_eq!(slop_api::demos::expire(&app.state).await.unwrap(), 0);
}

/// Makers' agents: a read token sees the demo but can't host one; a token for other apps can't
/// see this one; MCP errors come back as tool errors.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_agents_see_and_host_demos_within_their_token(db: PgPool) {
    let s = demo_setup(db, 10).await;
    let app = &s.app;
    listed_with(app, s.maker_id, "other", "released", "").await;
    let publish = maker_token(
        app,
        &s.maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let read = maker_token(app, &s.maker, json!({ "name": "Watcher", "scope": "read" })).await;
    let other_only = maker_token(
        app,
        &s.maker,
        json!({ "name": "Other", "scope": "publish", "apps": ["other"] }),
    )
    .await;
    let call = |token: &str, name: &str, args: Value| {
        req(Method::POST, "/v1/maker/mcp")
            .header("authorization", token.to_string())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }))
    };

    let seen = body_json(
        app.send(call(&read, "browser_demo", json!({ "slug": "game" })))
            .await,
    )
    .await;
    assert_eq!(seen["result"]["isError"], false, "{seen}");
    assert!(
        seen["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"eligible\":true")
    );
    let refused = body_json(
        app.send(call(
            &read,
            "host_browser_demo",
            json!({ "slug": "game", "url": s.file("web.zip") }),
        ))
        .await,
    )
    .await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    for token in [&read, &other_only] {
        let res = app
            .send(
                req(Method::POST, "/v1/apps/game/demo")
                    .header("authorization", token.to_string())
                    .json(json!({ "url": s.file("web.zip") })),
            )
            .await;
        assert!(
            [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND].contains(&res.status()),
            "{}",
            res.status()
        );
    }
    let res = app
        .send(req(Method::GET, "/v1/apps/game/demo").header("authorization", other_only.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(count(&app.db, "select count(*) from demos").await, 0);

    let hosted = body_json(
        app.send(call(
            &publish,
            "host_browser_demo",
            json!({ "slug": "game", "url": s.file("web.zip") }),
        ))
        .await,
    )
    .await;
    assert_eq!(hosted["result"]["isError"], false, "{hosted}");
    assert_eq!(s.settled().await["demo"]["status"], "live");
    let res = app
        .send(req(Method::DELETE, "/v1/apps/game/demo").header("authorization", read.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(req(Method::DELETE, "/v1/apps/game/demo").header("authorization", publish.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------------------------
// Notifications and email (notify.rs)

/// A notification written straight to the database, `age` old (a Postgres interval).
async fn notice(app: &TestApp, user: Uuid, category: &str, title: &str, age: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into notifications (id, user_id, category, title, link, created_at)
         values ($1, $2, $3, $4, '/account', now() - interval '{age}')"
    )))
    .bind(id)
    .bind(user)
    .bind(category)
    .bind(title)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

async fn inbox(app: &TestApp, r: Req) -> Value {
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn titles(inbox: &Value) -> Vec<String> {
    inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

/// The bell: newest first, at most 60, with the unread count. A maker's agent sees only maker
/// work: never chats, friends or account security, and People jobs only when its token may
/// handle them, and the unread count it sees counts only what it can see.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_bell_lists_newest_first_and_agents_see_only_maker_work(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (other_id, _) = app.user("Other", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/notifications"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for (i, category) in ["apps", "chat", "friends", "security", "jobs", "moderation"]
        .iter()
        .enumerate()
    {
        notice(
            &app,
            maker_id,
            category,
            category,
            &format!("{} minutes", 10 - i),
        )
        .await;
    }
    notice(&app, other_id, "apps", "someone else's", "1 minute").await;

    let all = inbox(
        &app,
        req(Method::GET, "/v1/me/notifications").cookie(&maker),
    )
    .await;
    assert_eq!(
        titles(&all),
        ["moderation", "jobs", "security", "friends", "chat", "apps"]
    );
    assert_eq!(all["unread"], 6);
    assert_eq!(all["items"][0]["read"], false);
    assert_eq!(all["items"][0]["count"], 1);

    let plain = maker_token(&app, &maker, json!({ "name": "Claude", "scope": "read" })).await;
    let seen = inbox(
        &app,
        req(Method::GET, "/v1/me/notifications").header("authorization", plain.clone()),
    )
    .await;
    assert_eq!(titles(&seen), ["moderation", "apps"]);
    assert_eq!(
        seen["unread"], 2,
        "only what the agent can see counts: {seen}"
    );
    let people = maker_token(
        &app,
        &maker,
        json!({ "name": "Hirer", "scope": "read", "people": true }),
    )
    .await;
    let seen = inbox(
        &app,
        req(Method::GET, "/v1/me/notifications").header("authorization", people),
    )
    .await;
    assert_eq!(titles(&seen), ["moderation", "jobs", "apps"]);
    assert_eq!(seen["unread"], 3, "{seen}");
    // The whats_new tool is the same list.
    let called = body_json(
        app.send(
            req(Method::POST, "/v1/maker/mcp")
                .header("authorization", plain)
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "whats_new", "arguments": {} } })),
        )
        .await,
    )
    .await;
    let text = called["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("moderation") && !text.contains("security"),
        "{text}"
    );

    // At most 60, newest first; the count is of all unread.
    for i in 0..65 {
        notice(
            &app,
            maker_id,
            "apps",
            &format!("old {i}"),
            &format!("{} hours", i + 1),
        )
        .await;
    }
    let all = inbox(
        &app,
        req(Method::GET, "/v1/me/notifications").cookie(&maker),
    )
    .await;
    let t = titles(&all);
    assert_eq!(t.len(), 60);
    assert_eq!(&t[..2], ["moderation", "jobs"]);
    assert_eq!(t[59], "old 53");
    assert_eq!(all["unread"], 71);
}

/// Marking read: only the listed ids, never someone else's; no ids marks them all.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn marking_notifications_read_touches_only_the_callers_own(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    let a = notice(&app, pat_id, "apps", "a", "3 minutes").await;
    let b = notice(&app, pat_id, "apps", "b", "2 minutes").await;
    let _c = notice(&app, pat_id, "apps", "c", "1 minute").await;
    let theirs = notice(&app, sam_id, "apps", "theirs", "1 minute").await;
    let read = |cookie: Option<&str>, body: Value| {
        let r = req(Method::POST, "/v1/me/notifications/read").json(body);
        match cookie {
            Some(c) => r.cookie(c),
            None => r,
        }
    };
    assert_eq!(
        app.send(read(None, json!({ "ids": [a] }))).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(read(Some(&pat), json!({ "ids": "all" })))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app
        .send(read(Some(&pat), json!({ "ids": [a, theirs] })))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let mine = inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&pat)).await;
    assert_eq!(mine["unread"], 2);
    let read_ones: Vec<&str> = mine["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["read"] == true)
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert_eq!(read_ones, ["a"]);
    let other = inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&sam)).await;
    assert_eq!(other["unread"], 1, "someone else's id is ignored");

    // Already read stays as it was read; no ids reads the rest.
    let first_read: OffsetDateTime =
        sqlx::query_scalar("select read_at from notifications where id = $1")
            .bind(a)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let res = app.send(read(Some(&pat), json!({}))).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&pat)).await["unread"],
        0
    );
    let again: OffsetDateTime =
        sqlx::query_scalar("select read_at from notifications where id = $1")
            .bind(a)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(first_read, again);
    assert_eq!(
        count(
            &app.db,
            &format!("select count(*) from notifications where id = '{b}' and read_at is not null")
        )
        .await,
        1
    );
    assert_eq!(
        inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&sam)).await["unread"],
        1
    );
}

/// The inbox's filters: unread per kind, one kind, unread only, and older pages after a given
/// notification (which must be the caller's own); an unknown kind is refused.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_bell_filters_by_kind_and_unread_and_pages_older(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    for i in 0..5 {
        notice(
            &app,
            pat_id,
            "apps",
            &format!("app {i}"),
            &format!("{} minutes", 10 + i),
        )
        .await;
    }
    let friend = notice(&app, pat_id, "friends", "friend", "1 minute").await;
    let read_one = notice(&app, pat_id, "friends", "old friend", "1 day").await;
    sqlx::query("update notifications set read_at = now() where id = $1")
        .bind(read_one)
        .execute(&app.db)
        .await
        .unwrap();
    let theirs = notice(&app, sam_id, "apps", "theirs", "2 minutes").await;
    let get = |q: &str| req(Method::GET, format!("/v1/me/notifications{q}")).cookie(&pat);

    let all = inbox(&app, get("")).await;
    assert_eq!(all["unread"], 6);
    assert_eq!(all["counts"], json!({ "apps": 5, "friends": 1 }));
    assert_eq!(all["more"], false);
    assert_eq!(titles(&all).len(), 7);

    let friends = inbox(&app, get("?category=friends")).await;
    assert_eq!(titles(&friends), ["friend", "old friend"]);
    assert_eq!(friends["unread"], 6, "the counts are of every kind");
    let unread = inbox(&app, get("?category=friends&unread=true")).await;
    assert_eq!(titles(&unread), ["friend"]);
    assert_eq!(titles(&inbox(&app, get("?unread=true")).await).len(), 6);

    // Two at a time, newest first, each page after the last one shown.
    let first = inbox(&app, get("?category=apps&limit=2")).await;
    assert_eq!(titles(&first), ["app 0", "app 1"]);
    assert_eq!(first["more"], true);
    let last = first["items"][1]["id"].as_str().unwrap().to_string();
    let second = inbox(&app, get(&format!("?category=apps&limit=2&before={last}"))).await;
    assert_eq!(titles(&second), ["app 2", "app 3"]);
    assert_eq!(second["more"], true);
    let last = second["items"][1]["id"].as_str().unwrap().to_string();
    let third = inbox(&app, get(&format!("?category=apps&limit=2&before={last}"))).await;
    assert_eq!(titles(&third), ["app 4"]);
    assert_eq!(third["more"], false);
    // Across kinds, after the friend request.
    let after = inbox(&app, get(&format!("?limit=1&before={friend}"))).await;
    assert_eq!(titles(&after), ["app 0"]);

    // Someone else's notification isn't a place to page from.
    let nothing = inbox(&app, get(&format!("?before={theirs}"))).await;
    assert_eq!(titles(&nothing).len(), 0);
    assert_eq!(nothing["more"], false);
    let res = app.send(get("?category=marketing")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "\"marketing\" isn't a kind of notification"
    );
    // At most 100 a page.
    assert_eq!(titles(&inbox(&app, get("?limit=1000")).await).len(), 7);
    let sams = inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&sam)).await;
    assert_eq!(titles(&sams), ["theirs"]);
    assert_eq!(sams["counts"], json!({ "apps": 1 }));
}

/// Mark all as read for one kind leaves the others; marking unread again and dismissing touch
/// only the caller's own. An unread one that a newer unread one of the same thread would merge
/// with stays on its own, and isn't pushed to phones again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn notifications_read_by_kind_unread_again_and_dismissed(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    let a = notice(&app, pat_id, "apps", "a", "3 minutes").await;
    let f = notice(&app, pat_id, "friends", "f", "2 minutes").await;
    let theirs = notice(&app, sam_id, "friends", "theirs", "1 minute").await;
    let post = |path: &str, cookie: &str, body: Value| {
        req(Method::POST, format!("/v1/me/notifications/{path}"))
            .cookie(cookie)
            .json(body)
    };
    let mine = |cookie: &str| req(Method::GET, "/v1/me/notifications").cookie(cookie);

    let res = app
        .send(post("read", &pat, json!({ "category": "nope" })))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(post("read", &pat, json!({ "category": "friends" })))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let inbox_now = inbox(&app, mine(&pat)).await;
    assert_eq!(inbox_now["counts"], json!({ "apps": 1 }));
    assert_eq!(inbox(&app, mine(&sam)).await["unread"], 1);

    // Unread again: only read ones of the caller's; between 1 and 100 ids.
    for body in [json!({ "ids": [] }), json!({})] {
        let res = app.send(post("unread", &pat, body)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let res = app
        .send(post("unread", &pat, json!({ "ids": [f, theirs, a] })))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let inbox_now = inbox(&app, mine(&pat)).await;
    assert_eq!(inbox_now["counts"], json!({ "apps": 1, "friends": 1 }));
    sqlx::query("update notifications set read_at = now() where id = $1")
        .bind(theirs)
        .execute(&app.db)
        .await
        .unwrap();
    app.send(post("unread", &pat, json!({ "ids": [theirs] })))
        .await;
    assert_eq!(
        inbox(&app, mine(&sam)).await["unread"],
        0,
        "not Pat's to change"
    );

    // A thread: read, then a newer message came in on its own row (only one unread row per
    // thread). The old one marked unread keeps both, and isn't pushed.
    let old = notice(&app, pat_id, "chat", "1 new message", "1 hour").await;
    let newer = notice(&app, pat_id, "chat", "another message", "1 minute").await;
    sqlx::query(
        "update notifications set collapse_key = 'chat:1', read_at = case when id = $1 then now() end
         where id = any($2)",
    )
    .bind(old)
    .bind(vec![old, newer])
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(post("unread", &pat, json!({ "ids": [old] })))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let rows: Vec<(Uuid, Option<String>, bool)> = sqlx::query_as(
        "select id, collapse_key, pushed_at is not null from notifications
         where id = any($1) order by created_at",
    )
    .bind(vec![old, newer])
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [(old, None, true), (newer, Some("chat:1".into()), false)]
    );
    assert_eq!(inbox(&app, mine(&pat)).await["counts"]["chat"], 2);

    // Dismissed: gone for its owner; someone else's is not found and stays.
    let dismiss = |id: Uuid, cookie: &str| {
        req(Method::DELETE, format!("/v1/me/notifications/{id}")).cookie(cookie)
    };
    assert_eq!(
        app.send(dismiss(a, &pat)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(dismiss(a, &pat)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(dismiss(theirs, &pat)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::DELETE, format!("/v1/me/notifications/{f}")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!titles(&inbox(&app, mine(&pat)).await).contains(&"a".to_string()));
    assert_eq!(titles(&inbox(&app, mine(&sam)).await), ["theirs"]);

    // A maker's AI agent can only read the bell, not change it.
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let m = notice(&app, maker_id, "apps", "m", "1 minute").await;
    let token = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    for r in [
        req(Method::DELETE, format!("/v1/me/notifications/{m}")),
        req(Method::POST, "/v1/me/notifications/read").json(json!({})),
    ] {
        let res = app.send(r.header("authorization", token.clone())).await;
        assert!(res.status().is_client_error(), "{}", res.status());
    }
    assert_eq!(
        inbox(&app, mine(&maker)).await["counts"],
        json!({ "apps": 1 })
    );
}

/// Email choices: every category on by default, whether this SLAPorSLOP sends email and whether the
/// account has an address; only known categories and true/false, and a change keeps the rest.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn email_preferences_default_on_and_change_one_at_a_time(db: PgPool) {
    let app = test_app(db.clone()).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/email-prefs"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/email-prefs").json(json!({ "jobs": false })))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let prefs = inbox(&app, req(Method::GET, "/v1/me/email-prefs").cookie(&pat)).await;
    assert_eq!(prefs["email_on"], false, "no SMTP on this server");
    assert_eq!(prefs["has_email"], false);
    let cats = prefs["categories"].as_array().unwrap();
    assert_eq!(cats.len(), 11);
    assert!(cats.iter().any(|c| c["category"] == "agents"), "{prefs}");
    assert!(
        cats.iter()
            .all(|c| c["email"] == true && c["label"].is_string())
    );

    for bad in [
        json!({ "jobs": "no" }),
        json!({ "jobs": 0 }),
        json!({ "jobs": null }),
        json!({ "ops": false }),
    ] {
        let res = app
            .send(
                req(Method::PUT, "/v1/me/email-prefs")
                    .cookie(&pat)
                    .json(bad.clone()),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let res = app
        .send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&pat)
                .json(json!({ "jobs": false, "chat": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&pat)
                .json(json!({ "chat": true })),
        )
        .await;
    let prefs = body_json(res).await;
    let off: Vec<&str> = prefs["categories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["email"] == false)
        .map(|c| c["category"].as_str().unwrap())
        .collect();
    assert_eq!(off, ["jobs"], "the earlier change is kept");

    // With SMTP and an address.
    sqlx::query("update users set email = 'pat@example.com' where id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    let mailing = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let prefs = inbox(
        &mailing,
        req(Method::GET, "/v1/me/email-prefs").cookie(&pat),
    )
    .await;
    assert_eq!(
        (prefs["email_on"].clone(), prefs["has_email"].clone()),
        (json!(true), json!(true))
    );
}

/// A maker's bug-report answers merge into one unread notification for the player, counted,
/// and long notes are cut to fit whether the notification is new or merged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn merged_notifications_are_counted_and_cut_to_fit(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(&pat))
        .await;
    let res = app
        .send(req(Method::POST, "/v1/apps/tower/bug-reports").cookie(&pat).json(json!({
            "title": "Crash", "body": "It crashes.", "version": "1.0.0", "platform": "windows"
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let answer = |status: &str, note: String| {
        req(Method::PATCH, format!("/v1/bug-reports/{id}"))
            .cookie(&maker)
            .json(json!({ "status": status, "maker_note": note }))
    };

    let res = app.send(answer("wont_fix", "w".repeat(1500))).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app.send(answer("fixed", "f".repeat(1500))).await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let rows: Vec<(String, String, i32)> =
        sqlx::query_as("select title, body, count from notifications where user_id = $1")
            .bind(pat_id)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1, "merged into one");
    let (title, body, n) = &rows[0];
    assert_eq!(*n, 2);
    assert_eq!(title, "Your bug report was fixed");
    assert_eq!(body.chars().count(), 1000);
    assert!(
        body.starts_with("\"Crash\": fff") && body.ends_with('…'),
        "{body}"
    );

    // Once read, the next one is a new notification.
    app.send(
        req(Method::POST, "/v1/me/notifications/read")
            .cookie(&pat)
            .json(json!({})),
    )
    .await;
    let res = app.send(answer("open", "again".into())).await;
    assert_eq!(res.status(), StatusCode::OK);
    let inbox = inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&pat)).await;
    assert_eq!(inbox["items"].as_array().unwrap().len(), 2);
    assert_eq!(inbox["items"][0]["title"], "Your bug report is open again");
    assert_eq!(inbox["items"][0]["count"], 1);

    // A closed account gets nothing.
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(answer("fixed", "for real".into())).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        count(
            &app.db,
            &format!("select count(*) from notifications where user_id = '{pat_id}'")
        )
        .await,
        2
    );
}

/// Notifications become emails only while they're news, for accounts with an address that are
/// open and want that kind; the rest are marked skipped and never looked at again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn notification_emails_skip_old_news_and_accounts_without_email(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let mut ids = HashMap::new();
    for (name, email, closed, prefs) in [
        ("wants", Some("w@example.com"), false, "{}"),
        ("no-email", None, false, "{}"),
        ("closed", Some("c@example.com"), true, "{}"),
        (
            "not-apps",
            Some("n@example.com"),
            false,
            r#"{"apps": false}"#,
        ),
    ] {
        let (id, _) = app.user(name, "US").await;
        sqlx::query(
            "update users set email = $2, closed_at = case when $3 then now() end, email_prefs = $4::jsonb where id = $1",
        )
        .bind(id)
        .bind(email)
        .bind(closed)
        .bind(prefs)
        .execute(&app.db)
        .await
        .unwrap();
        ids.insert(name, id);
    }
    let fresh = notice(&app, ids["wants"], "apps", "fresh", "1 hour").await;
    let stale = notice(&app, ids["wants"], "apps", "stale", "25 hours").await;
    let other_kind = notice(&app, ids["not-apps"], "moderation", "decided", "1 minute").await;
    let unwanted = notice(&app, ids["not-apps"], "apps", "unwanted", "1 minute").await;
    let no_email = notice(&app, ids["no-email"], "apps", "nowhere", "1 minute").await;
    let closed = notice(&app, ids["closed"], "apps", "gone", "1 minute").await;

    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        2
    );
    let status = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select email_status from notifications where id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    for (id, want) in [
        (fresh, "queued"),
        (stale, "skipped"),
        (other_kind, "queued"),
        (unwanted, "skipped"),
        (no_email, "skipped"),
        (closed, "skipped"),
    ] {
        assert_eq!(status(id).await, want);
    }
    let mut to: Vec<(String, String)> =
        sqlx::query_as("select to_address, subject from email_outbox order by to_address")
            .fetch_all(&app.db)
            .await
            .unwrap();
    to.sort();
    assert_eq!(
        to,
        [
            ("n@example.com".to_string(), "decided".to_string()),
            ("w@example.com".into(), "fresh".into())
        ]
    );
    // Nothing is queued twice.
    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        0
    );
    assert_eq!(count(&app.db, "select count(*) from email_outbox").await, 2);
}

/// A stand-in mail server: takes mail for anyone but addresses containing "bounce", which it
/// refuses. Records (recipient, message) of what it took.
async fn fake_smtp() -> (u16, Arc<Mutex<Vec<(String, String)>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let taken: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let record = taken.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let record = record.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
                let mut rcpt = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    let upper = line.to_ascii_uppercase();
                    let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        b"250 fake\r\n"
                    } else if upper.starts_with("RCPT") {
                        rcpt = line.clone();
                        if line.contains("bounce") {
                            b"550 no such user\r\n"
                        } else {
                            b"250 ok\r\n"
                        }
                    } else if upper.starts_with("DATA") {
                        write.write_all(b"354 go on\r\n").await.unwrap();
                        let mut message = String::new();
                        while let Ok(Some(l)) = lines.next_line().await {
                            if l == "." {
                                break;
                            }
                            message.push_str(&l);
                            message.push('\n');
                        }
                        record.lock().unwrap().push((rcpt.clone(), message));
                        b"250 queued\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        b"250 ok\r\n"
                    };
                    if write.write_all(reply).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (port, taken)
}

async fn outbox_row(app: &TestApp, to: &str, attempts: i32) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into email_outbox (id, to_address, subject, body, attempts) values ($1, $2, 'Hello', 'The body.', $3)",
    )
    .bind(id)
    .bind(to)
    .bind(attempts)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// The email worker sends the outbox: sent rows are marked, failures counted and retried, and
/// given up on at the fifth; an SMTP address that isn't one switches email off.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_email_worker_sends_retries_and_gives_up(db: PgPool) {
    let (port, taken) = fake_smtp().await;
    let app = test_app_with(db.clone(), |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: format!("smtp://127.0.0.1:{port}"),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let (pat_id, _) = app.user("Pat", "US").await;
    sqlx::query("update users set email = 'pat@example.com' where id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    notice(
        &app,
        pat_id,
        "apps",
        "Your release was approved",
        "1 minute",
    )
    .await;
    let good = outbox_row(&app, "good@example.com", 0).await;
    let bounce = outbox_row(&app, "bounce@example.com", 0).await;
    let last_try = outbox_row(&app, "bounce-again@example.com", 4).await;
    let junk = outbox_row(&app, "not an address", 0).await;

    let worker = tokio::spawn(slop_api::notify::run_email(app.state.clone()));
    let mut done = false;
    for _ in 0..200 {
        let waiting = count(
            &app.db,
            "select count(*) from email_outbox where attempts = 0 or (to_address = 'bounce-again@example.com' and attempts = 4)",
        )
        .await;
        if waiting == 0 {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    worker.abort();
    assert!(done, "the worker went through the outbox");

    let row = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (i32, bool, bool, Option<String>)>(
                "select attempts, sent_at is not null, failed_at is not null, last_error from email_outbox where id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let (attempts, sent, failed, error) = row(good).await;
    assert_eq!((attempts, sent, failed, error), (1, true, false, None));
    let (attempts, sent, failed, error) = row(bounce).await;
    assert_eq!(
        (attempts, sent, failed),
        (1, false, false),
        "tried again later"
    );
    assert!(error.unwrap().contains("no such user"));
    let (attempts, sent, failed, _) = row(last_try).await;
    assert_eq!(
        (attempts, sent, failed),
        (5, false, true),
        "given up after five"
    );
    let (attempts, sent, failed, error) = row(junk).await;
    assert_eq!((attempts, sent, failed), (1, false, false));
    assert!(error.is_some());

    let taken = taken.lock().unwrap().clone();
    let to: Vec<&str> = taken.iter().map(|(r, _)| r.as_str()).collect();
    assert!(to.iter().any(|r| r.contains("good@example.com")), "{to:?}");
    assert!(
        to.iter().any(|r| r.contains("pat@example.com")),
        "the pending notification was queued and sent in the same round: {to:?}"
    );
    let (_, message) = taken.iter().find(|(r, _)| r.contains("good@")).unwrap();
    assert!(
        message.contains("Subject: Hello") && message.contains("n@slop.test"),
        "{message}"
    );

    // A broken SMTP address: the worker stops at once and sends nothing.
    let broken = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "carrier-pigeon://coop".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let before = count(
        &broken.db,
        "select count(*) from email_outbox where sent_at is null",
    )
    .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        slop_api::notify::run_email(broken.state.clone()),
    )
    .await
    .expect("returns instead of looping");
    assert_eq!(
        count(
            &broken.db,
            "select count(*) from email_outbox where sent_at is null"
        )
        .await,
        before
    );
    // Without email set up at all, likewise.
    let off = test_app(broken.db.clone()).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        slop_api::notify::run_email(off.state.clone()),
    )
    .await
    .expect("returns instead of looping");
}

/// A signed-out reporter hears the outcome by email, when email is set up and they gave an
/// address that looks like one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signed_out_reporters_get_the_outcome_by_email(db: PgPool) {
    let app = test_app_with(db.clone(), |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "miner", "released", "").await;
    let report = |name: &str, email: &str| {
        let mut r = json!({ "kind": "app", "subject": "miner", "reason": "broken", "details": "It doesn't start.", "good_faith": true });
        r["name"] = json!(name);
        r["email"] = json!(email);
        req(Method::POST, "/v1/reports").json(r)
    };
    for (name, email) in [("Sam", "sam@example.com"), ("Nobody", "no-at-sign")] {
        let res = app.send(report(name, email)).await;
        assert!(
            res.status() == StatusCode::OK || res.status() == StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            res.status()
        );
    }
    let reports: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("select id, reference, reporter_email from reports order by created_at")
            .fetch_all(&app.db)
            .await
            .unwrap();
    for (id, _, _) in &reports {
        let res = app
            .send(
                req(Method::POST, format!("/v1/admin/reports/{id}/dismiss"))
                    .cookie(&admin)
                    .json(json!({ "outcome": "It works for us." })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }
    let mail: Vec<(String, String, String)> =
        sqlx::query_as("select to_address, subject, body from email_outbox")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(mail.len(), 1, "{mail:?}");
    let (to, subject, body) = &mail[0];
    assert_eq!(to, "sam@example.com");
    assert_eq!(
        subject,
        &format!("Your report {} was answered", reports[0].1)
    );
    assert!(
        body.contains("What we decided: It works for us.")
            && body.contains("because you sent a report"),
        "{body}"
    );

    // Without SMTP, nothing is queued.
    let quiet = test_app(db).await;
    let res = quiet.send(report("Kim", "kim@example.com")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let id: Uuid =
        sqlx::query_scalar("select id from reports where reporter_email = 'kim@example.com'")
            .fetch_one(&quiet.db)
            .await
            .unwrap();
    let res = quiet
        .send(
            req(Method::POST, format!("/v1/admin/reports/{id}/dismiss"))
                .cookie(&admin)
                .json(json!({ "outcome": "Fine." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        count(&quiet.db, "select count(*) from email_outbox").await,
        1
    );
}

/// The hourly cleanup: read notifications after 180 days, any after a year, sent or failed
/// emails after 30 days; the rest stays.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn old_notifications_and_sent_emails_are_cleaned_up(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, _) = app.user("Pat", "US").await;
    let read_old = notice(&app, pat_id, "apps", "read long ago", "200 days").await;
    let read_recent = notice(&app, pat_id, "apps", "read lately", "200 days").await;
    let unread_old = notice(&app, pat_id, "apps", "unread, old", "200 days").await;
    let ancient = notice(&app, pat_id, "apps", "unread, ancient", "400 days").await;
    sqlx::query("update notifications set read_at = now() - interval '181 days' where id = $1")
        .bind(read_old)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update notifications set read_at = now() - interval '10 days' where id = $1")
        .bind(read_recent)
        .execute(&app.db)
        .await
        .unwrap();
    for (to, set) in [
        ("sent-old", "sent_at = now() - interval '31 days'"),
        ("failed-old", "failed_at = now() - interval '31 days'"),
        ("sent-new", "sent_at = now() - interval '29 days'"),
        ("waiting-old", "created_at = now() - interval '60 days'"),
    ] {
        let id = outbox_row(&app, to, 0).await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update email_outbox set {set} where id = $1"
        )))
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    slop_api::notify::cleanup(&app.db).await.unwrap();
    let left: Vec<Uuid> = sqlx::query_scalar("select id from notifications order by id")
        .fetch_all(&app.db)
        .await
        .unwrap();
    let mut want = vec![read_recent, unread_old];
    want.sort();
    assert_eq!(left, want);
    assert!(!left.contains(&ancient));
    let mut outbox: Vec<String> = sqlx::query_scalar("select to_address from email_outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();
    outbox.sort();
    assert_eq!(outbox, ["sent-new", "waiting-old"]);
}

// ---------------------------------------------------------------------------------------------
// Push (push.rs)

fn push_on(c: &mut Config) {
    let (key, _) = slop_api::push::generate();
    c.push = Some(slop_api::config::PushConfig {
        key: slop_api::push::parse_key(&key).unwrap(),
        subject: "mailto:ops@slop.example".into(),
        extra_hosts: vec!["127.0.0.1".into()],
        allow_http: false,
    });
}

async fn subscriptions(app: &TestApp) -> Vec<(Uuid, String, i32)> {
    sqlx::query_as(
        "select user_id, endpoint, failures from push_subscriptions order by created_at, endpoint",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// Without push set up, the key and subscribing answer 503; signed out, 401.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn push_needs_the_server_to_have_it_on(db: PgPool) {
    let app = test_app(db).await;
    let (_, ann) = app.user("Ann", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/push/key")).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let sub = json!({ "endpoint": "https://fcm.googleapis.com/fcm/send/abc" });
    assert_eq!(
        app.send(req(Method::POST, "/v1/me/push").json(sub.clone()))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/me/push").cookie(&ann).json(sub))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(subscriptions(&app).await.is_empty());
    assert_eq!(slop_api::push::run_once(&app.state).await.unwrap(), 0);
}

/// Subscribing: known push services over https only, ten per account (an eleventh replaces the
/// oldest; posting one already there replaces nothing), and a known endpoint posted again is
/// fresh (its failures forgotten) and goes with whoever is signed in on that browser now.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn push_subscriptions_are_limited_and_follow_the_browsers_account(db: PgPool) {
    let app = test_app_with(db, push_on).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    let subscribe = |cookie: &str, endpoint: &str| {
        req(Method::POST, "/v1/me/push")
            .cookie(cookie)
            .json(json!({ "endpoint": endpoint, "keys": { "p256dh": "x", "auth": "y" } }))
    };
    for endpoint in [
        "http://fcm.googleapis.com/fcm/send/plain",
        "https://evil.example/push",
        "https://user:pw@fcm.googleapis.com/fcm/send/x",
        "not a url",
        "http://127.0.0.1:9/push",
    ] {
        let res = app.send(subscribe(&ann, endpoint)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{endpoint}");
    }
    for endpoint in [
        "https://fcm.googleapis.com/fcm/send/a",
        "https://eu-1.notify.windows.com/w/?token=b",
        "https://web.push.apple.com/c",
    ] {
        let res = app.send(subscribe(&ann, endpoint)).await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT, "{endpoint}");
    }
    for i in 3..10 {
        let res = app
            .send(subscribe(
                &ann,
                &format!("https://fcm.googleapis.com/fcm/send/{i}"),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert_eq!(subscriptions(&app).await.len(), 10);
    // Posting one she already has, at the limit: nothing goes.
    let res = app
        .send(subscribe(&ann, "https://web.push.apple.com/c"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let subs = subscriptions(&app).await;
    assert_eq!(subs.len(), 10);
    assert!(
        subs.iter()
            .any(|s| s.1 == "https://fcm.googleapis.com/fcm/send/a"),
        "re-subscribing an existing endpoint doesn't evict the oldest"
    );
    // An eleventh: the oldest goes.
    let res = app
        .send(subscribe(&ann, "https://fcm.googleapis.com/fcm/send/new"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let subs = subscriptions(&app).await;
    assert_eq!(subs.len(), 10);
    assert!(!subs.iter().any(|s| s.1.ends_with("/send/a")));
    assert!(subs.iter().any(|s| s.1.ends_with("/send/new")));

    // Failures are forgotten when the browser subscribes again; Bob signing in on that browser
    // takes it over.
    sqlx::query("update push_subscriptions set failures = 7 where endpoint like '%/send/new'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(subscribe(&ann, "https://fcm.googleapis.com/fcm/send/new"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let subs = subscriptions(&app).await;
    let new = subs.iter().find(|s| s.1.ends_with("/send/new")).unwrap();
    assert_eq!((new.0, new.2), (ann_id, 0));
    let res = app
        .send(subscribe(&bob, "https://fcm.googleapis.com/fcm/send/new"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let subs = subscriptions(&app).await;
    assert_eq!(subs.len(), 10);
    assert_eq!(
        subs.iter()
            .filter(|s| s.0 == bob_id)
            .map(|s| s.1.as_str())
            .collect::<Vec<_>>(),
        ["https://fcm.googleapis.com/fcm/send/new"]
    );
}

/// Unsubscribing removes only the caller's own endpoint.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unsubscribing_from_push_removes_only_your_own(db: PgPool) {
    let app = test_app_with(db, push_on).await;
    let (_, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    for (cookie, endpoint) in [
        (&ann, "https://fcm.googleapis.com/fcm/send/ann"),
        (&bob, "https://fcm.googleapis.com/fcm/send/bob"),
    ] {
        let res = app
            .send(
                req(Method::POST, "/v1/me/push")
                    .cookie(cookie)
                    .json(json!({ "endpoint": endpoint })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }
    let unsubscribe =
        |endpoint: &str| req(Method::DELETE, "/v1/me/push").json(json!({ "endpoint": endpoint }));
    assert_eq!(
        app.send(unsubscribe("https://fcm.googleapis.com/fcm/send/ann"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Ann can't remove Bob's.
    assert_eq!(
        app.send(unsubscribe("https://fcm.googleapis.com/fcm/send/bob").cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(subscriptions(&app).await.len(), 2);
    assert_eq!(
        app.send(unsubscribe("https://fcm.googleapis.com/fcm/send/ann").cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let subs = subscriptions(&app).await;
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].0, bob_id);
}

/// `slop-api keys generate-vapid` prints a private key the server can read back, and the public
/// key that goes with it.
#[test]
fn the_vapid_key_command_prints_a_key_the_server_reads() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_slop-api"))
        .args(["keys", "generate-vapid"])
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let private = stdout
        .lines()
        .find_map(|l| l.strip_prefix("SLOP_VAPID_PRIVATE_KEY="))
        .expect("a SLOP_VAPID_PRIVATE_KEY= line");
    let public = stdout
        .lines()
        .find_map(|l| l.strip_prefix("public key: "))
        .expect("the public key");
    let key = slop_api::push::parse_key(private).unwrap();
    assert_eq!(slop_api::push::public_key(&key), public);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Keep SLOP_VAPID_PRIVATE_KEY secret"));
}

// ---------------------------------------------------------------------------------------------
// The front page: Featured and staff shelves (featured.rs, shelves.rs)

/// A staff member with only this role (instead of the owner role make_admin gives).
async fn staff_with(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
    let (id, cookie) = make_admin(app, name).await;
    sqlx::query("delete from staff_roles where user_id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
        .bind(id)
        .bind(role)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

async fn audits(app: &TestApp, action: &str) -> Vec<Value> {
    sqlx::query_scalar("select detail from audit_log where action = $1 order by id")
        .bind(action)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

fn slugs_of(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap().to_string())
        .collect()
}

/// Setting the carousel: Review permission, at most 8 public apps (no community branches),
/// duplicates once (and logged once), an empty list clears it; staff see every slide with
/// whether it's in the store, visitors only what's in the store for them, 8 at most.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_set_the_featured_carousel_within_its_rules(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    for i in 0..10 {
        listed_with(&app, maker_id, &format!("app-{i}"), "released", "").await;
    }
    listed_with(&app, maker_id, "grown-up", "released", "min_age = 18").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    listed_with(&app, maker_id, "branch", "released", "").await;
    sqlx::query("update apps set parent_app_id = (select id from apps where slug = 'app-0'), branch_kind = 'fix' where slug = 'branch'")
        .execute(&app.db)
        .await
        .unwrap();
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff_with(&app, "Sue Support", "support").await;
    let (_, reviewer) = staff_with(&app, "Rae Reviewer", "reviewer").await;
    let set = |cookie: &str, apps: Value| {
        req(Method::PUT, "/v1/admin/featured")
            .cookie(cookie)
            .json(json!({ "apps": apps }))
    };

    // Who may look and who may change it.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/featured").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/featured"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/featured").cookie(&support))
            .await
            .status(),
        StatusCode::OK
    );
    let one = json!([{ "slug": "app-1" }]);
    assert_eq!(
        app.send(set(&support, one.clone())).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(set(&maker, one)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(audits(&app, "featured.set").await.is_empty());

    // Too many, or apps that can't be featured.
    let nine: Vec<Value> = (0..9)
        .map(|i| json!({ "slug": format!("app-{i}") }))
        .collect();
    assert_eq!(
        app.send(set(&reviewer, json!(nine))).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for bad in ["no-such-app", "secret", "branch"] {
        let res = app
            .send(set(
                &reviewer,
                json!([{ "slug": "app-1" }, { "slug": bad }]),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        let body = body_json(res).await;
        assert!(
            body["error"]["message"].as_str().unwrap().contains(bad),
            "{body}"
        );
    }
    assert_eq!(
        count(&app.db, "select count(*) from featured_apps").await,
        0,
        "all or nothing"
    );

    // A duplicate shows once and is logged once; staff see the delisted one as not listed.
    let res = app
        .send(set(
            &reviewer,
            json!([
                { "slug": "app-2", "blurb": "  Two  " },
                { "slug": "grown-up" },
                { "slug": "app-2", "blurb": "again" },
                { "slug": "app-3" },
                { "slug": "app-4" }
            ]),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let staff_view = body_json(res).await;
    assert_eq!(
        slugs_of(&staff_view),
        ["app-2", "grown-up", "app-3", "app-4"]
    );
    assert_eq!(staff_view[0]["blurb"], "Two");
    assert_eq!(
        audits(&app, "featured.set").await,
        [json!({ "apps": ["app-2", "grown-up", "app-3", "app-4"] })]
    );
    sqlx::query("update apps set status = 'delisted' where slug = 'app-4'")
        .execute(&app.db)
        .await
        .unwrap();
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/featured").cookie(&support))
            .await,
    )
    .await;
    let flags: Vec<(String, bool)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["slug"].as_str().unwrap().to_string(),
                s["listed"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        flags,
        [
            ("app-2".to_string(), true),
            ("grown-up".into(), true),
            ("app-3".into(), true),
            ("app-4".into(), false)
        ]
    );

    // Visitors: 18+ only in an adult's view, delisted never.
    let shown = |country: &str| {
        let app = &app;
        let country = country.to_string();
        async move {
            slugs_of(
                &body_json(
                    app.send(req(Method::GET, "/v1/featured").header("x-country", country))
                        .await,
                )
                .await,
            )
        }
    };
    assert_eq!(shown("US").await, ["app-2", "grown-up", "app-3"]);
    assert_eq!(shown("GB").await, ["app-2", "app-3"]);

    // At most 8 slides show, even if more were stored.
    sqlx::query(
        "insert into featured_apps (app_id, position, blurb)
         select id, 10 + row_number() over (order by slug), '' from apps
         where slug like 'app-%' and slug not in ('app-2', 'app-3', 'app-4')",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(shown("US").await.len(), 8);

    // An empty list clears it.
    let res = app.send(set(&owner, json!([]))).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, json!([]));
    assert_eq!(shown("US").await, Vec::<String>::new());
    assert_eq!(
        audits(&app, "featured.set").await.last().unwrap(),
        &json!({ "apps": [] })
    );
}

/// Creating shelves: Review permission, a unique address, at most 20 shelves of at most 24 public
/// apps, one-line titles; each one logged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_shelves_are_checked_when_made(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    for i in 0..25 {
        listed_with(&app, maker_id, &format!("app-{i}"), "released", "").await;
    }
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff_with(&app, "Sue Support", "support").await;
    let create = |cookie: &str, body: Value| {
        req(Method::POST, "/v1/admin/shelves")
            .cookie(cookie)
            .json(body)
    };
    let shelf = |slug: &str| json!({ "slug": slug, "title": "Cozy games", "blurb": "Warm.", "apps": ["app-1", "app-2"] });

    assert_eq!(
        app.send(create(&maker, shelf("cozy"))).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(create(&support, shelf("cozy"))).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/shelves").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(create(&owner, shelf("cozy"))).await;
    assert_eq!(res.status(), StatusCode::OK);
    let listed = body_json(res).await;
    assert_eq!(listed["shelves"][0]["slug"], "cozy");
    assert_eq!(
        audits(&app, "shelf.created").await,
        [json!({ "title": "Cozy games", "on_front": true, "apps": ["app-1", "app-2"] })]
    );

    let mut bad = vec![
        ("taken address", shelf("cozy"), "slug"),
        ("no address", json!({ "title": "x", "apps": [] }), "slug"),
        (
            "newline in the title",
            json!({ "slug": "nl-1", "title": "Two\nlines", "apps": [] }),
            "title",
        ),
        (
            "newline in the blurb",
            json!({ "slug": "nl-2", "title": "One", "blurb": "a\nb", "apps": [] }),
            "title",
        ),
        (
            "unknown app",
            json!({ "slug": "u-1", "title": "One", "apps": ["app-1", "nope"] }),
            "apps",
        ),
        (
            "private app",
            json!({ "slug": "u-2", "title": "One", "apps": ["secret"] }),
            "apps",
        ),
        (
            "long title",
            json!({ "slug": "u-3", "title": "t".repeat(61), "apps": [] }),
            "title",
        ),
    ];
    let many: Vec<String> = (0..25).map(|i| format!("app-{i}")).collect();
    bad.push((
        "25 apps",
        json!({ "slug": "big", "title": "Big", "apps": many }),
        "apps",
    ));
    for (why, body, field) in bad {
        let res = app.send(create(&owner, body)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{why}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{why}");
    }
    assert_eq!(
        count(&app.db, "select count(*) from store_shelves").await,
        1,
        "nothing half-made"
    );
    assert_eq!(audits(&app, "shelf.created").await.len(), 1);

    // A duplicated app is on the shelf (and in the log) once.
    let res = app
        .send(create(
            &owner,
            json!({ "slug": "dupes", "title": "Dupes", "apps": ["app-3", "app-3", "app-4"] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(app.send(req(Method::GET, "/v1/shelves/staff/dupes")).await).await;
    assert_eq!(slugs_of(&page["apps"]), ["app-3", "app-4"]);
    assert_eq!(
        audits(&app, "shelf.created").await[1]["apps"],
        json!(["app-3", "app-4"])
    );

    // Twenty at most.
    for i in 2..20 {
        let res = app.send(create(&owner, shelf(&format!("shelf-{i}")))).await;
        assert_eq!(res.status(), StatusCode::OK, "{i}");
    }
    let res = app.send(create(&owner, shelf("one-too-many"))).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("at most 20")
    );
    assert_eq!(
        count(&app.db, "select count(*) from store_shelves").await,
        20
    );
}

/// Two staff making the 20th shelf at the same moment don't make 21.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn racing_shelf_creations_stay_within_the_limit(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    for i in 0..19 {
        sqlx::query(
            "insert into store_shelves (id, slug, title, position) values ($1, $2, 'x', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(format!("shelf-{i}"))
        .bind(i)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let create = |slug: &str| {
        let r = req(Method::POST, "/v1/admin/shelves")
            .cookie(&owner)
            .json(json!({ "slug": slug, "title": "Racing", "apps": [] }));
        let app = &app;
        async move { app.send(r).await }
    };
    let racers: Vec<_> = (0..8)
        .map(|i| create(&format!("race-{i}")))
        .collect::<Vec<_>>();
    let ok = futures_util::future::join_all(racers)
        .await
        .iter()
        .filter(|r| r.status() == StatusCode::OK)
        .count();
    assert_eq!(ok, 1);
    assert_eq!(
        count(&app.db, "select count(*) from store_shelves").await,
        20
    );
}

/// Editing, removing, ordering and hiding: Review permission, unknown shelves 404, the address
/// can't change, the order names every shelf once; each change logged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_edit_order_remove_and_hide_shelves(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    for slug in ["tea-shop", "cozy-farm", "dark-dungeon"] {
        listed_with(&app, maker_id, slug, "released", "").await;
    }
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff_with(&app, "Sue Support", "support").await;
    for (slug, apps) in [
        ("cozy", json!(["tea-shop"])),
        ("dark", json!(["dark-dungeon"])),
        ("farm", json!(["cozy-farm"])),
    ] {
        let res = app
            .send(
                req(Method::POST, "/v1/admin/shelves")
                    .cookie(&owner)
                    .json(json!({ "slug": slug, "title": slug, "apps": apps })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    let front = || async {
        slugs_of(&body_json(app.send(req(Method::GET, "/v1/shelves")).await).await["shelves"])
    };
    assert_eq!(front().await, ["cozy", "dark", "farm"]);

    // Edit.
    let edit = |cookie: &str, slug: &str, body: Value| {
        req(Method::PUT, format!("/v1/admin/shelves/{slug}"))
            .cookie(cookie)
            .json(body)
    };
    let body = json!({ "title": "Cozier", "apps": ["cozy-farm", "tea-shop"] });
    assert_eq!(
        app.send(edit(&maker, "cozy", body.clone())).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(edit(&support, "cozy", body.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(edit(&owner, "nowhere", body.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(edit(
            &owner,
            "cozy",
            json!({ "slug": "cozier", "title": "Cozier", "apps": [] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "slug");
    let res = app
        .send(edit(
            &owner,
            "cozy",
            json!({ "slug": "cozy", "title": "Cozier", "apps": ["cozy-farm", "tea-shop"] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(app.send(req(Method::GET, "/v1/shelves/staff/cozy")).await).await;
    assert_eq!(page["title"], "Cozier");
    assert_eq!(slugs_of(&page["apps"]), ["cozy-farm", "tea-shop"]);
    assert_eq!(
        audits(&app, "shelf.changed").await,
        [json!({ "title": "Cozier", "on_front": true, "apps": ["cozy-farm", "tea-shop"] })]
    );

    // Order: every shelf, once each.
    let order = |cookie: &str, slugs: Value| {
        req(Method::PUT, "/v1/admin/shelves/order")
            .cookie(cookie)
            .json(json!({ "slugs": slugs }))
    };
    assert_eq!(
        app.send(order(&maker, json!(["farm", "dark", "cozy"])))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(order(&support, json!(["farm", "dark", "cozy"])))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for bad in [
        json!(["farm"]),
        json!(["farm", "dark", "cozy", "nowhere"]),
        json!(["farm", "farm", "dark"]),
        json!(["farm", "farm", "dark", "cozy"]),
    ] {
        let res = app.send(order(&owner, bad.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    assert_eq!(
        front().await,
        ["cozy", "dark", "farm"],
        "a refused order changes nothing"
    );
    assert!(audits(&app, "shelf.reordered").await.is_empty());
    let res = app
        .send(order(&owner, json!(["farm", "dark", "cozy"])))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        slugs_of(&body_json(res).await["shelves"]),
        ["farm", "dark", "cozy"]
    );
    assert_eq!(front().await, ["farm", "dark", "cozy"]);
    assert_eq!(
        audits(&app, "shelf.reordered").await,
        [json!({ "order": ["farm", "dark", "cozy"] })]
    );

    // Remove.
    let remove = |cookie: &str, slug: &str| {
        req(Method::DELETE, format!("/v1/admin/shelves/{slug}")).cookie(cookie)
    };
    assert_eq!(
        app.send(remove(&maker, "dark")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&support, "dark")).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(remove(&owner, "nowhere")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(audits(&app, "shelf.removed").await.is_empty());
    assert_eq!(
        app.send(remove(&owner, "dark")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(front().await, ["farm", "cozy"]);
    let removed: Vec<String> =
        sqlx::query_scalar("select subject from audit_log where action = 'shelf.removed'")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(removed, ["shelf dark"]);

    // Hide and show an automatic shelf.
    let hide = |cookie: &str, key: &str, hidden: bool| {
        req(Method::POST, format!("/v1/admin/front-shelves/{key}"))
            .cookie(cookie)
            .json(json!({ "hidden": hidden }))
    };
    assert_eq!(
        app.send(hide(&maker, "popular", true)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(hide(&support, "popular", true)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(hide(&owner, "not_a_shelf", true)).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(hide(&owner, "popular", true)).await;
    let listed = body_json(res).await;
    let popular = listed["built_in"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["key"] == "popular")
        .unwrap();
    assert_eq!(popular["hidden"], true);
    assert_eq!(listed["built_in"].as_array().unwrap().len(), 9);
    let hidden = || async {
        body_json(app.send(req(Method::GET, "/v1/shelves")).await).await["hidden"].clone()
    };
    assert_eq!(hidden().await, json!(["popular"]));
    app.send(hide(&owner, "popular", true)).await;
    assert_eq!(
        hidden().await,
        json!(["popular"]),
        "hiding twice is the same"
    );
    assert_eq!(
        app.send(hide(&owner, "popular", false)).await.status(),
        StatusCode::OK
    );
    assert_eq!(hidden().await, json!([]));
    let actions: Vec<String> = sqlx::query_scalar(
        "select action from audit_log where subject = 'shelf popular' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "shelf.built_in_hidden",
            "shelf.built_in_hidden",
            "shelf.built_in_shown"
        ]
    );

    // The staff list: every shelf with its apps and whether each is in the store.
    sqlx::query("update apps set status = 'delisted' where slug = 'tea-shop'")
        .execute(&app.db)
        .await
        .unwrap();
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/shelves").cookie(&support))
            .await,
    )
    .await;
    assert_eq!(slugs_of(&listed["shelves"]), ["farm", "cozy"]);
    let cozy = &listed["shelves"][1];
    assert_eq!(cozy["on_front"], true);
    assert_eq!(
        cozy["apps"],
        json!([
            { "slug": "cozy-farm", "title": "cozy-farm", "listed": true },
            { "slug": "tea-shop", "title": "tea-shop", "listed": false }
        ])
    );
}

/// A shelf's own page shows only apps in the store for the viewer; an unknown shelf is 404.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_shelf_page_shows_only_what_the_viewer_may_see(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "for-all", "released", "").await;
    listed_with(&app, maker_id, "grown-up", "released", "min_age = 18").await;
    listed_with(&app, maker_id, "gone", "released", "").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let res = app
        .send(req(Method::POST, "/v1/admin/shelves").cookie(&owner).json(json!({
            "slug": "mixed", "title": "Mixed", "on_front": false, "apps": ["grown-up", "gone", "for-all"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    sqlx::query("update apps set status = 'delisted' where slug = 'gone'")
        .execute(&app.db)
        .await
        .unwrap();
    let page = |country: &str| {
        req(Method::GET, "/v1/shelves/staff/mixed").header("x-country", country.to_string())
    };
    let adult = body_json(app.send(page("US")).await).await;
    assert_eq!(slugs_of(&adult["apps"]), ["grown-up", "for-all"]);
    assert_eq!(adult["title"], "Mixed");
    let young = body_json(app.send(page("GB")).await).await;
    assert_eq!(slugs_of(&young["apps"]), ["for-all"]);
    assert_eq!(
        app.send(req(Method::GET, "/v1/shelves/staff/nowhere"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

/// Staff agents change the front page through MCP with their token's rights: a read token only
/// looks; a review token sets Featured and edits shelves; errors come back as tool errors.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_agents_edit_the_front_page_with_their_tokens_rights(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    for slug in ["tea-shop", "cozy-farm"] {
        listed_with(&app, maker_id, slug, "released", "").await;
    }
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let token =
        |scope: &str| {
            let app = &app;
            let owner = owner.clone();
            let scope = scope.to_string();
            async move {
                let res = app
                    .send(req(Method::POST, "/v1/admin/tokens").cookie(&owner).json(
                        json!({ "name": format!("{scope} agent"), "scope": scope, "days": 7 }),
                    ))
                    .await;
                assert_eq!(res.status(), StatusCode::CREATED);
                format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
            }
        };
    let read = token("read").await;
    let review = token("review").await;
    let call = |bearer: &str, name: &str, args: Value| {
        let app = &app;
        let bearer = bearer.to_string();
        let name = name.to_string();
        async move {
            let res = app
                .send(
                    req(Method::POST, "/v1/admin/mcp")
                        .header("authorization", bearer)
                        .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })),
                )
                .await;
            assert_eq!(res.status(), StatusCode::OK);
            let out = body_json(res).await;
            (
                out["result"]["isError"] == false,
                out["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };

    // A read token looks but changes nothing.
    assert!(call(&read, "featured_apps", json!({})).await.0);
    assert!(call(&read, "store_shelves", json!({})).await.0);
    for (name, args) in [
        (
            "set_featured_apps",
            json!({ "apps": [{ "slug": "tea-shop" }] }),
        ),
        (
            "create_shelf",
            json!({ "slug": "cozy", "title": "Cozy", "apps": ["tea-shop"] }),
        ),
        (
            "hide_front_shelf",
            json!({ "key": "popular", "hidden": true }),
        ),
        ("order_shelves", json!({ "slugs": [] })),
    ] {
        let (ok, _) = call(&read, name, args).await;
        assert!(!ok, "{name}");
    }
    assert_eq!(
        count(&app.db, "select count(*) from featured_apps").await,
        0
    );
    assert_eq!(
        count(&app.db, "select count(*) from store_shelves").await,
        0
    );
    assert_eq!(
        count(&app.db, "select count(*) from hidden_front_shelves").await,
        0
    );

    // A review token does the work, logged under the agent's name.
    let (ok, text) = call(
        &review,
        "set_featured_apps",
        json!({ "apps": [{ "slug": "cozy-farm", "blurb": "Calm" }] }),
    )
    .await;
    assert!(ok, "{text}");
    assert!(
        call(&review, "featured_apps", json!({}))
            .await
            .1
            .contains("cozy-farm")
    );
    assert!(
        call(
            &review,
            "create_shelf",
            json!({ "slug": "cozy", "title": "Cozy", "apps": ["tea-shop"] })
        )
        .await
        .0
    );
    assert!(
        call(
            &review,
            "create_shelf",
            json!({ "slug": "farm", "title": "Farm", "apps": ["cozy-farm"] })
        )
        .await
        .0
    );
    assert!(
        call(
            &review,
            "update_shelf",
            json!({ "slug": "cozy", "title": "Cozier", "apps": ["tea-shop", "cozy-farm"] })
        )
        .await
        .0
    );
    assert!(
        call(
            &review,
            "order_shelves",
            json!({ "slugs": ["farm", "cozy"] })
        )
        .await
        .0
    );
    assert!(
        call(
            &review,
            "hide_front_shelf",
            json!({ "key": "fresh_slop", "hidden": true })
        )
        .await
        .0
    );
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(slugs_of(&front["shelves"]), ["farm", "cozy"]);
    assert_eq!(front["shelves"][1]["title"], "Cozier");
    assert_eq!(front["hidden"], json!(["fresh_slop"]));
    assert!(
        call(&review, "delete_shelf", json!({ "slug": "farm" }))
            .await
            .0
    );
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'shelf.removed'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("review agent"), "{actor}");

    // Refusals come back as tool errors with the reason.
    let (ok, text) = call(
        &review,
        "set_featured_apps",
        json!({ "apps": [{ "slug": "nowhere" }] }),
    )
    .await;
    assert!(!ok);
    assert!(text.contains("nowhere"), "{text}");
    let (ok, _) = call(&review, "delete_shelf", json!({ "slug": "farm" })).await;
    assert!(!ok, "already gone");
}

/// Two makers starting demos at the same moment for the last slot: only one gets it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn racing_demo_starts_stay_within_the_live_limit(db: PgPool) {
    let s = demo_setup(db, 1).await;
    let app = &s.app;
    for i in 0..4 {
        listed_with(app, s.maker_id, &format!("game-{i}"), "released", "").await;
    }
    sqlx::query("update apps set stage_entered_at = now(), repo_url = $1 where slug like 'game-%'")
        .bind(format!("{}octo/game", s.host))
        .execute(&app.db)
        .await
        .unwrap();
    let starts: Vec<_> = ["game", "game-0", "game-1", "game-2", "game-3"]
        .iter()
        .map(|slug| {
            let r = req(Method::POST, format!("/v1/apps/{slug}/demo"))
                .cookie(&s.maker)
                .json(json!({ "url": s.file("web.zip") }));
            async move { app.send(r).await.status() }
        })
        .collect();
    let statuses = futures_util::future::join_all(starts).await;
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::ACCEPTED)
            .count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(count(&app.db, "select count(*) from demos").await, 1);
}

// ---------------------------------------------------------------------------------------------
// Playtime sharing (pulse.rs)

/// With "Share my playtime" off, a launch gets no Pulse token and records no playtime, but it
/// still counts as a launch: the player can vote (weighted by launch days) and review.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_who_dont_share_playtime_can_still_vote_and_review(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&pat)
                .json(json!({ "share_playtime": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let launched = body_json(
        app.send(req(Method::POST, "/v1/apps/tower/launch?platform=linux").cookie(&pat))
            .await,
    )
    .await;
    assert!(launched["token"].is_null());
    assert_eq!(
        count(&app.db, "select count(*) from pulse_sessions").await,
        0
    );
    let usage: (i64, i32, Option<String>) = sqlx::query_as(
        "select playtime_seconds, days_launched, last_platform from user_app_usage where user_id = $1",
    )
    .bind(pat_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(usage, (0, 1, Some("linux".to_string())));

    let res = app
        .send(
            req(Method::POST, "/v1/apps/tower/vote")
                .cookie(&pat)
                .json(json!({ "vote": "not_slop" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let weight: f32 = sqlx::query_scalar("select weight from votes where user_id = $1")
        .bind(pat_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(weight, slop_api::votes::vote_weight(0, 1));
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/tower/review")
                .cookie(&pat)
                .json(json!({ "body": "Solid little game." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------------------------
// Reviews (reviews.rs)

/// A player who launched `slug`, voted and wrote a review.
async fn reviewer(app: &TestApp, slug: &str, name: &str, body: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    app.send(req(Method::POST, format!("/v1/apps/{slug}/launch")).cookie(&cookie))
        .await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/apps/{slug}/vote"))
                .cookie(&cookie)
                .json(json!({ "vote": "not_slop" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::PUT, format!("/v1/apps/{slug}/review"))
                .cookie(&cookie)
                .json(json!({ "body": body })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    (id, cookie)
}

/// Deleting your review takes it off the app's page and your profile; your vote stays, and
/// nobody else's review is touched.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_delete_their_own_review_only(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (pat_id, pat) = reviewer(&app, "tower", "Pat", "Great towers.").await;
    let (_, _sam) = reviewer(&app, "tower", "Sam", "Fun for a while.").await;
    let reviews = || async {
        body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["body"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(reviews().await.len(), 2);

    assert_eq!(
        app.send(req(Method::DELETE, "/v1/apps/tower/review"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/apps/no-such-app/review").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(reviews().await.len(), 2);
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/apps/tower/review").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(reviews().await, ["Fun for a while."]);
    let profile = body_json(
        app.send(req(Method::GET, format!("/v1/users/{pat_id}")).cookie(&pat))
            .await,
    )
    .await;
    let votes = profile["votes"].as_array().unwrap();
    assert_eq!(votes.len(), 1, "the vote stays: {profile}");
    assert!(votes[0]["review"].is_null());
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/ratings").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(ratings["mine"]["vote"], "not_slop", "{ratings}");
    assert!(ratings["mine"]["review"].is_null(), "{ratings}");
    // Again: nothing to do.
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/apps/tower/review").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

/// A maker takes their reply back; there must be one, only the maker can, and a reply staff
/// removed can't be taken back (or written again).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_remove_their_reply_to_a_review(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    reviewer(&app, "tower", "Pat", "Great towers.").await;
    reviewer(&app, "tower", "Sam", "Too short.").await;
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    let id_of = |who: &str| {
        page["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["body"] == who)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (pat_review, sam_review) = (id_of("Great towers."), id_of("Too short."));
    let reply_url = |id: &str| format!("/v1/apps/tower/reviews/{id}/reply");
    for id in [&pat_review, &sam_review] {
        let res = app
            .send(
                req(Method::PUT, reply_url(id))
                    .cookie(&maker)
                    .json(json!({ "body": "Thanks!" })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(req(Method::DELETE, reply_url(&pat_review)).cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::DELETE, reply_url(&Uuid::now_v7().to_string())).cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::DELETE, reply_url(&pat_review)).cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let replies: Vec<(String, Option<String>)> =
        sqlx::query_as("select body, reply from app_reviews order by body")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        replies,
        [
            ("Great towers.".to_string(), None),
            ("Too short.".into(), Some("Thanks!".into()))
        ]
    );
    // No reply left to remove.
    assert_eq!(
        app.send(req(Method::DELETE, reply_url(&pat_review)).cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Staff removed Sam's reply: the maker can neither take it back nor write it again.
    sqlx::query("update app_reviews set reply_hidden_at = now() where body = 'Too short.'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::DELETE, reply_url(&sam_review)).cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(
            req(Method::PUT, reply_url(&sam_review))
                .cookie(&maker)
                .json(json!({ "body": "Again" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

/// Makers' agents and reviews: a read token lists them but can't reply; a token for other apps
/// can't touch this one's; a publish token replies, and errors are tool errors.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_agents_list_and_answer_reviews_within_their_token(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let maker_id: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    listed_with(&app, maker_id, "other", "released", "").await;
    let (pat_id, _) = reviewer(&app, "game", "Pat", "Needs a save button.").await;
    let review: String = sqlx::query_scalar("select id::text from app_reviews")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let read = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let publish = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let other_only = maker_token(
        &app,
        &maker,
        json!({ "name": "Other", "scope": "publish", "apps": ["other"] }),
    )
    .await;
    let call = |token: &str, name: &str, args: Value| {
        let app = &app;
        let r = req(Method::POST, "/v1/maker/mcp")
            .header("authorization", token.to_string())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }));
        async move {
            let out = body_json(app.send(r).await).await;
            (
                out["result"]["isError"] == false,
                out["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };
    let (ok, text) = call(&read, "list_reviews", json!({ "slug": "game" })).await;
    assert!(ok && text.contains("Needs a save button."), "{text}");
    let reply = json!({ "slug": "game", "id": review, "body": "Added in 1.1!" });
    let (ok, _) = call(&read, "reply_to_review", reply.clone()).await;
    assert!(!ok, "a read token can't reply");
    let (ok, _) = call(&other_only, "reply_to_review", reply.clone()).await;
    assert!(!ok, "not one of the token's apps");
    let (ok, _) = call(&other_only, "list_reviews", json!({ "slug": "game" })).await;
    assert!(!ok);
    let replied: Option<String> = sqlx::query_scalar("select reply from app_reviews")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(replied.is_none());

    let (ok, text) = call(&publish, "reply_to_review", reply).await;
    assert!(ok, "{text}");
    let replied: Option<String> = sqlx::query_scalar("select reply from app_reviews")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(replied.as_deref(), Some("Added in 1.1!"));
    assert_eq!(
        count(
            &app.db,
            &format!("select count(*) from notifications where user_id = '{pat_id}'")
        )
        .await,
        1
    );
    let (ok, text) = call(
        &publish,
        "reply_to_review",
        json!({ "slug": "game", "id": review, "body": "" }),
    )
    .await;
    assert!(!ok && text.contains("2,000"), "{text}");
    // The agent can take the reply back through the API too, but a read token can't.
    let path = format!("/v1/apps/game/reviews/{review}/reply");
    assert_eq!(
        app.send(req(Method::DELETE, path.clone()).header("authorization", read.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(req(Method::DELETE, path).header("authorization", publish.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

// ---------------------------------------------------------------------------------------------
// Ratings (votes.rs)

/// The maker's breakdown groups each player's current vote by the version it was cast on, and
/// shows per release how many changed their minds or voted for the first time; withdrawn votes
/// don't count.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_rating_breakdown_follows_votes_across_releases(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let release = |version: &str| {
        let db = app.db.clone();
        let version = version.to_string();
        async move {
            sqlx::query(
                "insert into releases (id, app_id, version, commit_sha, status, submitted_by, signed_manifest, reviewed_at)
                 select $1, id, $2, repeat('a', 40), 'approved', $3, '{}'::jsonb, now() from apps where slug = 'tower'",
            )
            .bind(Uuid::now_v7())
            .bind(version)
            .bind(maker_id)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    let mut players = Vec::new();
    for i in 0..4 {
        let (_, cookie) = app.user(&format!("Player {i}"), "US").await;
        app.send(req(Method::POST, "/v1/apps/tower/launch?platform=windows").cookie(&cookie))
            .await;
        players.push(cookie);
    }
    let vote = |cookie: &str, vote: &str| {
        req(Method::POST, "/v1/apps/tower/vote")
            .cookie(cookie)
            .json(json!({ "vote": vote }))
    };
    release("1.0.0").await;
    for (p, v) in [(0, "slop"), (1, "slop"), (2, "not_slop"), (3, "slop")] {
        assert_eq!(
            app.send(vote(&players[p], v)).await.status(),
            StatusCode::OK
        );
    }
    // 1.1.0 wins two over, one more changes their mind back, one withdraws.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    release("1.1.0").await;
    for (p, v) in [(0, "not_slop"), (1, "not_slop"), (2, "slop")] {
        assert_eq!(
            app.send(vote(&players[p], v)).await.status(),
            StatusCode::OK
        );
    }
    let withdraw = req(Method::POST, "/v1/apps/tower/vote")
        .cookie(&players[3])
        .json(json!({ "vote": null }));
    assert_eq!(app.send(withdraw).await.status(), StatusCode::OK);

    let b = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/ratings/breakdown").cookie(&maker))
            .await,
    )
    .await;
    let by_version = b["by_version"].as_array().unwrap();
    assert_eq!(
        by_version.len(),
        1,
        "only current votes, each on the version it was cast: {b}"
    );
    assert_eq!(by_version[0]["key"], "1.1.0");
    assert_eq!(
        (
            by_version[0]["not_slop"].as_i64(),
            by_version[0]["slop"].as_i64()
        ),
        (Some(2), Some(1))
    );
    let impact: Vec<(String, i64, i64, i64)> = b["impact"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["version"].as_str().unwrap().to_string(),
                i["to_not_slop"].as_i64().unwrap(),
                i["to_slop"].as_i64().unwrap(),
                i["new_votes"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        impact,
        [
            ("1.1.0".to_string(), 2, 1, 0),
            ("1.0.0".to_string(), 0, 0, 4)
        ]
    );
    assert_eq!(b["by_platform"][0]["key"], "windows");
    assert_eq!(
        b["by_platform"][0]["not_slop"].as_i64().unwrap()
            + b["by_platform"][0]["slop"].as_i64().unwrap(),
        3
    );
    // Signed out: 401.
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/tower/ratings/breakdown"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------------------------
// Library, Interested and upcoming (library.rs, interested.rs)

/// Puts `slug` in a user's library or Interested list straight away, `age` ago.
async fn mark(app: &TestApp, table: &str, user: Uuid, slug: &str, age: &str) {
    let column = if table == "library_entries" {
        "added_at"
    } else {
        "created_at"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into {table} (user_id, app_id, {column}) select $1, id, now() - interval '{age}' from apps where slug = $2"
    )))
    .bind(user)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// The library lists newest first and keeps apps that left the store; removing touches only the
/// caller's own entry and is a no-op for anything else.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_library_keeps_apps_that_left_the_store(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "old-favourite", "released", "").await;
    listed_with(
        &app,
        maker_id,
        "delisted",
        "released",
        "status = 'delisted'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "past-deadline",
        "coming_soon",
        "stage_entered_at = now() - interval '400 days'",
    )
    .await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    mark(&app, "library_entries", pat_id, "old-favourite", "3 days").await;
    mark(&app, "library_entries", pat_id, "delisted", "2 days").await;
    mark(&app, "library_entries", pat_id, "past-deadline", "1 day").await;
    mark(&app, "library_entries", sam_id, "old-favourite", "1 day").await;

    assert_eq!(
        app.send(req(Method::GET, "/v1/me/library")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let library = inbox(&app, req(Method::GET, "/v1/me/library").cookie(&pat)).await;
    assert_eq!(
        slugs_of(&library),
        ["past-deadline", "delisted", "old-favourite"]
    );
    assert_eq!(
        library[1]["status"], "delisted",
        "the tile can say it left the store"
    );

    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/library/old-favourite"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/library/no-such-app").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/library/old-favourite").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let library = inbox(&app, req(Method::GET, "/v1/me/library").cookie(&pat)).await;
    assert_eq!(slugs_of(&library), ["past-deadline", "delisted"]);
    let theirs = inbox(&app, req(Method::GET, "/v1/me/library").cookie(&sam)).await;
    assert_eq!(slugs_of(&theirs), ["old-favourite"]);
}

/// Interested lists newest first, keeps apps that were released or left; removing drops it
/// from the list and from the public count.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_interested_list_keeps_apps_that_moved_on(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "out-now", "released", "").await;
    listed_with(&app, maker_id, "gone", "beta", "status = 'delisted'").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (sam_id, sam) = app.user("Sam", "US").await;
    mark(&app, "interests", pat_id, "out-now", "3 days").await;
    mark(&app, "interests", pat_id, "gone", "2 days").await;
    mark(&app, "interests", pat_id, "soon", "1 day").await;
    mark(&app, "interests", sam_id, "soon", "1 day").await;

    for (m, path) in [
        (Method::GET, "/v1/me/interested"),
        (Method::DELETE, "/v1/me/interested/soon"),
    ] {
        assert_eq!(
            app.send(req(m, path)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let list = inbox(&app, req(Method::GET, "/v1/me/interested").cookie(&pat)).await;
    assert_eq!(slugs_of(&list), ["soon", "gone", "out-now"]);
    assert!(list[0]["interested_at"].is_string());
    let page = inbox(&app, req(Method::GET, "/v1/apps/soon").cookie(&pat)).await;
    assert_eq!(
        (page["interested"].as_i64(), page["is_interested"].as_bool()),
        (Some(2), Some(true))
    );

    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/interested/no-such-app").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/interested/soon").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let list = inbox(&app, req(Method::GET, "/v1/me/interested").cookie(&pat)).await;
    assert_eq!(slugs_of(&list), ["gone", "out-now"]);
    let page = inbox(&app, req(Method::GET, "/v1/apps/soon").cookie(&pat)).await;
    assert_eq!(
        (page["interested"].as_i64(), page["is_interested"].as_bool()),
        (Some(1), Some(false))
    );
    let theirs = inbox(&app, req(Method::GET, "/v1/me/interested").cookie(&sam)).await;
    assert_eq!(slugs_of(&theirs), ["soon"]);
}

/// Moving on: Interested players hear when an app is released; moving back to Coming soon tells
/// no one, and closed accounts hear nothing.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn interested_players_hear_the_app_is_out(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "soon", "beta", "title = 'Soon Game'").await;
    approved_release(&app, "soon", maker_id).await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (gone_id, _) = app.user("Gone", "US").await;
    mark(&app, "interests", pat_id, "soon", "1 day").await;
    mark(&app, "interests", gone_id, "soon", "1 day").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(gone_id)
        .execute(&app.db)
        .await
        .unwrap();

    admin::set_stage(
        &app.db,
        "soon",
        slop_api::catalog::AppStage::ComingSoon,
        "test",
    )
    .await
    .unwrap();
    assert_eq!(
        count(&app.db, "select count(*) from notifications").await,
        0
    );
    admin::set_stage(
        &app.db,
        "soon",
        slop_api::catalog::AppStage::Released,
        "test",
    )
    .await
    .unwrap();
    let mine = inbox(&app, req(Method::GET, "/v1/me/notifications").cookie(&pat)).await;
    assert_eq!(titles(&mine), ["Soon Game is out now"]);
    assert_eq!(mine["items"][0]["link"], "/app/soon");
    assert_eq!(mine["items"][0]["category"], "interested");
    assert_eq!(
        count(&app.db, "select count(*) from notifications").await,
        1,
        "not for a closed account"
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'app.stage' order by id desc limit 1",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail["interested_told"], 2, "{detail}");
}

/// "Coming soon & in beta, for you": signed out or without personal recommendations, the most
/// anticipated (12 at most, never released or dead ones, by the viewer's age policy); with them,
/// matched to the viewer's tags, leaving out what they have, marked or made.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn upcoming_for_you_matches_taste_or_falls_back_to_most_anticipated(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (other_maker, _) = app.user("Other Maker", "US").await;
    for i in 0..14 {
        listed_with(&app, other_maker, &format!("up-{i:02}"), "beta", "").await;
    }
    listed_with(&app, other_maker, "racer", "coming_soon", "").await;
    listed_with(&app, other_maker, "grown-up", "coming_soon", "min_age = 18").await;
    listed_with(
        &app,
        other_maker,
        "dead",
        "beta",
        "last_commit_at = now() - interval '200 days'",
    )
    .await;
    listed_with(&app, other_maker, "out", "released", "").await;
    listed_with(&app, maker_id, "my-own", "coming_soon", "").await;
    listed_with(&app, other_maker, "liked", "released", "").await;
    tag_app(&app, "racer", &["racing"]).await;
    tag_app(&app, "liked", &["racing"]).await;
    tag_app(&app, "up-00", &["racing"]).await;
    // Demand: grown-up and dead most, then up-00 .. up-13 in order, racer least.
    let fans: Vec<Uuid> = {
        let mut v = Vec::new();
        for i in 0..20 {
            v.push(app.user(&format!("Fan {i}"), "US").await.0);
        }
        v
    };
    for (slug, n) in [("grown-up", 20), ("dead", 20), ("out", 20), ("my-own", 19)] {
        for f in &fans[..n] {
            mark(&app, "interests", *f, slug, "1 day").await;
        }
    }
    for i in 0..14usize {
        for f in &fans[..(16 - i)] {
            mark(&app, "interests", *f, &format!("up-{i:02}"), "1 day").await;
        }
    }

    let signed_out = |country: &str| {
        let app = &app;
        let country = country.to_string();
        async move {
            inbox(
                app,
                req(Method::GET, "/v1/upcoming/for-you").header("x-country", country),
            )
            .await
        }
    };
    let us = signed_out("US").await;
    let slugs = slugs_of(&us);
    assert_eq!(slugs.len(), 12);
    assert_eq!(&slugs[..3], ["grown-up", "my-own", "up-00"]);
    assert!(
        !slugs.iter().any(|s| s == "dead" || s == "out"),
        "{slugs:?}"
    );
    assert_eq!(us[0]["reason"], "20 interested");
    let gb = slugs_of(&signed_out("GB").await);
    assert_eq!(&gb[..2], ["my-own", "up-00"], "no 18+ in the under-18 view");

    // Personal: Pat likes racing (a racing game in the library) and is Interested in up-00.
    let (pat_id, pat) = app.user("Pat", "US").await;
    mark(&app, "library_entries", pat_id, "liked", "1 day").await;
    mark(&app, "interests", pat_id, "up-01", "1 day").await;
    let mine = inbox(&app, req(Method::GET, "/v1/upcoming/for-you").cookie(&pat)).await;
    let slugs = slugs_of(&mine);
    assert_eq!(
        slugs[0], "up-00",
        "a racing game with demand first: {slugs:?}"
    );
    assert_eq!(mine[0]["reason"], "Because you like Racing");
    assert!(slugs.contains(&"racer".to_string()));
    assert!(!slugs.contains(&"up-01".to_string()), "already marked");
    assert!(!slugs.iter().any(|s| s == "dead" || s == "out"));
    assert!(slugs.len() <= 12);

    // The maker never sees their own app suggested, personal or not.
    let theirs = slugs_of(
        &inbox(
            &app,
            req(Method::GET, "/v1/upcoming/for-you").cookie(&maker),
        )
        .await,
    );
    assert!(!theirs.contains(&"my-own".to_string()), "{theirs:?}");
    sqlx::query("update users set personalize = false where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let theirs = inbox(
        &app,
        req(Method::GET, "/v1/upcoming/for-you").cookie(&maker),
    )
    .await;
    assert!(
        !slugs_of(&theirs).contains(&"my-own".to_string()),
        "{theirs}"
    );
    assert_eq!(theirs[0]["reason"], "20 interested", "not personal");
}

/// Without personal recommendations, the upcoming list still leaves out what the viewer
/// already marked Interested or has in their library.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "b15: the non-personal upcoming list shows apps the viewer already marked; needs a decision on whether opted-out viewers' own marks may filter it"]
async fn upcoming_without_personalization_leaves_out_what_you_marked(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "later", "coming_soon", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    sqlx::query("update users set personalize = false where id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    mark(&app, "interests", pat_id, "soon", "1 day").await;
    let list = slugs_of(&inbox(&app, req(Method::GET, "/v1/upcoming/for-you").cookie(&pat)).await);
    assert_eq!(list, ["later"]);
}

// ---------------------------------------------------------------------------------------------
// The catalog: ratings on tiles, my apps, similar apps (catalog.rs)

/// Tiles show % Slaps, the rating filter and the rating sort count an app only from
/// votes::MIN_VOTES votes.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn tile_ratings_need_the_minimum_number_of_votes(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let min = slop_api::votes::MIN_VOTES as usize;
    listed_with(&app, maker_id, "enough", "released", "").await;
    listed_with(&app, maker_id, "one-short", "released", "").await;
    votes_for(&app, "enough", min - 1, 1).await;
    votes_for(&app, "one-short", min - 1, 0).await;
    let page = inbox(&app, req(Method::GET, "/v1/apps?sort=rating")).await;
    assert_eq!(slugs_of(&page["apps"]), ["enough", "one-short"]);
    let tile = |slug: &str| {
        page["apps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["slug"] == slug)
            .unwrap()
            .clone()
    };
    assert_eq!(tile("enough")["not_slop_percent"], 80);
    assert!(tile("one-short")["not_slop_percent"].is_null());
    assert_eq!(tile("one-short")["vote_count"], (min - 1) as i64);
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?min_not_slop=50")).await,
        ["enough"]
    );
}

/// A maker's own apps: theirs only (drafts, delisted and private too), most recently changed
/// first; signed out, 401.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn my_apps_lists_only_the_makers_own_newest_first(db: PgPool) {
    let app = test_app(db).await;
    let (mia_id, mia) = app.user("Mia", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    listed_with(
        &app,
        mia_id,
        "listed",
        "released",
        "updated_at = now() - interval '3 days'",
    )
    .await;
    listed_with(
        &app,
        mia_id,
        "draft",
        "coming_soon",
        "status = 'draft', updated_at = now() - interval '1 day'",
    )
    .await;
    listed_with(
        &app,
        mia_id,
        "delisted",
        "released",
        "status = 'delisted', updated_at = now() - interval '2 days'",
    )
    .await;
    listed_with(
        &app,
        mia_id,
        "private",
        "released",
        "visibility = 'private', updated_at = now()",
    )
    .await;
    listed_with(&app, bob_id, "bobs", "released", "").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/apps")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let mine = inbox(&app, req(Method::GET, "/v1/me/apps").cookie(&mia)).await;
    assert_eq!(slugs_of(&mine), ["private", "draft", "delisted", "listed"]);
    assert_eq!(mine[1]["status"], "draft");
}

/// Similar-app shelves: at most 6 each, no app twice, never community branches, no 18+ for an
/// under-18 viewer, nothing Coming soon under "popular", and only public apps, even on a private
/// app's page.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn similar_app_shelves_keep_to_the_store_rules(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "base", "released", "").await;
    tag_app(&app, "base", &["roguelike", "retro"]).await;
    for i in 0..9 {
        let slug = format!("match-{i}");
        listed_with(&app, maker_id, &slug, "released", "").await;
        tag_app(&app, &slug, &["roguelike", "retro"]).await;
    }
    for i in 0..9 {
        listed_with(&app, maker_id, &format!("played-{i}"), "released", "").await;
    }
    listed_with(&app, maker_id, "adult", "released", "min_age = 18").await;
    tag_app(&app, "adult", &["roguelike", "retro"]).await;
    listed_with(&app, maker_id, "fork", "released", "").await;
    sqlx::query("update apps set parent_app_id = (select id from apps where slug = 'base'), branch_kind = 'fix' where slug = 'fork'")
        .execute(&app.db)
        .await
        .unwrap();
    tag_app(&app, "fork", &["roguelike", "retro"]).await;
    listed_with(&app, maker_id, "teaser", "coming_soon", "").await;
    listed_with(
        &app,
        maker_id,
        "hidden-gem",
        "released",
        "visibility = 'private'",
    )
    .await;
    tag_app(&app, "hidden-gem", &["roguelike", "retro"]).await;
    for slug in [
        "teaser", "played-0", "played-1", "played-2", "played-3", "played-4", "played-5",
        "played-6", "adult",
    ] {
        sqlx::query(
            "insert into pulse_counts (app_id, bucket, players) select id, now() - interval '1 hour', 50 from apps where slug = $1",
        )
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let shelves = |r: Req| {
        let app = &app;
        async move {
            let body = inbox(app, r).await;
            ["like_this", "popular", "discover"].map(|k| slugs_of(&body[k]))
        }
    };
    let check = |s: &[Vec<String>; 3], page: &str| {
        let all: Vec<&String> = s.iter().flatten().collect();
        let unique: std::collections::HashSet<&&String> = all.iter().collect();
        assert_eq!(all.len(), unique.len(), "no app twice: {s:?}");
        assert!(s.iter().all(|shelf| shelf.len() <= 6), "{s:?}");
        assert!(
            !all.iter()
                .any(|a| *a == "fork" || *a == "hidden-gem" || *a == page),
            "{s:?}"
        );
        assert!(!s[1].contains(&"teaser".to_string()), "{s:?}");
    };

    let adult_view =
        shelves(req(Method::GET, "/v1/apps/base/similar").header("x-country", "US")).await;
    check(&adult_view, "base");
    assert_eq!(adult_view[0].len(), 6);
    assert_eq!(adult_view[1].len(), 6);
    assert!(adult_view.iter().flatten().any(|a| a == "adult"));
    let young_view =
        shelves(req(Method::GET, "/v1/apps/base/similar").header("x-country", "GB")).await;
    check(&young_view, "base");
    assert!(
        !young_view.iter().flatten().any(|a| a == "adult"),
        "{young_view:?}"
    );

    // A private app's page, seen by someone it's shared with: shelves of public apps only.
    let (pat_id, pat) = app.user("Pat", "US").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    tag_app(&app, "secret", &["roguelike", "retro"]).await;
    for slug in ["secret", "hidden-gem"] {
        approved_release(&app, slug, maker_id).await;
        sqlx::query(
            "insert into app_shares (app_id, user_id) select id, $1 from apps where slug = $2",
        )
        .bind(pat_id)
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let shared = shelves(req(Method::GET, "/v1/apps/secret/similar").cookie(&pat)).await;
    check(&shared, "secret");
    assert!(!shared[0].is_empty());
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret/similar").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// Labels: accessibility reports and Glow-ups (label.rs)

/// Withdrawing an accessibility report takes it out of the count and `mine`; nothing to withdraw
/// is a no-op, and only the caller's own report goes.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_withdraw_their_accessibility_report(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "tower",
        "released",
        "accessibility = '{subtitles}'",
    )
    .await;
    let (_, pat) = app.user("Pat", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    for c in [&pat, &sam] {
        app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(c))
            .await;
        let res = app
            .send(
                req(Method::PUT, "/v1/apps/tower/accessibility/subtitles")
                    .cookie(c)
                    .json(json!({ "works": true })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }
    let subtitles = |cookie: &str| {
        let app = &app;
        let cookie = cookie.to_string();
        async move {
            let label = inbox(app, req(Method::GET, "/v1/apps/tower/label").cookie(cookie)).await;
            label["accessibility"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["key"] == "subtitles")
                .unwrap()
                .clone()
        }
    };
    assert_eq!(subtitles(&pat).await["confirmed"], 2);
    let withdraw = |path: &str| req(Method::DELETE, path.to_string());
    assert_eq!(
        app.send(withdraw("/v1/apps/tower/accessibility/subtitles"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for path in [
        "/v1/apps/tower/accessibility/colorblind",
        "/v1/apps/no-such-app/accessibility/subtitles",
    ] {
        assert_eq!(
            app.send(withdraw(path).cookie(&pat)).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(subtitles(&pat).await["confirmed"], 2);
    assert_eq!(
        app.send(withdraw("/v1/apps/tower/accessibility/subtitles").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let mine = subtitles(&pat).await;
    assert_eq!(
        (mine["confirmed"].as_i64(), mine["mine"].clone()),
        (Some(1), Value::Null),
        "{mine}"
    );
    assert_eq!(subtitles(&sam).await["mine"], true);
}

/// Voters made straight in the database (no sessions), for vote counts.
async fn plain_voters(app: &TestApp, n: usize) -> Vec<Uuid> {
    let mut out = Vec::new();
    for i in 0..n {
        let id = Uuid::now_v7();
        sqlx::query("insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, 'US', now())")
            .bind(id)
            .bind(format!("Voter {i}"))
            .execute(&app.db)
            .await
            .unwrap();
        out.push(id);
    }
    out
}

/// An app's votes 90 days ago (`then`: 1/-1 per voter) and now (`now`).
async fn rated(app: &TestApp, voters: &[Uuid], slug: &str, then: &[i16], now: &[i16]) {
    let id = app_id(app, slug).await;
    for (voter, vote) in voters.iter().zip(then) {
        sqlx::query("insert into vote_events (id, user_id, app_id, vote, weight, created_at) values ($1, $2, $3, $4, 1, now() - interval '90 days')")
            .bind(Uuid::now_v7())
            .bind(voter)
            .bind(id)
            .bind(vote)
            .execute(&app.db)
            .await
            .unwrap();
    }
    for (voter, vote) in voters.iter().zip(now) {
        sqlx::query("insert into votes (user_id, app_id, vote, weight) values ($1, $2, $3, 1)")
            .bind(voter)
            .bind(id)
            .bind(vote)
            .execute(&app.db)
            .await
            .unwrap();
    }
}

/// Glow-ups: enough votes then and now, a rise of 15 points or more, no community branches,
/// the viewer's age policy, at most 8, biggest rise first.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn glow_ups_need_enough_votes_and_a_clear_rise(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let voters = plain_voters(&app, 10).await;
    let (u, d) = (1i16, -1i16);
    // 1 of 5 then (20%); now k of 5 for rise-k: 40%, 60%, 80%, 100%.
    let then = [u, d, d, d, d];
    for k in 2..=5usize {
        let slug = format!("rise-{k}");
        listed_with(&app, maker_id, &slug, "released", "").await;
        let now: Vec<i16> = (0..5).map(|i| if i < k { u } else { d }).collect();
        rated(&app, &voters, &slug, &then, &now).await;
    }
    // Excluded: too few votes then or now, a small rise, a branch; and 18+ in a young view.
    let cases: [(&str, &str, Vec<i16>, Vec<i16>); 5] = [
        ("few-then", "", vec![d, d, d, d], vec![u, u, u, u, u]),
        ("few-now", "", vec![d, d, d, d, d], vec![u, u, u, u]),
        (
            "small-rise",
            "",
            vec![u, u, u, d, d],
            vec![u, u, u, u, u, u, u, d, d, d],
        ),
        ("branch", "", then.to_vec(), vec![u, u, u, u, u]),
        ("adult", "min_age = 18", then.to_vec(), vec![u, u, u, u, u]),
    ];
    for (slug, set, then, now) in &cases {
        listed_with(&app, maker_id, slug, "released", set).await;
        rated(&app, &voters, slug, then, now).await;
    }
    sqlx::query("update apps set parent_app_id = (select id from apps where slug = 'rise-2'), branch_kind = 'fix' where slug = 'branch'")
        .execute(&app.db)
        .await
        .unwrap();
    let glow = |country: &str| {
        let app = &app;
        let country = country.to_string();
        async move {
            inbox(
                app,
                req(Method::GET, "/v1/shelves/glow-ups").header("x-country", country),
            )
            .await
        }
    };
    let us = glow("US").await;
    let order = slugs_of(&us);
    let mut tied = order[..2].to_vec();
    tied.sort();
    assert_eq!(tied, ["adult", "rise-5"], "the same rise: {order:?}");
    assert_eq!(&order[2..], ["rise-4", "rise-3", "rise-2"]);
    assert_eq!(
        (us[0]["before"].as_i64(), us[0]["after"].as_i64()),
        (Some(20), Some(100))
    );
    assert_eq!(
        slugs_of(&glow("GB").await),
        ["rise-5", "rise-4", "rise-3", "rise-2"]
    );

    // At most 8.
    for k in 0..6 {
        let slug = format!("more-{k}");
        listed_with(&app, maker_id, &slug, "released", "").await;
        rated(&app, &voters, &slug, &then, &[u, u, u, u, d]).await;
    }
    assert_eq!(slugs_of(&glow("US").await).len(), 8);
}

/// Each player's vote as it stood 60 days ago is their latest event then, even when two events
/// share a timestamp: the later one counts.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn glow_ups_take_the_later_of_two_votes_at_the_same_moment(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let voters = plain_voters(&app, 5).await;
    // Then: each voter said Slop and, in the same instant, changed to Slaps (100% then).
    // Written in both orders, so neither the table's order nor luck decides.
    for (slug, later_first) in [("tower-a", true), ("tower-b", false)] {
        listed_with(&app, maker_id, slug, "released", "").await;
        let tower = app_id(&app, slug).await;
        for voter in &voters {
            let first = Uuid::now_v7();
            let second = Uuid::now_v7();
            assert!(second > first);
            let events = if later_first {
                [(second, 1i16), (first, -1i16)]
            } else {
                [(first, -1i16), (second, 1i16)]
            };
            for (id, vote) in events {
                sqlx::query("insert into vote_events (id, user_id, app_id, vote, weight, created_at) values ($1, $2, $3, $4, 1, date_trunc('second', now()) - interval '90 days')")
                .bind(id)
                .bind(voter)
                .bind(tower)
                .bind(vote)
                .execute(&app.db)
                .await
                .unwrap();
            }
            sqlx::query("insert into votes (user_id, app_id, vote, weight) values ($1, $2, 1, 1)")
                .bind(voter)
                .bind(tower)
                .execute(&app.db)
                .await
                .unwrap();
        }
    }
    // 100% then and now: no glow-up.
    let glow = inbox(&app, req(Method::GET, "/v1/shelves/glow-ups")).await;
    assert_eq!(glow, json!([]), "{glow}");
}

// ---------------------------------------------------------------------------------------------
// For You scoring (foryou.rs), on candidates made in memory from one real row

use slop_api::catalog::AppKind;
use slop_api::foryou::{self, Candidate, PickKind, Signals};

struct Shop {
    template: Candidate,
    list: Vec<Candidate>,
}

impl Shop {
    async fn new(db: PgPool) -> Self {
        let app = test_app(db).await;
        let (maker_id, _) = app.user("Maker", "US").await;
        listed_with(&app, maker_id, "template", "released", "").await;
        let template = foryou::candidates(
            &app.db,
            &app.config,
            slop_api::policy::ViewerPolicy::FULL_VIEW,
        )
        .await
        .unwrap()
        .remove(0);
        Shop {
            template,
            list: Vec::new(),
        }
    }

    /// Adds an app with these tags; returns its ID.
    fn add(&mut self, slug: &str, kind: AppKind, tags: &[&str]) -> Uuid {
        let mut c = self.template.clone();
        c.id = Uuid::now_v7();
        c.maker_id = Uuid::now_v7();
        c.app.slug = slug.to_string();
        c.app.kind = kind;
        c.app.tags = tags.iter().map(|t| t.to_string()).collect();
        let id = c.id;
        self.list.push(c);
        id
    }

    fn get(&mut self, slug: &str) -> &mut Candidate {
        self.list.iter_mut().find(|c| c.app.slug == slug).unwrap()
    }

    fn slate(&self, signals: &Signals, personalized: bool, seed: u64) -> Vec<(String, PickKind)> {
        foryou::recommend(
            signals,
            &self.list,
            &HashMap::new(),
            personalized,
            seed,
            foryou::SLATE,
        )
        .into_iter()
        .map(|p| (self.list[p.index].app.slug.clone(), p.kind))
        .collect()
    }
}

/// Never recommended: what the user has, marked, launched or voted on, their own apps, and dead
/// projects, whether personal or not.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn for_you_leaves_out_what_the_user_already_has(db: PgPool) {
    let mut shop = Shop::new(db).await;
    let mut s = Signals::default();
    let me = Uuid::now_v7();
    s.user_id = Some(me);
    for slug in [
        "owned", "marked", "launched", "voted", "mine", "dead", "fine-1", "fine-2",
    ] {
        shop.add(slug, AppKind::Game, &["roguelike"]);
    }
    s.library.insert(shop.get("owned").id);
    s.interested.insert(shop.get("marked").id);
    s.launched.insert(shop.get("launched").id, (600, 1));
    s.votes.insert(shop.get("voted").id, 1);
    shop.get("mine").maker_id = me;
    shop.get("dead").app.is_dead = true;
    for personalized in [true, false] {
        let mut got: Vec<String> = shop
            .slate(&s, personalized, 7)
            .into_iter()
            .map(|(slug, _)| slug)
            .collect();
        got.sort();
        assert_eq!(got, ["fine-1", "fine-2"], "personalized: {personalized}");
    }
}

/// A personal slate: matches to the user's taste, every fourth card a discovery from outside it,
/// no two discoveries led by the same tag; disliked tags push apps down or out; the same seed
/// gives the same slate.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn for_you_mixes_matches_and_discoveries(db: PgPool) {
    let mut shop = Shop::new(db).await;
    let mut s = Signals::default();
    for i in 0..2 {
        let id = shop.add(
            &format!("liked-{i}"),
            AppKind::Game,
            &["roguelike", "pixel-art"],
        );
        s.library.insert(id);
    }
    let hated = shop.add("hated", AppKind::Game, &["horror"]);
    s.votes.insert(hated, -1);
    for i in 0..10 {
        shop.add(
            &format!("rogue-{i}"),
            AppKind::Game,
            &["roguelike", "pixel-art"],
        );
    }
    shop.add(
        "rogue-horror",
        AppKind::Game,
        &["roguelike", "pixel-art", "horror"],
    );
    shop.add("pure-horror", AppKind::Game, &["horror"]);
    for (slug, tags) in [
        ("racer-1", &["racing", "arcade"][..]),
        ("racer-2", &["racing", "cars"][..]),
        ("cozy", &["cozy"][..]),
        ("puzzle", &["puzzle"][..]),
        ("music", &["music"][..]),
    ] {
        shop.add(slug, AppKind::Game, tags);
    }

    let slate = shop.slate(&s, true, 42);
    assert_eq!(slate.len(), foryou::SLATE);
    for (i, (slug, kind)) in slate.iter().enumerate() {
        let want = if (i + 1) % foryou::DISCOVER_EVERY == 0 {
            PickKind::Discover
        } else {
            PickKind::Match
        };
        assert_eq!(*kind, want, "card {i} ({slug}): {slate:?}");
    }
    let discoveries: Vec<&str> = slate
        .iter()
        .filter(|(_, k)| *k == PickKind::Discover)
        .map(|(s, _)| s.as_str())
        .collect();
    assert!(
        discoveries.iter().all(|d| !d.starts_with("rogue")),
        "{discoveries:?}"
    );
    assert!(
        !(discoveries.contains(&"racer-1") && discoveries.contains(&"racer-2")),
        "two discoveries led by racing: {discoveries:?}"
    );
    let all: Vec<&str> = slate.iter().map(|(s, _)| s.as_str()).collect();
    assert!(
        !all.contains(&"pure-horror"),
        "a disliked tag alone keeps it out: {all:?}"
    );
    let pos = |slug: &str| all.iter().position(|s| *s == slug);
    let horror = pos("rogue-horror");
    assert!(
        horror.is_none() || horror > pos("rogue-0").max(pos("rogue-9")),
        "a disliked tag lowers a match: {all:?}"
    );

    assert_eq!(
        shop.slate(&s, true, 42),
        slate,
        "the same seed, the same slate"
    );
    let seed = foryou::daily_seed(
        Uuid::from_u128(5),
        time::Date::from_calendar_date(2026, time::Month::October, 22).unwrap(),
    );
    assert_eq!(
        seed,
        foryou::daily_seed(
            Uuid::from_u128(5),
            time::Date::from_calendar_date(2026, time::Month::October, 22).unwrap()
        )
    );
    assert_ne!(
        seed,
        foryou::daily_seed(
            Uuid::from_u128(5),
            time::Date::from_calendar_date(2026, time::Month::October, 23).unwrap()
        )
    );
    assert_ne!(
        seed,
        foryou::daily_seed(
            Uuid::from_u128(6),
            time::Date::from_calendar_date(2026, time::Month::October, 22).unwrap()
        )
    );

    // Without personal recommendations (or no taste yet): popular, rated and new, nothing else.
    let plain = shop.slate(&s, false, 42);
    assert!(plain.iter().all(|(_, k)| *k == PickKind::Popular));
    assert_eq!(plain.len(), foryou::SLATE);
    assert!(
        shop.slate(&Signals::default(), true, 42)
            .iter()
            .all(|(_, k)| *k == PickKind::Popular)
    );
}

/// Games and apps get the share of match cards the user gives them: someone with three games
/// and one tool sees mostly games, and some tools.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn for_you_matches_games_and_apps_in_the_users_proportion(db: PgPool) {
    let mut shop = Shop::new(db).await;
    let mut s = Signals::default();
    for i in 0..3 {
        let id = shop.add(&format!("my-game-{i}"), AppKind::Game, &["roguelike"]);
        s.library.insert(id);
    }
    let tool = shop.add("my-tool", AppKind::App, &["productivity"]);
    s.library.insert(tool);
    for i in 0..15 {
        shop.add(&format!("game-{i}"), AppKind::Game, &["roguelike"]);
    }
    for i in 0..6 {
        shop.add(&format!("tool-{i}"), AppKind::App, &["productivity"]);
    }
    let slate = shop.slate(&s, true, 1);
    let matches: Vec<&String> = slate
        .iter()
        .filter(|(_, k)| *k == PickKind::Match)
        .map(|(s, _)| s)
        .collect();
    assert_eq!(matches.len(), 9, "{slate:?}");
    let tools = matches.iter().filter(|s| s.starts_with("tool-")).count();
    assert_eq!(tools, 2, "a quarter of the matches, rounded: {slate:?}");
}

// ---------------------------------------------------------------------------------------------
// Store-page pictures (media.rs, admin_web.rs, agent.rs)

/// A draft with pictures at the head commit, checked: its set is pending. Returns the set's ID.
async fn pending_pictures(app: &TestApp, maker: Uuid, cookie: &str, slug: &str) -> String {
    app_with_pictures(app, maker, slug, &"b".repeat(40)).await;
    let res = app
        .send(req(Method::POST, format!("/v1/apps/{slug}/media/check")).cookie(cookie))
        .await;
    let body = body_json(res).await;
    assert_eq!(body["result"], "pending", "{body}");
    body["pending"]["id"].as_str().unwrap().to_string()
}

/// The maker's pictures page: only the maker (or their agent); the live, pending and turned-down
/// sets, in the maker's order; not supported for repositories outside GitHub.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_makers_pictures_page_shows_each_set(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (_, admin) = make_admin(&app, "Admin").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let first = pending_pictures(&app, maker_id, &maker, "pics").await;
    let overview = |r: Req| {
        let app = &app;
        async move { inbox(app, r).await }
    };
    for r in [
        req(Method::GET, "/v1/apps/pics/media").cookie(&stranger),
        req(Method::GET, "/v1/apps/nope/media").cookie(&maker),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/pics/media"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let o = overview(req(Method::GET, "/v1/apps/pics/media").cookie(&maker)).await;
    assert_eq!(o["pending"]["id"], first.as_str());
    assert!(o["live"].is_null() && o["rejected"].is_null());
    assert_eq!(o["supported"], true);
    assert_eq!(o["manifest_path"], ".slop/media.json");
    assert!(o["checked_at"].is_string());

    // Approved: live, in the maker's order.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/media/{first}/approve")).cookie(&admin))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    sqlx::query("update apps set media_order = '{image:shots/b.png,video:dQw4w9WgXcQ,image:shots/a.png}' where slug = 'pics'")
        .execute(&app.db)
        .await
        .unwrap();
    let o = overview(req(Method::GET, "/v1/apps/pics/media").cookie(&maker)).await;
    assert_eq!(o["live"]["id"], first.as_str());
    let paths: Vec<Value> = o["live"]["gallery"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].clone())
        .collect();
    assert_eq!(
        paths,
        [json!("shots/b.png"), Value::Null, json!("shots/a.png")]
    );
    assert!(o["pending"].is_null());

    // A newer set turned down shows as rejected, with the reason, next to the live one.
    let next = "9".repeat(40);
    app_with_pictures_files(&next);
    let id = app_id(&app, "pics").await;
    let slop_api::media::Outcome::Pending(second) = slop_api::media::refresh(
        &app.state,
        id,
        "https://github.com/octo/game",
        Some(&next),
        "release",
    )
    .await
    .unwrap() else {
        panic!("expected new pictures");
    };
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/media/{second}/reject"))
                .cookie(&admin)
                .json(json!({ "reason": "Blurry." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let o = overview(req(Method::GET, "/v1/apps/pics/media").cookie(&maker)).await;
    assert_eq!(o["live"]["id"], first.as_str());
    assert_eq!(o["rejected"]["id"], second.to_string());
    assert_eq!(o["rejected"]["status_reason"], "Blurry.");

    // The maker's agent may look.
    let agent = maker_token(&app, &maker, json!({ "name": "Claude", "scope": "read" })).await;
    let o = overview(req(Method::GET, "/v1/apps/pics/media").header("authorization", agent)).await;
    assert_eq!(o["live"]["id"], first.as_str());

    // A repository outside GitHub: not supported.
    listed_with(
        &app,
        maker_id,
        "elsewhere",
        "released",
        "repo_url = 'https://codeberg.org/x/y'",
    )
    .await;
    let o = overview(req(Method::GET, "/v1/apps/elsewhere/media").cookie(&maker)).await;
    assert_eq!(o["supported"], false);
    assert!(o["live"].is_null() && o["pending"].is_null());
}

/// Ordering the gallery: keys that look like gallery keys, 40 at most, only the maker, and not
/// through an agent.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn gallery_orders_are_checked(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    listed_with(
        &app,
        maker_id,
        "pics",
        "released",
        "repo_url = 'https://github.com/octo/game'",
    )
    .await;
    let order =
        |keys: Value| req(Method::PUT, "/v1/apps/pics/media/order").json(json!({ "keys": keys }));
    let stored = || async {
        sqlx::query_scalar::<_, Vec<String>>("select media_order from apps where slug = 'pics'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let long = format!("image:{}", "x".repeat(215));
    let forty_one: Vec<String> = (0..41).map(|i| format!("image:{i}.png")).collect();
    for bad in [
        json!(["shots/a.png"]),
        json!(["image:a.png", "audio:b.ogg"]),
        json!([long]),
        json!(forty_one),
    ] {
        let res = app.send(order(bad.clone()).cookie(&maker)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    assert_eq!(
        app.send(order(json!(["image:a.png"])).cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let agent = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    assert_eq!(
        app.send(order(json!(["image:a.png"])).header("authorization", agent))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(stored().await.is_empty());
    let forty: Vec<String> = (0..40).map(|i| format!("image:{i}.png")).collect();
    let res = app.send(order(json!(forty)).cookie(&maker)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(stored().await.len(), 40);
}

/// Turning pictures down: Review permission, a reason, only a pending set; logged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_turn_down_pictures_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (_, reviewer) = staff_with(&app, "Rae Reviewer", "reviewer").await;
    let (_, support) = staff_with(&app, "Sue Support", "support").await;
    let set = pending_pictures(&app, maker_id, &maker, "pics").await;
    let reject = |cookie: &str, reason: &str| {
        req(Method::POST, format!("/v1/admin/media/{set}/reject"))
            .cookie(cookie)
            .json(json!({ "reason": reason }))
    };
    let status = || async {
        sqlx::query_scalar::<_, String>("select status from media_sets where id = $1::uuid")
            .bind(&set)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    assert_eq!(
        app.send(reject(&maker, "No.")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(reject(&support, "No.")).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = app.send(reject(&reviewer, "   ")).await;
    assert!(res.status().is_client_error(), "{}", res.status());
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("say why")
    );
    assert_eq!(status().await, "pending");
    assert!(audits(&app, "media.reject").await.is_empty());

    assert_eq!(
        app.send(reject(&reviewer, " Shows another game. "))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(status().await, "rejected");
    let logged = audits(&app, "media.reject").await;
    assert_eq!(
        logged,
        [json!({ "set": set, "reason": "Shows another game." })]
    );
    // Not pending any more: neither again nor approved now.
    assert_eq!(
        app.send(reject(&reviewer, "Again.")).await.status(),
        StatusCode::CONFLICT
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/media/{set}/approve")).cookie(&reviewer))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/media/{}/reject", Uuid::now_v7()),
            )
            .cookie(&reviewer)
            .json(json!({ "reason": "x" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

/// Reviewer agents decide pictures through MCP: a review token approves or turns down (with a
/// reason); a read token can't.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewer_agents_decide_pictures(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let first = pending_pictures(&app, maker_id, &maker, "pics-a").await;
    let token = |scope: &str| {
        let app = &app;
        let r = req(Method::POST, "/v1/admin/tokens")
            .cookie(&owner)
            .json(json!({ "name": format!("{scope} agent"), "scope": scope, "days": 7 }));
        async move {
            format!(
                "Bearer {}",
                body_json(app.send(r).await).await["token"]
                    .as_str()
                    .unwrap()
            )
        }
    };
    let (read, review) = (token("read").await, token("review").await);
    let call = |bearer: &str, args: Value| {
        let app = &app;
        let r = req(Method::POST, "/v1/admin/mcp")
            .header("authorization", bearer.to_string())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "decide_pictures", "arguments": args } }));
        async move {
            let out = body_json(app.send(r).await).await;
            (
                out["result"]["isError"] == false,
                out["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };
    let status = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select status from media_sets where id = $1::uuid")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let (ok, _) = call(&read, json!({ "id": first, "decision": "approve" })).await;
    assert!(!ok);
    assert_eq!(status(first.clone()).await, "pending");
    let (ok, text) = call(&review, json!({ "id": first, "decision": "reject" })).await;
    assert!(!ok, "a reason is needed: {text}");
    let (ok, text) = call(
        &review,
        json!({ "id": first, "decision": "reject", "reason": "Watermarked." }),
    )
    .await;
    assert!(ok, "{text}");
    assert_eq!(status(first.clone()).await, "rejected");
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'media.reject'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("review agent"), "{actor}");

    // A second app's pictures, approved by the agent.
    sqlx::query("update apps set slug = 'pics-old' where slug = 'pics-a'")
        .execute(&app.db)
        .await
        .unwrap();
    let second = pending_pictures(&app, maker_id, &maker, "pics-b").await;
    let (ok, text) = call(&review, json!({ "id": second, "decision": "approve" })).await;
    assert!(ok, "{text}");
    assert_eq!(status(second.clone()).await, "approved");
    let live: Option<Uuid> =
        sqlx::query_scalar("select media_set_id from apps where slug = 'pics-b'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(live.map(|u| u.to_string()), Some(second));
}

/// After a release is fetched, its commit's pictures are read: a new set waits for review; a
/// repository outside GitHub is skipped, and a broken picture list never fails anything.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_bring_their_pictures_for_review(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    // A release fetched for an app outside GitHub: fetched fine, no pictures read.
    let _ = submit_and_fetch(&app, &maker, "1.0.0").await;
    let (status, problem): (String, Option<String>) = sqlx::query_as(
        "select r.status::text, a.media_problem from releases r join apps a on a.id = r.app_id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "awaiting_review");
    assert!(problem.is_none());
    assert_eq!(count(&app.db, "select count(*) from media_sets").await, 0);

    let commit = "7".repeat(40);
    app_with_pictures(&app, maker_id, "pics", &commit).await;
    let id = app_id(&app, "pics").await;
    slop_api::media::refresh_for_release(&app.state, id, "https://github.com/octo/game", &commit)
        .await;
    let (status, source): (String, String) =
        sqlx::query_as("select status, source from media_sets where app_id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), source.as_str()), ("pending", "release"));

    // A broken list at the next release: no new set, the problem kept for the maker.
    let broken = "8".repeat(40);
    put_raw(&broken, "media.json", b"{ not json".to_vec());
    slop_api::media::refresh_for_release(&app.state, id, "https://github.com/octo/game", &broken)
        .await;
    assert_eq!(
        count(
            &app.db,
            &format!("select count(*) from media_sets where app_id = '{id}'")
        )
        .await,
        1
    );
    let problem: Option<String> =
        sqlx::query_scalar("select media_problem from apps where id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(problem.unwrap().contains("isn't valid"));
    // Outside GitHub: nothing read, nothing recorded.
    slop_api::media::refresh_for_release(&app.state, id, "https://codeberg.org/octo/game", &commit)
        .await;
    assert_eq!(
        count(
            &app.db,
            &format!("select count(*) from media_sets where app_id = '{id}'")
        )
        .await,
        1
    );
}

/// Live thumbnails from before look-alike hashing get a hash a few at a time; one that can't be
/// read gets an empty hash so it isn't tried again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn old_thumbnails_are_hashed_a_few_at_a_time(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (_, admin) = make_admin(&app, "Admin").await;
    for slug in ["one", "two", "three"] {
        let set = pending_pictures(&app, maker_id, &maker, slug).await;
        let res = app
            .send(req(Method::POST, format!("/v1/admin/media/{set}/approve")).cookie(&admin))
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        // Renamed, to tell the live ones apart below.
        sqlx::query("update apps set slug = slug || '-done' where slug = $1")
            .bind(slug)
            .execute(&app.db)
            .await
            .unwrap();
    }
    // As if hashed before look-alike checks existed; one picture has since gone.
    sqlx::query("update media_sets set thumbnail = thumbnail - 'dhash'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "update media_sets set thumbnail = jsonb_set(thumbnail, '{url}', to_jsonb($1::text))
         where id = (select media_set_id from apps where slug = 'two-done')",
    )
    .bind(format!(
        "{}raw/octo/game/{}/.slop/gone.png",
        app.fake_base,
        "b".repeat(40)
    ))
    .execute(&app.db)
    .await
    .unwrap();
    let hashes = || async {
        let mut rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "select a.slug, ms.thumbnail->>'dhash' from apps a join media_sets ms on ms.id = a.media_set_id",
        )
        .fetch_all(&app.db)
        .await
        .unwrap();
        rows.sort();
        rows
    };
    assert!(hashes().await.iter().all(|(_, h)| h.is_none()));

    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 2)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        hashes().await.iter().filter(|(_, h)| h.is_some()).count(),
        2
    );
    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 2)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 2)
            .await
            .unwrap(),
        0,
        "never retried"
    );
    let done = hashes().await;
    assert_eq!(done.len(), 3);
    for (slug, hash) in done {
        let hash = hash.unwrap();
        if slug == "two-done" {
            assert_eq!(hash, "", "unreadable: an empty hash");
        } else {
            assert_eq!(hash.len(), 16, "{slug}: {hash}");
        }
    }
}

/// Emails the law asks for (a server donation's renewal reminder) go to every donor with an
/// address whatever their email choices, but never to a closed account, and not without SMTP.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn required_emails_skip_closed_accounts_and_need_smtp(db: PgPool) {
    let app = test_app_with(db.clone(), |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        })
    })
    .await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "server-game",
        "released",
        "title = 'Server Game'",
    )
    .await;
    for (name, email, closed) in [
        ("Open", Some("open@example.com"), false),
        ("No email", None, false),
        ("Closed", Some("closed@example.com"), true),
        ("Opted out", Some("quiet@example.com"), false),
    ] {
        let (id, _) = app.user(name, "US").await;
        sqlx::query(
            "update users set email = $2, closed_at = case when $3 then now() end, email_prefs = '{\"donations\": false}' where id = $1",
        )
        .bind(id)
        .bind(email)
        .bind(closed)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor, slop_fee_minor,
                                           total_minor, consent_text, status, started_at, next_charge_at)
             select $1, id, $2, 'acct_x', 'usd', 500, 25, 525, 'ok', 'active', now() - interval '20 days', now() + interval '2 days'
             from apps where slug = 'server-game'",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    slop_api::donations::hourly(&app.state).await.unwrap();
    let mut to: Vec<String> = sqlx::query_scalar("select to_address from email_outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();
    to.sort();
    assert_eq!(to, ["open@example.com", "quiet@example.com"]);
    let body: String =
        sqlx::query_scalar("select body from email_outbox where to_address = 'open@example.com'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        body.starts_with("Hi Open,") && body.contains("Server Game"),
        "{body}"
    );

    // Without SMTP: the reminder is in SLAPorSLOP, but no email is queued.
    sqlx::query("update server_donations set first_reminder_at = null")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("delete from email_outbox")
        .execute(&app.db)
        .await
        .unwrap();
    let quiet = test_app(db).await;
    slop_api::donations::hourly(&quiet.state).await.unwrap();
    assert_eq!(
        count(&quiet.db, "select count(*) from email_outbox").await,
        0
    );
    assert!(
        count(
            &quiet.db,
            "select count(*) from notifications where category = 'donations'"
        )
        .await
            >= 2
    );
}
