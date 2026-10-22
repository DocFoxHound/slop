// SPDX-License-Identifier: AGPL-3.0-or-later
//! The store and catalog, front page and shelves, For You, reviews, votes, labels, playtime, cloud saves, demos, media, notifications and push.

use crate::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------------------------
// Helpers for this area

/// A small GitHub API for licence lookups that counts what it was asked. octo/secret is private:
/// its licence is there only with Slop's token. octo/flaky answers 502. Every repository is
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

/// Only the repository's owner asks GitHub with Slop's credentials, and what they see isn't
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
    // The owner gets it with Slop's token...
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
    // The same app on a Slop without a play origin: not available, and starting one is refused.
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

/// Email choices: every category on by default, whether this Slop sends email and whether the
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
    assert_eq!(cats.len(), 10);
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
            from: "Slop <n@slop.test>".into(),
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
            from: "Slop <n@slop.test>".into(),
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
            from: "Slop <n@slop.test>".into(),
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
            from: "Slop <n@slop.test>".into(),
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
            from: "Slop <n@slop.test>".into(),
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
