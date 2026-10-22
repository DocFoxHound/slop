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
