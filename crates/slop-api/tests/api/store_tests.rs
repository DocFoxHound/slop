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
