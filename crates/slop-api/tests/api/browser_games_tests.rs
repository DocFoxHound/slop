// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser games (DESIGN §8): store pages that play only in the browser, releases that are play
//! addresses (checked, approved by a person, never signed), where Play goes and who sees it, and
//! everything only downloads have, refused for them.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// A new store page that plays in the browser.
fn browser_page(slug: &str) -> Value {
    let mut body = new_app(slug);
    body["platforms"] = json!(["browser"]);
    body
}

/// (status, error.field, error.message) of a refusal.
async fn refusal(res: Response) -> (StatusCode, String, String) {
    let status = res.status();
    let body = body_json(res).await;
    let text = |k: &str| body["error"][k].as_str().unwrap_or_default().to_string();
    (status, text("field"), text("message"))
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The store page as `cookie` sees it (signed out: None), from the US.
async fn page_for(app: &TestApp, cookie: Option<&str>) -> Value {
    let mut r = req(Method::GET, "/v1/apps/game").header("x-country", "US");
    if let Some(c) = cookie {
        r = r.cookie(c);
    }
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// The releases list as `cookie` sees it (signed out: None).
async fn releases_for(app: &TestApp, cookie: Option<&str>) -> Value {
    let mut r = req(Method::GET, "/v1/apps/game/releases").header("x-country", "US");
    if let Some(c) = cookie {
        r = r.cookie(c);
    }
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// The newest notification `user` got with this title: its body.
async fn notice(app: &TestApp, user: Uuid, title: &str) -> String {
    sqlx::query_scalar(
        "select body from notifications where user_id = $1 and title = $2 order by created_at desc limit 1",
    )
    .bind(user)
    .bind(title)
    .fetch_optional(&app.db)
    .await
    .unwrap()
    .unwrap_or_else(|| panic!("no notification {title:?}"))
}

/// A release of `game` inserted as it is while waiting (or `status`), with its address.
async fn address_release(app: &TestApp, version: &str, status: &str, address: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, play_address, address_checks,
                               reviewed_at)
         select $1, a.id, $2, repeat('a', 40), $3::release_status, a.maker_id, $4, jsonb_build_object('ok', true),
                case when $3 in ('rejected', 'withdrawn') then clock_timestamp() end
         from apps a where a.slug = 'game'",
    )
    .bind(id)
    .bind(version)
    .bind(status)
    .bind(address)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

async fn approve(app: &TestApp, id: &str) -> admin::Approval {
    admin::approve(&app.db, None, lax(), id.parse().unwrap(), "rev")
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Store pages

/// "In the browser" is a store page's only platform, for a public game without sandbox mode or
/// a mods folder; it's saved without release watching, and summaries say it plays in the
/// browser (but never where: that's on the store page, for who may play it).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_browser_game_is_a_public_game_that_plays_only_in_the_browser(db: PgPool) {
    let app = test_app(db).await;
    let (_, tries) = app.user("Tries", "US").await;
    let (_, maker) = app.user("Maker", "US").await;
    let folder = json!({ "base": "game", "path": "mods", "layout": "folder" });
    for (change, field, why) in [
        (
            json!({ "platforms": ["browser", "windows"] }),
            "platforms",
            "plays only in the browser",
        ),
        (
            json!({ "platforms": ["linux", "linux-deb", "browser"] }),
            "platforms",
            "plays only in the browser",
        ),
        (
            json!({ "platforms": [] }),
            "platforms",
            "choose Windows, Linux or both, or In the browser",
        ),
        (
            json!({ "kind": "app" }),
            "platforms",
            "only games can play in the browser",
        ),
        (
            json!({ "visibility": "private" }),
            "visibility",
            "can't be private",
        ),
        (
            json!({ "sandbox_ready": true }),
            "sandbox_ready",
            "sandbox mode",
        ),
        (
            json!({ "mods_folder": folder }),
            "mods_folder",
            "no folder to read mods from",
        ),
    ] {
        let mut body = browser_page("orbit-tap");
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        let (status, got, message) = refusal(
            app.send(req(Method::POST, "/v1/apps").cookie(&tries).json(body))
                .await,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{change}");
        assert_eq!(got, field, "{change}");
        assert!(message.contains(why), "{change}: {message}");
    }
    assert_eq!(count(&app, "select count(*) from apps").await, 0);

    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(browser_page("orbit-tap")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let page = body_json(res).await;
    assert_eq!(page["platforms"], json!(["browser"]));
    assert_eq!(page["plays_in_browser"], true);
    assert!(page["play_address"].is_null(), "nothing approved yet");
    assert_eq!(page["sandbox_ready"], false);
    assert!(page["demo"].is_null());
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("castle")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["plays_in_browser"], false);
    let watched: Vec<(String, bool)> =
        sqlx::query_as("select slug, watch_releases from apps order by slug")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        watched,
        [
            ("castle".to_string(), true),
            ("orbit-tap".to_string(), false)
        ]
    );

    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/apps").cookie(&maker))
            .await,
    )
    .await;
    for a in mine.as_array().unwrap() {
        assert_eq!(a["plays_in_browser"], a["slug"] == "orbit-tap", "{a}");
        assert!(a.get("play_address").is_none(), "summaries never carry it");
        assert!(a.get("platforms").is_none());
    }
}

/// A store page can switch between a download and a browser game only while it has no release,
/// since its releases are files or play addresses, never both; saving the same platforms again
/// is always fine. Switching drops what only downloads have.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_store_page_switches_to_or_from_the_browser_only_before_its_first_release(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let folder = json!({ "base": "game", "path": "mods", "layout": "folder" });
    sqlx::query("update apps set sandbox_ready = true, mods_folder = $1 where slug = 'game'")
        .bind(&folder)
        .execute(&app.db)
        .await
        .unwrap();
    let patch = |slug: &str, body: Value| {
        req(Method::PATCH, format!("/v1/apps/{slug}"))
            .cookie(&maker)
            .json(body)
    };
    let watched = || async {
        sqlx::query_scalar::<_, bool>("select watch_releases from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    let res = app
        .send(patch("game", json!({ "platforms": ["browser"] })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(res).await;
    assert_eq!(page["platforms"], json!(["browser"]));
    assert_eq!(page["plays_in_browser"], true);
    assert_eq!(page["sandbox_ready"], false);
    assert!(page["mods_folder"].is_null());
    assert!(!watched().await);
    for (body, field) in [
        (json!({ "sandbox_ready": true }), "sandbox_ready"),
        (json!({ "mods_folder": folder }), "mods_folder"),
        (json!({ "platforms": ["browser", "windows"] }), "platforms"),
    ] {
        let (status, got, _) = refusal(app.send(patch("game", body.clone())).await).await;
        assert_eq!(
            (status, got.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, field),
            "{body}"
        );
    }
    let res = app
        .send(patch("game", json!({ "platforms": ["windows"] })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["plays_in_browser"], false);
    assert!(watched().await);
    let res = app
        .send(patch(
            "game",
            json!({ "platforms": ["browser"], "sandbox_ready": false }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // With a release, it stays a browser game; the same platforms (and anything else) save.
    approved_play_address(&app, "game", "1.0.0", "https://games.example.com/play/").await;
    let (status, _, message) = refusal(
        app.send(patch("game", json!({ "platforms": ["windows"] })))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("make a new store page"), "{message}");
    let res = app
        .send(patch(
            "game",
            json!({ "platforms": ["browser"], "title": "Orbit" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["title"], "Orbit");

    // A download with a release stays a download.
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, submitted_by)
         select $1, id, '1.0.0', repeat('a', 40), maker_id from apps where slug = 'castle'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let (status, _, _) = refusal(
        app.send(patch("castle", json!({ "platforms": ["browser"] })))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Only games, and no community builds of one or as one.
    listed_with(
        &app,
        maker_id,
        "tool",
        "released",
        "kind = 'app', platforms = '{windows}'",
    )
    .await;
    let (status, field, message) = refusal(
        app.send(patch("tool", json!({ "platforms": ["browser"] })))
            .await,
    )
    .await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "platforms")
    );
    assert!(message.contains("only games"), "{message}");
    listed_with(
        &app,
        maker_id,
        "tower",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "tower-linux",
        "released",
        "platforms = '{linux,linux-deb}', branch_kind = 'port',
         parent_app_id = (select id from apps where slug = 'tower')",
    )
    .await;
    let (status, _, message) = refusal(
        app.send(patch("tower", json!({ "platforms": ["browser"] })))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("community builds"), "{message}");
    let (status, field, _) = refusal(
        app.send(patch("tower-linux", json!({ "platforms": ["browser"] })))
            .await,
    )
    .await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "platforms")
    );
    // Nor registered as one.
    let (_, author) = app.user("Bo Brancher", "US").await;
    let (status, field, message) = refusal(
        app.send(
            req(Method::POST, "/v1/apps/tower/branches")
                .cookie(&author)
                .json(json!({
                    "slug": "tower-web", "title": "Tower on the web", "summary": "Plays in the browser.",
                    "repo_url": "https://github.com/bo/tower-web", "license_spdx": "MIT", "kind": "port",
                    "note": "A web port.", "platforms": ["browser"]
                })),
        )
        .await,
    )
    .await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "platforms")
    );
    assert!(
        message.contains("can't play in the browser yet"),
        "{message}"
    );
}

/// The store's "runs on" filter: browser games only match "browser", and downloads never do.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_store_lists_browser_games_by_where_they_run(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker,
        "orbit-tap",
        "released",
        "platforms = '{browser}'",
    )
    .await;
    listed_with(&app, maker, "castle", "released", "platforms = '{windows}'").await;
    listed_with(
        &app,
        maker,
        "penguin",
        "released",
        "platforms = '{linux,linux-deb}'",
    )
    .await;
    let on = |platform: &str| {
        req(Method::GET, format!("/v1/apps?platform={platform}")).header("x-country", "US")
    };
    assert_eq!(list_slugs(&app, on("browser")).await, ["orbit-tap"]);
    assert_eq!(list_slugs(&app, on("windows")).await, ["castle"]);
    assert_eq!(list_slugs(&app, on("linux")).await, ["penguin"]);
    let (status, field, message) = refusal(app.send(on("amiga")).await).await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "platform")
    );
    assert_eq!(message, "use windows, linux or browser");
    let all = body_json(
        app.send(req(Method::GET, "/v1/apps").header("x-country", "US"))
            .await,
    )
    .await;
    for a in all["apps"].as_array().unwrap() {
        assert_eq!(a["plays_in_browser"], a["slug"] == "orbit-tap", "{a}");
    }
}

/// The database keeps a browser game's platform alone, and a play address an http(s) address
/// that's never signed; launches, votes and bug reports can say "browser".
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_database_keeps_browser_games_apart(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = maker_with_browser_app(&app, "beta").await;
    for bad in ["{browser,windows}", "{linux,linux-deb,browser}", "{amiga}"] {
        assert!(
            sqlx::query("update apps set platforms = $1::text[] where slug = 'game'")
                .bind(bad)
                .execute(&app.db)
                .await
                .is_err(),
            "{bad}"
        );
    }
    let address = |a: String| {
        let db = app.db.clone();
        async move {
            sqlx::query(
                "insert into releases (id, app_id, version, commit_sha, submitted_by, play_address)
                 select $1, id, $2, repeat('a', 40), maker_id, $3 from apps where slug = 'game'",
            )
            .bind(Uuid::now_v7())
            .bind(format!("v{}", Uuid::now_v7().simple()))
            .bind(a)
            .execute(&db)
            .await
        }
    };
    for bad in [
        "javascript:alert(1)".to_string(),
        "ftp://games.example.com/".to_string(),
        format!("https://games.example.com/{}", "a".repeat(2000)),
    ] {
        assert!(address(bad.clone()).await.is_err(), "{bad}");
    }
    assert!(address("https://games.example.com/".into()).await.is_ok());
    let id = approved_play_address(&app, "game", "1.0.0", "https://games.example.com/play/").await;
    assert!(
        sqlx::query("update releases set signed_manifest = '{}' where id = $1")
            .bind(id)
            .execute(&app.db)
            .await
            .is_err(),
        "there's nothing to sign for an address"
    );

    let (player, _) = app.user("Pat", "US").await;
    let app_id: Uuid = sqlx::query_scalar("select id from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into user_app_usage (user_id, app_id, days_launched, last_platform) values ($1, $2, 1, 'browser')",
    )
    .bind(player)
    .bind(app_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into vote_events (id, user_id, app_id, vote, weight, platform) values ($1, $2, $3, 1, 1, 'browser')",
    )
    .bind(Uuid::now_v7())
    .bind(player)
    .bind(app_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into bug_reports (id, app_id, reporter_id, platform, title, body)
         values ($1, $2, $3, 'browser', 'Stuck', 'The ball goes through the wall.')",
    )
    .bind(Uuid::now_v7())
    .bind(app_id)
    .bind(player)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(
        sqlx::query("update user_app_usage set last_platform = 'amiga'")
            .execute(&app.db)
            .await
            .is_err()
    );
    let _ = maker_id;
}

// ---------------------------------------------------------------------------------------------
// Submitting and checking a play address

/// A browser game's release is a play address and a version, with no files or runtimes; a
/// download's has no address. Bad and known-bad addresses are refused on submit, the maker hears
/// about each release, and three waiting is the most, as for any app.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_browser_games_release_is_a_play_address(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "beta").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let submit = |cookie: &str, slug: &str, body: Value| {
        req(Method::POST, format!("/v1/apps/{slug}/releases"))
            .cookie(cookie)
            .json(body)
    };
    let good = play_page(&app, "play");
    assert_eq!(
        app.send(submit(&stranger, "game", play_body("1.0.0", &good)))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let with = |extra: Value| {
        let mut body = play_body("1.0.0", &good);
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };
    let mut no_address = play_body("1.0.0", &good);
    no_address.as_object_mut().unwrap().remove("play_address");
    let mut no_commit = play_body("1.0.0", &good);
    no_commit.as_object_mut().unwrap().remove("commit_sha");
    for (body, field, why) in [
        (
            with(json!({ "files": release_body(&app, "1.0.0", "x.zip")["files"] })),
            "files",
            "has no files",
        ),
        (
            with(json!({ "runtimes": ["dotnet-8"] })),
            "runtimes",
            "needs no runtimes",
        ),
        (no_address, "play_address", "give the https:// address"),
        (
            with(json!({ "play_address": "ftp://games.example.com/" })),
            "play_address",
            "https://",
        ),
        (
            with(json!({ "play_address": "javascript:alert(1)" })),
            "play_address",
            "https://",
        ),
        (
            with(
                json!({ "play_address": format!("http://me:secret@{}", &good["http://".len()..]) }),
            ),
            "play_address",
            "user name and password",
        ),
        (
            with(json!({ "play_address": "http://localhost:8080/play/" })),
            "play_address",
            "SLAPorSLOP's own site",
        ),
        (
            with(json!({ "play_address": format!("{good}{}", "a".repeat(2000)) })),
            "play_address",
            "2,000",
        ),
        (
            with(json!({ "version": "not a version" })),
            "version",
            "letters, numbers",
        ),
        (
            no_commit,
            "commit_sha",
            "give the commit your site is built from",
        ),
    ] {
        let (status, got, message) =
            refusal(app.send(submit(&maker, "game", body.clone())).await).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(got, field, "{body}");
        assert!(message.contains(why), "{body}: {message}");
    }

    // On a list of known-bad links: refused, naming the list (it takes nothing from anyone).
    sqlx::query(
        "insert into bad_links (url, host, source) values ('bad-games.example/play', 'bad-games.example', 'URLhaus')",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let (status, field, message) = refusal(
        app.send(submit(
            &maker,
            "game",
            play_body("1.0.0", "http://www.bad-games.example/play/"),
        ))
        .await,
    )
    .await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "play_address")
    );
    assert_eq!(
        message,
        "this address is on URLhaus, a list of known-bad links"
    );

    // A download's release has no play address, and always names its commit.
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    let mut file_release = release_body(&app, "1.0.0", "x.zip");
    file_release["play_address"] = json!(good);
    let (status, field, message) =
        refusal(app.send(submit(&maker, "castle", file_release)).await).await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "play_address")
    );
    assert!(
        message.contains("only games that play in the browser"),
        "{message}"
    );
    let mut file_release = release_body(&app, "1.0.0", "x.zip");
    file_release.as_object_mut().unwrap().remove("commit_sha");
    let (status, field, _) = refusal(app.send(submit(&maker, "castle", file_release)).await).await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "commit_sha")
    );
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    // A good one: checking its address, with no files, and the maker hears about it.
    let res = app
        .send(submit(&maker, "game", play_body("1.0.0", &good)))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["status"], "fetching");
    assert_eq!(release["play_address"], good);
    assert!(release["address_checks"].is_null());
    assert_eq!(release["files"], json!([]));
    assert_eq!(release["runtimes"], json!([]));
    assert_eq!(release["progress"]["stage"], "checking_address");
    assert_eq!(
        release["progress"]["summary"],
        "Checking the play address: https, a public site, the bad-link lists, and that the page loads."
    );
    assert!(
        notice(&app, maker_id, "Game 1.0.0 was submitted")
            .await
            .contains("SLAPorSLOP is checking its play address now")
    );
    assert_eq!(count(&app, "select count(*) from release_files").await, 0);

    // Three waiting is the most.
    for v in ["1.0.1", "1.0.2"] {
        assert_eq!(
            app.send(submit(&maker, "game", play_body(v, &good)))
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    let (status, _, message) = refusal(
        app.send(submit(&maker, "game", play_body("1.0.3", &good)))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("three releases"), "{message}");
}

/// A browser game on GitHub may leave the commit out: SLAPorSLOP takes the default branch's
/// newest. Elsewhere it has to be given.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_browser_game_on_github_may_leave_the_commit_out(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_browser_app(&app, "beta").await;
    let body = json!({ "version": "1.0.0", "play_address": play_page(&app, "play") });
    let submit = || {
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body.clone())
    };
    let (status, field, message) = refusal(app.send(submit()).await).await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "commit_sha")
    );
    assert!(
        message.contains("only look up the newest one on GitHub"),
        "{message}"
    );

    sqlx::query("update apps set repo_url = 'https://github.com/octo/game' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(submit()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(
        release["commit_sha"],
        "b".repeat(40),
        "the fake GitHub's HEAD"
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (status, verified): (String, bool) =
        sqlx::query_as("select status::text, commit_verified from releases")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), verified), ("awaiting_review", true));
}

/// On a server set up like production, a play address is https on a public site: plain http,
/// IP addresses and this machine are refused on submit, and an address that leads somewhere
/// private by the time it's checked fails its check.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn play_addresses_must_be_https_on_public_sites(db: PgPool) {
    let app = test_app_with(db, |c| c.releases.allow_http_downloads = false).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "beta").await;
    for (address, why) in [
        ("http://games.example.com/", "https://"),
        ("https://127.0.0.1/game/", "private network"),
        ("https://10.1.2.3/", "private network"),
        ("https://[::1]/", "private network"),
        ("https://169.254.169.254/latest/", "private network"),
        ("https://localhost/", "private network"),
        ("https://93.184.216.34/", "not its IP address"),
    ] {
        let (status, field, message) = refusal(
            app.send(
                req(Method::POST, "/v1/apps/game/releases")
                    .cookie(&maker)
                    .json(play_body("1.0.0", address)),
            )
            .await,
        )
        .await;
        assert_eq!(
            (status, field.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "play_address"),
            "{address}"
        );
        assert!(message.contains(why), "{address}: {message}");
    }
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    // One that got past (a name pointing somewhere else by now, here written in): its check fails.
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, submitted_by, play_address)
         values ($1, (select id from apps where slug = 'game'), '1.0.0', repeat('a', 40), $2, 'https://10.0.0.1/play/')",
    )
    .bind(id)
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (status, reason, checks): (String, Option<String>, Value) = sqlx::query_as(
        "select status::text, status_reason, address_checks from releases where id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "failed");
    assert!(reason.unwrap().contains("private network"));
    assert_eq!(checks["public_host"], false);
    assert_eq!(checks["https"], true);
    assert_eq!(checks["ok"], false);
}

/// The fetcher loads a play page the way Play will open it, following redirects itself, and
/// records what it found. A page that doesn't load, isn't a web page, is a download, or leads to
/// SLAPorSLOP's own site or to a listed address fails, and the maker hears why.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_fetcher_checks_that_a_play_page_loads(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "beta").await;
    let checks_of = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>, Value)>(
                "select status::text, status_reason, address_checks from releases where id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };

    let page = play_page(&app, "play");
    let ok = submit_play_and_check(&app, &maker, "1.0.0", &page).await;
    let (status, reason, checks) = checks_of(ok.clone()).await;
    assert_eq!(status, "awaiting_review");
    assert!(
        reason.unwrap().contains("first release"),
        "auto-approval ran"
    );
    assert_eq!(checks["ok"], true);
    assert!(checks["problem"].is_null());
    assert_eq!(checks["status"], 200);
    assert_eq!(checks["content_type"], "text/html");
    assert_eq!(checks["redirects"], 0);
    assert_eq!(checks["final_address"], page);
    assert_eq!(
        checks["https"], false,
        "the test setup's pages are plain http"
    );
    assert!(
        checks["public_host"].is_null(),
        "not checked on a test setup"
    );
    assert_eq!(
        checks["bad_links"],
        json!({ "listed_by": null, "lists_loaded": false }),
        "no lists loaded here: nothing can be said about them"
    );
    assert!(checks["checked_at"].is_string());
    let mine = releases_for(&app, Some(&maker)).await;
    assert_eq!(mine[0]["address_checks"], checks);
    assert_eq!(mine[0]["progress"]["stage"], "waiting_for_review");
    assert!(
        mine[0]["progress"]["summary"]
            .as_str()
            .unwrap()
            .starts_with("The play address passed SLAPorSLOP's checks.")
    );

    let moved = submit_play_and_check(&app, &maker, "1.0.1", &play_page(&app, "play-moved")).await;
    let (status, _, checks) = checks_of(moved.clone()).await;
    assert_eq!(status, "awaiting_review");
    assert_eq!(checks["redirects"], 1);
    assert_eq!(checks["final_address"], page);
    assert_eq!(checks["content_type"], "text/html");
    for id in [&ok, &moved] {
        admin::reject(&app.db, id.parse().unwrap(), "rev", "Just testing.")
            .await
            .unwrap();
    }

    for (n, (page, why)) in [
        (
            "play-missing",
            "the page didn't load: the site answered HTTP 404",
        ),
        (
            "play-binary",
            "it doesn't open a web page: the site sends application/octet-stream",
        ),
        ("play-file", "it downloads a file instead of opening a page"),
        ("play-home", "it leads to SLAPorSLOP's own site"),
    ]
    .into_iter()
    .enumerate()
    {
        let version = format!("2.0.{n}");
        let id = submit_play_and_check(&app, &maker, &version, &play_page(&app, page)).await;
        let (status, reason, checks) = checks_of(id).await;
        assert_eq!(status, "failed", "{page}");
        assert_eq!(reason.as_deref(), Some(why), "{page}");
        assert_eq!(checks["ok"], false, "{page}");
        assert_eq!(checks["problem"], why, "{page}");
        assert_eq!(
            notice(
                &app,
                maker_id,
                &format!("Game {version} couldn't be checked")
            )
            .await,
            why
        );
    }
    let (_, _, checks) = checks_of(
        sqlx::query_scalar::<_, Uuid>("select id from releases where version = '2.0.3'")
            .fetch_one(&app.db)
            .await
            .unwrap()
            .to_string(),
    )
    .await;
    assert_eq!(checks["final_address"], "http://localhost:8080/");
    assert_eq!(checks["redirects"], 1);
    let mine = releases_for(&app, Some(&maker)).await;
    let failed = mine
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == "2.0.0")
        .unwrap();
    assert_eq!(failed["progress"]["stage"], "failed");
    assert!(
        failed["progress"]["summary"]
            .as_str()
            .unwrap()
            .starts_with("SLAPorSLOP couldn't check the play address: the page didn't load")
    );

    // A list naming where it leads, not the address itself: found by the check.
    let fake = url::Url::parse(&app.fake_base).unwrap();
    sqlx::query(
        "insert into bad_links (url, host, source) values ($1, 'unrelated.example', 'URLhaus')",
    )
    .bind(format!(
        "{}:{}/octo/game/play",
        fake.host_str().unwrap(),
        fake.port().unwrap()
    ))
    .execute(&app.db)
    .await
    .unwrap();
    let id = submit_play_and_check(&app, &maker, "3.0.0", &play_page(&app, "play-moved")).await;
    let (status, reason, checks) = checks_of(id).await;
    assert_eq!(status, "failed");
    assert_eq!(
        reason.unwrap(),
        format!("the address it leads to ({page}) is on URLhaus, a list of known-bad links")
    );
    assert_eq!(
        checks["bad_links"],
        json!({ "listed_by": "URLhaus", "lists_loaded": true })
    );
}

/// Play addresses are checked before files that have waited longer: their checks take seconds,
/// a file's scan minutes.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn play_addresses_are_checked_before_files_waiting_longer(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "game-windows.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms, watch_releases)
         values ($1, 'orbit', 'Orbit', 'A game', 'game', 'beta', $2, $3, 'MIT', '{browser}', false)",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .bind(format!("{}octo/game", app.fake_base))
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/orbit/releases")
                .cookie(&maker)
                .json(play_body("1.0.0", &play_page(&app, "play"))),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let statuses = || async {
        sqlx::query_as::<_, (String, String)>(
            "select a.slug, r.status::text from releases r join apps a on a.id = r.app_id order by a.slug",
        )
        .fetch_all(&app.db)
        .await
        .unwrap()
    };
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(
        statuses().await,
        [
            ("game".to_string(), "fetching".to_string()),
            ("orbit".to_string(), "awaiting_review".to_string())
        ]
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(statuses().await[0].1, "awaiting_review");
}

// ---------------------------------------------------------------------------------------------
// Approving, and where Play goes

/// A person approves a play address once its checks passed and no list names it, with no key
/// and nothing signed: there's still nothing to install, the store page takes its rating, and
/// the store page says where Play goes.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_play_address_is_approved_with_nothing_signed(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "beta").await;
    let address = play_page(&app, "play");
    let mut body = play_body("1.0.0", &address);
    body["rating"] = json!({ "min_age": 16, "content_labels": ["violence"] });
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let release_id: Uuid = id.parse().unwrap();
    assert!(
        admin::approve(&app.db, None, lax(), release_id, "rev")
            .await
            .is_err(),
        "not checked yet"
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let manifest = || async {
        app.send(req(Method::GET, "/v1/apps/game/manifest").cookie(&maker))
            .await
            .status()
    };
    assert_eq!(manifest().await, StatusCode::NOT_FOUND);

    // Not when its checks didn't pass, or once a list names it.
    sqlx::query(
        "update releases set address_checks = jsonb_set(address_checks, '{ok}', 'false') where id = $1",
    )
    .bind(release_id)
    .execute(&app.db)
    .await
    .unwrap();
    let err = admin::approve(&app.db, None, lax(), release_id, "rev")
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("didn't pass its checks"),
        "{err:#}"
    );
    sqlx::query(
        "update releases set address_checks = jsonb_set(address_checks, '{ok}', 'true') where id = $1",
    )
    .bind(release_id)
    .execute(&app.db)
    .await
    .unwrap();
    let fake = url::Url::parse(&app.fake_base).unwrap();
    sqlx::query("insert into bad_links (url, host, source) values ('x', $1, 'URLhaus')")
        .bind(fake.host_str().unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    let err = admin::approve(&app.db, None, lax(), release_id, "rev")
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("known-bad links"), "{err:#}");
    sqlx::query("delete from bad_links")
        .execute(&app.db)
        .await
        .unwrap();

    // No signing key needed.
    assert_eq!(approve(&app, &id).await, admin::Approval::PlayAddress);
    let (status, signed, by): (String, bool, Option<String>) = sqlx::query_as(
        "select status::text, signed_manifest is not null, reviewed_by from releases where id = $1",
    )
    .bind(release_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), signed, by.as_deref()),
        ("approved", false, Some("rev"))
    );
    let (min_age, labels): (i16, Vec<String>) =
        sqlx::query_as("select min_age, content_labels from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((min_age, labels), (16, vec!["violence".to_string()]));
    let audit: (String, Value) =
        sqlx::query_as("select action, detail from audit_log order by id desc limit 1")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(audit.0, "release.approve");
    assert_eq!(audit.1["play_address"], address);
    assert!(audit.1.get("key_id").is_none());
    assert_eq!(
        notice(&app, maker_id, "Game 1.0.0 is approved").await,
        "A reviewer approved its play address. Play opens it now."
    );
    let mine = releases_for(&app, Some(&maker)).await;
    assert_eq!(mine[0]["progress"]["stage"], "approved");
    assert_eq!(
        mine[0]["progress"]["summary"],
        "Approved by a person at SLAPorSLOP: Play opens this address."
    );

    // Still nothing to install: a 404, never an error.
    assert_eq!(manifest().await, StatusCode::NOT_FOUND);
    assert_eq!(page_for(&app, Some(&maker)).await["play_address"], address);
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    assert_eq!(page_for(&app, None).await["play_address"], address);
    let updates = body_json(
        app.send(req(Method::GET, "/v1/library/updates?apps=game").header("x-country", "US"))
            .await,
    )
    .await;
    assert!(updates[0]["latest"].is_null(), "{updates}");
    assert_eq!(updates[0]["mods"], json!([]));
    assert_eq!(updates[0]["sandbox_suggested"], false);
}

/// A browser game moves to Beta once a person approved its address. Until then, and while it's
/// Coming soon, nobody but its maker gets the address.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_browser_game_moves_on_once_its_address_is_approved(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_browser_app(&app, "coming_soon").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    let address = play_page(&app, "play");
    let id = submit_play_and_check(&app, &maker, "0.1.0", &address).await;
    let overview = || async {
        body_json(
            app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
                .await,
        )
        .await
    };
    let before = overview().await;
    assert_eq!(before["can_advance"], false);
    assert!(before["serving"].is_null());
    assert_eq!(before["waiting"].as_array().unwrap().len(), 1);
    let to_beta = || {
        req(Method::POST, "/v1/apps/game/stage")
            .cookie(&maker)
            .json(json!({ "stage": "beta" }))
    };
    assert_eq!(app.send(to_beta()).await.status(), StatusCode::CONFLICT);
    assert!(
        admin::set_stage(&app.db, "game", slop_api::catalog::AppStage::Beta, "rev")
            .await
            .is_err()
    );
    for viewer in [None, Some(stranger.as_str()), Some(maker.as_str())] {
        assert!(
            page_for(&app, viewer).await["play_address"].is_null(),
            "pending"
        );
    }

    approve(&app, &id).await;
    // Coming soon: only its maker plays it yet.
    assert!(page_for(&app, None).await["play_address"].is_null());
    assert!(page_for(&app, Some(&stranger)).await["play_address"].is_null());
    assert_eq!(page_for(&app, Some(&maker)).await["play_address"], address);
    let public = releases_for(&app, Some(&stranger)).await;
    assert_eq!(public.as_array().unwrap().len(), 1);
    assert!(public[0]["play_address"].is_null());
    assert!(public[0]["address_checks"].is_null());
    assert_eq!(
        releases_for(&app, Some(&maker)).await[0]["play_address"],
        address
    );
    let after = overview().await;
    assert_eq!(after["can_advance"], true);
    assert_eq!(after["serving"]["play_address"], address);
    assert_eq!(after["approved"][0]["play_address"], address);

    assert_eq!(app.send(to_beta()).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(page_for(&app, None).await["play_address"], address);
    let public = releases_for(&app, None).await;
    assert_eq!(public[0]["play_address"], address);
    assert_eq!(public[0]["address_checks"]["ok"], true);
}

/// Only the address Play opens is shown to anyone but the maker: never an older, waiting or
/// turned-down one, and a draft's is the maker's alone.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn play_addresses_show_only_where_play_goes(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_browser_app(&app, "beta").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    approved_play_address(&app, "game", "1.0.0", "https://old-site.example/play/").await;
    for path in ["/v1/apps/game", "/v1/apps/game/releases"] {
        assert_eq!(
            app.send(req(Method::GET, path).cookie(&stranger))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "a draft: {path}"
        );
    }
    assert_eq!(
        page_for(&app, Some(&maker)).await["play_address"],
        "https://old-site.example/play/"
    );

    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    approved_play_address(&app, "game", "1.1.0", "https://games.example.com/play/").await;
    address_release(
        &app,
        "1.2.0",
        "awaiting_review",
        "https://next-site.example/",
    )
    .await;
    address_release(&app, "1.3.0", "rejected", "https://phish.example/").await;
    for viewer in [None, Some(stranger.as_str())] {
        assert_eq!(
            page_for(&app, viewer).await["play_address"],
            "https://games.example.com/play/"
        );
        let public = releases_for(&app, viewer).await;
        let versions: Vec<&str> = public
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["version"].as_str().unwrap())
            .collect();
        assert_eq!(versions, ["1.1.0", "1.0.0"], "approved ones only");
        assert_eq!(public[0]["play_address"], "https://games.example.com/play/");
        assert!(public[1]["play_address"].is_null(), "an older address");
        assert!(public[1]["address_checks"].is_null());
        let text = public.to_string();
        for hidden in ["old-site", "next-site", "phish"] {
            assert!(!text.contains(hidden), "{hidden}");
        }
    }
    let mine = releases_for(&app, Some(&maker)).await;
    assert_eq!(mine.as_array().unwrap().len(), 4, "the maker sees them all");
    assert_eq!(mine[0]["play_address"], "https://phish.example/");
    assert_eq!(
        page_for(&app, Some(&maker)).await["play_address"],
        "https://games.example.com/play/"
    );
}

/// Once a person approved an address, a release with the same one goes through on its own; a new
/// (or earlier) address always waits for a person, and Play keeps opening the approved one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_new_play_address_always_waits_for_a_person(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "beta").await;
    let (a, b) = (play_page(&app, "play"), play_page(&app, "play-v2"));
    let first = submit_play_and_check(&app, &maker, "1.0.0", &a).await;
    assert_eq!(release_status(&app, &first).await.0, "awaiting_review");
    approve(&app, &first).await;

    let same = submit_play_and_check(&app, &maker, "1.1.0", &a).await;
    assert_eq!(
        release_status(&app, &same).await,
        ("approved".into(), None, Some("slop".into()))
    );
    assert_eq!(
        notice(&app, maker_id, "Game 1.1.0 is approved").await,
        "A person approved its play address before, and it passed every check, so it was approved automatically. Play opens it now."
    );
    let mine = releases_for(&app, Some(&maker)).await;
    assert_eq!(mine[0]["auto_approved"], true);
    assert_eq!(
        mine[0]["progress"]["summary"],
        "Approved automatically: a person approved this play address before. Play opens this address."
    );
    assert_eq!(
        mine[1]["progress"]["summary"],
        "Approved by a person at SLAPorSLOP; it's no longer the address Play opens."
    );

    let new = submit_play_and_check(&app, &maker, "1.2.0", &b).await;
    assert_eq!(
        release_status(&app, &new).await,
        (
            "awaiting_review".into(),
            Some("Waiting for a person: a new play address is always checked by a person".into()),
            None
        )
    );
    assert_eq!(page_for(&app, Some(&maker)).await["play_address"], a);
    approve(&app, &new).await;
    assert_eq!(page_for(&app, Some(&maker)).await["play_address"], b);

    let back = submit_play_and_check(&app, &maker, "1.3.0", &a).await;
    assert_eq!(release_status(&app, &back).await.0, "awaiting_review");
    assert!(
        release_status(&app, &back)
            .await
            .1
            .unwrap()
            .contains("new play address")
    );
}

/// Withdrawing the address Play opens stops Play: it doesn't fall back to an older address,
/// which is often the same site. It isn't on the signed revocation list (nothing was signed or
/// installed), and a person approving an address again brings Play back.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn withdrawing_the_address_play_opens_stops_play(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let a = play_page(&app, "play");
    let first = approved_play_address(&app, "game", "1.0.0", &a).await;
    let second =
        approved_play_address(&app, "game", "1.1.0", "https://games.example.com/play/").await;
    assert_eq!(
        page_for(&app, None).await["play_address"],
        "https://games.example.com/play/"
    );

    admin::revoke(&app.db, second, "rev", "The site now asks for passwords")
        .await
        .unwrap();
    assert!(
        page_for(&app, None).await["play_address"].is_null(),
        "not the older one"
    );
    assert!(page_for(&app, Some(&maker)).await["play_address"].is_null());
    assert_eq!(
        notice(&app, maker_id, "Game 1.1.0 was withdrawn").await,
        "Play no longer opens this address. Why: The site now asks for passwords"
    );
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    assert!(overview["serving"].is_null(), "{overview}");
    assert_eq!(overview["approved"][0]["id"], first.to_string());
    let public = releases_for(&app, None).await;
    assert_eq!(public.as_array().unwrap().len(), 1);
    assert!(public[0]["play_address"].is_null());
    let mine = releases_for(&app, Some(&maker)).await;
    assert_eq!(mine[0]["progress"]["stage"], "revoked");
    assert_eq!(
        mine[0]["progress"]["summary"],
        "Withdrawn by SLAPorSLOP: The site now asks for passwords. Play no longer opens this address."
    );
    let signed: slop_manifest::Signed = serde_json::from_value(
        body_json(app.send(req(Method::GET, "/v1/revocations")).await).await,
    )
    .unwrap();
    let list: slop_manifest::RevocationList = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::RevocationList,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert!(list.revoked.is_empty(), "nothing signed to revoke");

    let again = submit_play_and_check(&app, &maker, "1.2.0", &a).await;
    let (status, reason, _) = release_status(&app, &again).await;
    assert_eq!(status, "awaiting_review");
    assert!(reason.unwrap().contains("withdrawn"));
    approve(&app, &again).await;
    assert_eq!(page_for(&app, None).await["play_address"], a);
}

// ---------------------------------------------------------------------------------------------
// What only downloads have

/// A browser game isn't watched for GitHub releases, picked from them, built by Slop Build, held
/// at an older release, demoed, modded or branched, run in sandbox mode or synced; a malware
/// report goes to a person rather than to a re-scan that would never come.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn what_only_downloads_have_is_refused_for_browser_games(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.demos.play_url = Some("http://play.localhost:8080".parse().unwrap())
    })
    .await;
    let (maker_id, maker) = github_app(&app).await;
    sqlx::query(
        "update apps set platforms = '{browser}', status = 'listed', stage = 'released', stage_entered_at = now(),
                         online = '{own_servers}', online_hosts = '{play.example.com}'
         where slug = 'game'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let held =
        approved_play_address(&app, "game", "1.0.0", "https://games.example.com/play/").await;
    let (_, pat) = app.user("Pat", "US").await;
    let refused = |res: Response| async move {
        let (status, _, message) = refusal(res).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        message
    };

    // Release watching: can't be turned on, and the watcher never looks, whatever the setting.
    let settings = |body: Value| {
        req(Method::PUT, "/v1/apps/game/update-settings")
            .cookie(&maker)
            .json(body)
    };
    let message = refused(app.send(settings(json!({ "watch_releases": true }))).await).await;
    assert!(
        message.contains("play address on the Releases page"),
        "{message}"
    );
    let res = app.send(settings(json!({ "auto_update": true }))).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        body_json(res).await["watch_note"]
            .as_str()
            .unwrap()
            .contains("nothing to watch")
    );
    sqlx::query("update apps set watch_releases = true")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 0);
    let looked: bool = sqlx::query_scalar("select watch_checked_at is not null from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!looked);

    // GitHub's release picker.
    refused(
        app.send(req(Method::GET, "/v1/apps/game/github-releases").cookie(&maker))
            .await,
    )
    .await;
    refused(
        app.send(
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .cookie(&maker)
                .json(json!({ "tag": "v1.1.0", "files": [] })),
        )
        .await,
    )
    .await;

    // Slop Build: a run is refused, and its setup says why.
    let message =
        refused(submit_ci(&app, &github_token(&app, json!({})), ci_release()).await).await;
    assert!(message.contains("Slop Build makes downloads"), "{message}");
    let setup = body_json(
        app.send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(setup["supported"], false);
    assert!(
        setup["note"]
            .as_str()
            .unwrap()
            .contains("give its play address on the Releases page"),
        "{setup}"
    );
    assert!(setup["create_url"].is_null());

    // Holds: Play always opens the newest approved address.
    for release_id in [json!(held), Value::Null] {
        let message = refused(
            request_change(
                &app,
                &maker,
                json!({ "kind": "release", "release_id": release_id }),
            )
            .await,
        )
        .await;
        assert!(message.contains("newest approved address"), "{message}");
    }

    // Demos.
    let status = body_json(
        app.send(req(Method::GET, "/v1/apps/game/demo").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(status["eligible"], false);
    let message =
        refused(
            app.send(req(Method::POST, "/v1/apps/game/demo").cookie(&maker).json(
                json!({ "url": "https://github.com/octo/game/releases/download/v1/web.zip" }),
            ))
            .await,
        )
        .await;
    assert!(
        message.contains("already plays in the browser"),
        "{message}"
    );

    // Mods and community builds.
    let (_, author) = app.user("Bo Brancher", "US").await;
    let message = refused(
        app.send(
            req(Method::POST, "/v1/apps/game/branches").cookie(&author).json(json!({
                "slug": "game-dragons", "title": "Dragons", "summary": "Three dragons.",
                "repo_url": "https://github.com/bo/game-dragons", "license_spdx": "MIT", "kind": "addon",
                "note": "Three dragons."
            })),
        )
        .await,
    )
    .await;
    assert!(
        message.contains("can't have community builds or mods"),
        "{message}"
    );

    // Sandbox mode and synced saves.
    let message = refused(
        app.send(req(Method::PUT, "/v1/apps/game/sandbox-report").cookie(&pat))
            .await,
    )
    .await;
    assert!(message.contains("no sandbox mode"), "{message}");
    let page = page_for(&app, Some(&pat)).await;
    assert_eq!(
        page["sandbox_suggested"], false,
        "it goes online, but there's no sandbox"
    );
    assert_eq!(page["sandbox_ready"], false);
    let save = Request::builder()
        .method(Method::PUT)
        .uri("/v1/me/saves/game")
        .header(header::HOST, "localhost:8080")
        .header(header::COOKIE, &pat)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header("x-slop-base", "0")
        .header("x-slop-device", "d-1")
        .body(Body::from(b"save".to_vec()))
        .unwrap();
    let res = app.router.clone().oneshot(save).await.unwrap();
    let message = refused(res).await;
    assert!(message.contains("saves"), "{message}");

    // A malware report: a person looks; no re-scan request is left that nothing would take up.
    let res = app
        .send(req(Method::POST, "/v1/reports").cookie(&pat).json(json!({
            "kind": "app", "subject": "game", "reason": "malware",
            "details": "The page asks to download an .exe.", "good_faith": true
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(count(&app, "select count(*) from rescan_requests").await, 0);
    assert_eq!(count(&app, "select count(*) from releases").await, 1);
    let _ = maker_id;
}

/// The hourly abuse checks match a browser game's play addresses (the one Play opens, ones
/// waiting, and where each was found to lead) against the bad-link lists, once each; an older
/// address Play no longer opens isn't.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn listed_play_addresses_are_pointed_out(db: PgPool) {
    let app = test_app(db).await;
    maker_with_browser_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    approved_play_address(&app, "game", "0.9.0", "https://old-site.example/").await;
    approved_play_address(&app, "game", "1.0.0", "https://games.example.com/play/").await;
    let pending = address_release(
        &app,
        "1.1.0",
        "awaiting_review",
        "https://new-site.example/play/",
    )
    .await;
    sqlx::query(
        "update releases set address_checks = jsonb_build_object('ok', true, 'final_address', 'https://landing.example/x')
         where id = $1",
    )
    .bind(pending)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into bad_links (url, host, source) values
             ('games.example.com/elsewhere', 'games.example.com', 'URLhaus'),
             ('landing.example/x', 'unrelated.example', 'OpenPhish'),
             ('old-site.example', 'old-site.example', 'URLhaus')",
    )
    .execute(&app.db)
    .await
    .unwrap();

    slop_api::abuse::run_once(&app.state).await.unwrap();
    let mut found: Vec<(String, String, String, bool)> = sqlx::query_as(
        "select detail->>'link', detail->>'listed_by', detail->>'version', (detail->>'plays_now')::boolean
         from abuse_signals where kind = 'bad_link' and (detail->>'play_address')::boolean",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    found.sort();
    assert_eq!(
        found,
        [
            (
                "https://games.example.com/play/".to_string(),
                "URLhaus".to_string(),
                "1.0.0".to_string(),
                true
            ),
            (
                "https://landing.example/x".to_string(),
                "OpenPhish".to_string(),
                "1.1.0".to_string(),
                false
            ),
        ]
    );
    assert_eq!(
        slop_api::abuse::run_once(&app.state).await.unwrap(),
        0,
        "raised once"
    );
}

// ---------------------------------------------------------------------------------------------
// Playing

/// A listed browser game `game` in `stage`, whose address a person approved (version 1.0.0).
/// Returns its maker's (ID, cookie) and that release's ID.
async fn listed_browser_game(app: &TestApp, stage: &str, address: &str) -> (Uuid, String, Uuid) {
    let (maker_id, maker) = maker_with_browser_app(app, stage).await;
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let release = approved_play_address(app, "game", "1.0.0", address).await;
    (maker_id, maker, release)
}

async fn launch(app: &TestApp, cookie: &str, slug: &str, query: &str) -> Response {
    app.send(req(Method::POST, format!("/v1/apps/{slug}/launch{query}")).cookie(cookie))
        .await
}

/// (days launched, last platform) of `user`'s use of `slug`, once they've launched it.
async fn usage(app: &TestApp, user: Uuid, slug: &str) -> Option<(i32, Option<String>)> {
    sqlx::query_as(
        "select u.days_launched, u.last_platform from user_app_usage u join apps a on a.id = u.app_id
         where u.user_id = $1 and a.slug = $2",
    )
    .bind(user)
    .bind(slug)
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

/// Play records the day a browser game was played, as played in the browser whatever the client
/// says, and hands out no Pulse token (its site can't be given one). With nothing to play (no
/// approved address, Coming soon for anyone but its maker, or withdrawn) it says so and records
/// nothing. Downloads launch as before.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn playing_a_browser_game_records_the_day_and_hands_out_no_pulse_token(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_browser_app(&app, "coming_soon").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let nothing_to_play = (
        StatusCode::CONFLICT,
        String::new(),
        "nothing to play yet".to_string(),
    );

    // A draft: nobody else can play it, or find out it's there.
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    // No approved address yet, only one waiting for review.
    assert_eq!(
        refusal(launch(&app, &pat, "game", "?platform=windows").await).await,
        nothing_to_play
    );
    address_release(
        &app,
        "1.1.0",
        "awaiting_review",
        "https://next-site.example/",
    )
    .await;
    assert_eq!(
        refusal(launch(&app, &pat, "game", "").await).await,
        nothing_to_play
    );
    let address = "https://games.example.com/play/";
    let approved = approved_play_address(&app, "game", "1.0.0", address).await;

    // Coming soon: only its maker plays it yet.
    assert_eq!(
        refusal(launch(&app, &pat, "game", "").await).await,
        nothing_to_play
    );
    assert_eq!(usage(&app, pat_id, "game").await, None, "nothing recorded");
    let res = launch(&app, &maker, "game", "").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["play_address"], address);

    // In Beta anyone who sees it plays it: the day is recorded as played in the browser, with
    // no Pulse token although Pat shares playtime.
    sqlx::query("update apps set stage = 'beta' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = launch(&app, &pat, "game", "?platform=windows").await;
    assert_eq!(res.status(), StatusCode::OK);
    let launched = body_json(res).await;
    assert_eq!(launched["play_address"], address);
    assert!(launched["token"].is_null(), "{launched}");
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((1, Some("browser".into())))
    );
    // Once a day, however often it's played.
    launch(&app, &pat, "game", "?platform=linux").await;
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((1, Some("browser".into())))
    );
    sqlx::query("update user_app_usage set last_launch_day = current_date - 1 where user_id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    launch(&app, &pat, "game", "").await;
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((2, Some("browser".into())))
    );
    assert_eq!(count(&app, "select count(*) from pulse_sessions").await, 0);
    // Signed out, the store page's Play opens the site and nothing is recorded.
    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/game/launch"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // Withdrawn: nothing to play again; the days played stay.
    admin::revoke(&app.db, approved, "rev", "The site asks for passwords now")
        .await
        .unwrap();
    assert_eq!(
        refusal(launch(&app, &pat, "game", "").await).await,
        nothing_to_play
    );
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((2, Some("browser".into())))
    );

    // A download launches as before: never recorded as browser, and with a Pulse token.
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    let res = launch(&app, &pat, "castle", "?platform=browser").await;
    assert_eq!(res.status(), StatusCode::OK);
    let launched = body_json(res).await;
    assert!(launched["token"].is_string(), "{launched}");
    assert!(launched["play_address"].is_null());
    assert_eq!(usage(&app, pat_id, "castle").await, Some((1, None)));
    launch(&app, &pat, "castle", "?platform=linux").await;
    assert_eq!(
        usage(&app, pat_id, "castle").await,
        Some((1, Some("linux".into())))
    );
}

/// Only players vote on a browser game too, and its page says how to become one. Its votes are
/// weighted by the days it was played (it has no Pulse playtime), and its maker sees them under
/// the browser and the address's version.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn browser_players_vote_by_the_days_they_played(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker, _) =
        listed_browser_game(&app, "beta", "https://games.example.com/play/").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let vote = |slug: &str, v: &str| {
        req(Method::POST, format!("/v1/apps/{slug}/vote"))
            .cookie(&pat)
            .json(json!({ "vote": v }))
    };
    let why = "Play it from SLAPorSLOP first: only players can vote.";
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/game/ratings").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(ratings["mine"]["can_vote"], false);
    assert_eq!(ratings["mine"]["reason"], why);
    let (status, _, message) = refusal(app.send(vote("game", "not_slop")).await).await;
    assert_eq!((status, message.as_str()), (StatusCode::CONFLICT, why));
    // A download still says to launch it from the app.
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/castle/ratings").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(
        ratings["mine"]["reason"],
        "Launch it from the SLAPorSLOP app first: only players can vote."
    );

    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    sqlx::query("update user_app_usage set days_launched = 3 where user_id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(vote("game", "not_slop")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["mine"]["vote"], "not_slop");
    let event: (f32, Option<String>, Option<String>, i64) = sqlx::query_as(
        "select weight, platform, release_version, playtime_seconds from vote_events where user_id = $1",
    )
    .bind(pat_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        event,
        (
            slop_api::votes::vote_weight(0, 3),
            Some("browser".into()),
            Some("1.0.0".into()),
            0
        )
    );
    let breakdown = body_json(
        app.send(req(Method::GET, "/v1/apps/game/ratings/breakdown").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(breakdown["by_platform"][0]["key"], "browser", "{breakdown}");
    assert_eq!(breakdown["by_platform"][0]["not_slop"], 1);
    assert_eq!(breakdown["by_version"][0]["key"], "1.0.0");
}

/// A bug report on a browser game says it was played in the browser, whatever was sent, on the
/// version Play opens unless the player names one, and carries no game output (nothing of it
/// runs in the SLAPorSLOP app). Downloads' reports don't take "browser".
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bug_reports_on_a_browser_game_say_where_it_was_played(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _, _) =
        listed_browser_game(&app, "released", "https://games.example.com/play/").await;
    let (_, pat) = app.user("Pat", "US").await;
    let file = |slug: &str, body: Value| {
        req(Method::POST, format!("/v1/apps/{slug}/bug-reports"))
            .cookie(&pat)
            .json(body)
    };
    let report = json!({
        "title": "The ball goes through walls", "body": "On level 3, every time.",
        "platform": "windows", "log": "panic: wall missing"
    });
    let (status, _, message) = refusal(app.send(file("game", report.clone())).await).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("play it from SLAPorSLOP first"),
        "{message}"
    );

    launch(&app, &pat, "game", "").await;
    let res = app.send(file("game", report.clone())).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let filed = body_json(res).await;
    assert_eq!(
        (&filed["platform"], &filed["version"]),
        (&json!("browser"), &json!("1.0.0"))
    );
    let mut named = report.clone();
    named["version"] = json!("0.9.0");
    named.as_object_mut().unwrap().remove("platform");
    let res = app.send(file("game", named)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let filed = body_json(res).await;
    assert_eq!(
        (&filed["platform"], &filed["version"]),
        (&json!("browser"), &json!("0.9.0"))
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from bug_reports where log is not null"
        )
        .await,
        0,
        "no game output from a site"
    );

    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    launch(&app, &pat, "castle", "?platform=windows").await;
    let mut on_castle = report.clone();
    on_castle["platform"] = json!("browser");
    let (status, field, _) = refusal(app.send(file("castle", on_castle)).await).await;
    assert_eq!(
        (status, field.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "platform")
    );
    let res = app.send(file("castle", report)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let filed = body_json(res).await;
    assert_eq!(filed["platform"], "windows");
    let log: Option<String> =
        sqlx::query_scalar("select log from bug_reports where id = $1::text::uuid")
            .bind(filed["id"].as_str().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(log.as_deref(), Some("panic: wall missing"));
}

/// A browser game goes in the library like any game, which says it plays in the browser but not
/// where (Play reads that from its store page). It never has an update to install or queue.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_library_keeps_browser_games_without_their_address(db: PgPool) {
    let app = test_app(db).await;
    let address = "https://games.example.com/play/";
    let (maker_id, _, _) = listed_browser_game(&app, "beta", address).await;
    let (_, pat) = app.user("Pat", "US").await;
    listed_with(
        &app,
        maker_id,
        "secret-tap",
        "beta",
        "status = 'draft', platforms = '{browser}'",
    )
    .await;
    let add = |slug: &str| req(Method::PUT, format!("/v1/me/library/{slug}")).cookie(&pat);
    assert_eq!(app.send(add("game")).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(add("secret-tap")).await.status(),
        StatusCode::NOT_FOUND,
        "someone else's draft"
    );
    let library = body_json(
        app.send(req(Method::GET, "/v1/me/library").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(library.as_array().unwrap().len(), 1);
    assert_eq!(library[0]["slug"], "game");
    assert_eq!(library[0]["plays_in_browser"], true);
    let text = library.to_string();
    assert!(
        !text.contains("games.example.com") && !text.contains("play_address"),
        "{text}"
    );

    // Nothing to install, even with a new address waiting for review.
    address_release(
        &app,
        "1.1.0",
        "awaiting_review",
        "https://next-site.example/",
    )
    .await;
    let updates = body_json(
        app.send(req(Method::GET, "/v1/library/updates?apps=game").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(updates.as_array().unwrap().len(), 1);
    let game = &updates[0];
    assert!(game["latest"].is_null(), "{game}");
    assert!(game["pending"].is_null(), "{game}");
    assert_eq!((&game["builds"], &game["mods"]), (&json!([]), &json!([])));
    assert_eq!(game["sandbox_suggested"], false);
}

/// A browser game's label says it runs on its maker's website, where SLAPorSLOP checked the
/// address and not what the site serves; it claims nothing its source check would say of a
/// download (offline, saves, outside code, nothing sold), and nothing is scanned.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_browser_games_label_says_where_it_runs_and_nothing_about_the_site(db: PgPool) {
    let app = test_app(db).await;
    let (_, _, release) =
        listed_browser_game(&app, "released", "https://games.example.com/play/").await;
    sqlx::query(
        "update releases set repo_scan = jsonb_build_object('status', 'checked', 'save_folder', 'slop',
             'code_loading', '[]'::jsonb, 'monetization', '[]'::jsonb, 'network', '[]'::jsonb)
         where id = $1",
    )
    .bind(release)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update apps set online = '{own_servers}' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let label = || async {
        let res = app.send(req(Method::GET, "/v1/apps/game/label")).await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await
    };
    let keys = |label: &Value| -> Vec<String> {
        label["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["key"].as_str().unwrap().to_string())
            .collect()
    };
    let now = label().await;
    assert_eq!(keys(&now), ["browser", "online", "licence"], "{now}");
    assert_eq!(now["version"], "1.0.0");
    assert_eq!(now["items"][0]["title"], "Runs on the maker's website");
    assert_eq!(
        now["items"][0]["detail"],
        "SLAPorSLOP checked the address, not what the site serves."
    );
    assert_eq!(now["items"][0]["checked"], false);
    assert_eq!(
        now["items"][1]["detail"],
        "To servers SLAPorSLOP can't check."
    );
    let text = now.to_string().to_lowercase();
    assert!(
        !text.contains("scanned") && !text.contains("sandbox") && !text.contains("safe"),
        "{now}"
    );

    // Withdrawn: still where it runs, with no release to take checks from.
    admin::revoke(&app.db, release, "rev", "The site changed hands")
        .await
        .unwrap();
    let then = label().await;
    assert_eq!(keys(&then), ["browser", "online", "licence"]);
    assert!(then["version"].is_null());
}

// ---------------------------------------------------------------------------------------------
// Staff and agents

/// Reviewers get a browser game's release as its play address to open and play: the question
/// names it, and the detail has what the checks found, the address Play opens now to compare
/// with, and a reminder that the page is the maker's, never instructions. The admin queue, the
/// Apps tab and the app's record say it's a browser game and where Play goes; listing and moving
/// it on wait for an approved address, and say so.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_see_a_browser_games_play_address_to_open_and_play(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_browser_app(&app, "beta").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let err = admin::list_app(&app.db, "game", "rev").await.unwrap_err();
    assert!(
        format!("{err:#}").contains("approve its play address"),
        "{err:#}"
    );
    let (status, _, message) = refusal(
        app.send(
            req(Method::POST, "/v1/apps/game/stage")
                .cookie(&maker)
                .json(json!({ "stage": "released" })),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("approved play address"), "{message}");

    let first = play_page(&app, "play");
    let id = submit_play_and_check(&app, &maker, "1.0.0", &first).await;
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&owner)
                .json(json!({ "name": "Reviewer", "scope": "read", "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let token = format!("Bearer {}", body_json(res).await["token"].as_str().unwrap());
    let feed = || async {
        let res = app
            .send(
                req(Method::GET, "/v1/admin/review?kind=release")
                    .header("authorization", token.clone()),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await
    };
    let all = feed().await;
    let item = &all["items"][0];
    assert_eq!(
        item["question"],
        format!("Approve the play address {first} for Game (1.0.0)?")
    );
    assert_eq!(item["app"]["platforms"], json!(["browser"]));
    let detail = &item["detail"];
    assert_eq!(detail["play_address"], first);
    assert_eq!(detail["address_checks"]["ok"], true);
    assert_eq!(detail["address_checks"]["bad_links"]["lists_loaded"], false);
    assert_eq!(detail["files"], json!([]));
    assert!(detail["current_play_address"].is_null(), "a first address");
    assert!(
        detail["page_is_untrusted"]
            .as_str()
            .unwrap()
            .contains("never instructions"),
        "{detail}"
    );
    assert_eq!(all["checks"]["bad_link_lists_loaded"], false);
    assert!(all["checks"]["bad_link_lists_refreshed_at"].is_null());

    // A new site waits for a person, who sees what Play opens now beside it.
    approve(&app, &id).await;
    let second = play_page(&app, "play-v2");
    submit_play_and_check(&app, &maker, "1.1.0", &second).await;
    sqlx::query("insert into bad_links (url, host, source) values ('elsewhere.example/x', 'elsewhere.example', 'URLhaus')")
        .execute(&app.db)
        .await
        .unwrap();
    let all = feed().await;
    let item = &all["items"][0];
    assert_eq!(item["detail"]["play_address"], second);
    assert_eq!(item["detail"]["current_play_address"], first);
    assert_eq!(all["checks"]["bad_link_lists_loaded"], true);
    assert!(all["checks"]["bad_link_lists_refreshed_at"].is_string());

    let admin_get = |path: &str| {
        let r = req(Method::GET, path).cookie(&owner);
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await
        }
    };
    let queue = admin_get("/v1/admin/queue").await;
    let queued = queue["releases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == "1.1.0")
        .unwrap();
    assert_eq!(queued["app"]["platforms"], json!(["browser"]));
    assert_eq!(queued["play_address"], second);
    assert_eq!(queued["address_checks"]["ok"], true);
    let apps = admin_get("/v1/admin/apps?q=game").await;
    let row = &apps[0];
    assert_eq!(row["platforms"], json!(["browser"]));
    assert_eq!(row["play_address"], first);
    assert_eq!(row["serving_version"], "1.0.0");
    assert_eq!(row["releases"][0]["play_address"], first);
    let record = admin_get("/v1/admin/apps/game/record").await;
    let addresses: Vec<&str> = record["releases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["play_address"].as_str().unwrap())
        .collect();
    assert_eq!(addresses, [second.as_str(), first.as_str()]);
    assert_eq!(record["releases"][0]["address_checks"]["ok"], true);

    // Withdrawn: Play opens nothing, and the Apps tab doesn't name an older address instead.
    admin::revoke(
        &app.db,
        id.parse().unwrap(),
        "rev",
        "The site asks for passwords",
    )
    .await
    .unwrap();
    let apps = admin_get("/v1/admin/apps?q=game").await;
    assert!(apps[0]["play_address"].is_null(), "{}", apps[0]);
    assert!(apps[0]["serving_version"].is_null());
}

/// Reports on a browser game are offered in its own words where a download's don't fit, but only
/// to who can see it: anyone else gets the usual words, so they learn nothing about a draft.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn browser_games_are_reported_in_their_own_words(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker, _) =
        listed_browser_game(&app, "released", "https://games.example.com/play/").await;
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "secret-tap",
        "beta",
        "status = 'draft', platforms = '{browser}'",
    )
    .await;
    let labels = |query: &str, cookie: Option<&str>| {
        let mut r = req(Method::GET, format!("/v1/reports/reasons?{query}"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            let reasons = body_json(res).await;
            reasons
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    (
                        r["code"].as_str().unwrap().to_string(),
                        r["label"].as_str().unwrap().to_string(),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        }
    };
    let browser = labels("kind=app&app=game", None).await;
    assert_eq!(
        browser["source_mismatch"],
        "The game on its site isn't the one in its repository"
    );
    assert_eq!(
        browser["broken"],
        "Its site doesn't load, or the game doesn't work"
    );
    assert_eq!(browser["malware"], "Malware, hidden mining or data theft");
    let usual = labels("kind=app", None).await;
    assert_eq!(
        usual["source_mismatch"],
        "The build doesn't match its source"
    );
    assert_eq!(usual["broken"], "The build is broken");
    assert_eq!(browser.len(), usual.len(), "the same reasons");
    assert_eq!(labels("kind=app&app=castle", None).await, usual);
    assert_eq!(labels("kind=app&app=nothing-here", None).await, usual);
    assert_eq!(
        labels("kind=app&app=secret-tap", None).await,
        usual,
        "a draft"
    );
    assert_eq!(
        labels("kind=app&app=secret-tap", Some(&maker)).await,
        browser
    );
    assert_eq!(
        labels("kind=review&app=game", None).await,
        labels("kind=review", None).await
    );
}

/// A maker's MCP agent gives a browser game's play address with its own tool (submit_release
/// still asks for files), and whats_left and get_fixes say what's next in its terms: give the
/// address, wait for the check and the person, give it again after a failed check.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_give_a_browser_games_play_address(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_browser_app(&app, "beta").await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Claude", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let token = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    let rpc = |method: &str, params: Value| {
        let r = req(Method::POST, "/v1/maker/mcp")
            .header("authorization", token.clone())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await
        }
    };
    let tool = |name: &str, args: Value| {
        let call = rpc("tools/call", json!({ "name": name, "arguments": args }));
        async move {
            let reply = call.await;
            (
                reply["result"]["isError"] == false,
                reply["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };
    let json_of = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
    let steps = || async {
        let (ok, text) = tool("whats_left", json!({})).await;
        assert!(ok, "{text}");
        json_of(&text)["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["app"] == "game" && s["optional"] == false)
            .map(|s| {
                (
                    s["who"].as_str().unwrap().to_string(),
                    s["what"].as_str().unwrap().to_string(),
                    s["link"].as_str().unwrap().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };

    let tools = rpc("tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    let find = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap().clone();
    let submit_play = find("submit_play_address");
    assert_eq!(
        submit_play["inputSchema"]["required"],
        json!(["slug", "play_address", "version"])
    );
    for field in ["commit_sha", "notes", "rating"] {
        assert!(
            submit_play["inputSchema"]["properties"][field].is_object(),
            "{field}"
        );
    }
    assert!(
        submit_play["description"]
            .as_str()
            .unwrap()
            .contains("checks the address, not what the site serves")
    );
    assert!(
        find("submit_release")["description"]
            .as_str()
            .unwrap()
            .contains("Windows or Linux")
    );

    // Nothing yet: give the address.
    let todo = steps().await;
    assert_eq!(todo.len(), 1, "{todo:?}");
    assert_eq!(todo[0].0, "assistant");
    assert!(
        todo[0].1.starts_with("Give Game's play address"),
        "{todo:?}"
    );
    assert!(todo[0].2.ends_with("/publish/guide#browser-games"));
    let (ok, text) = tool("get_fixes", json!({ "slug": "game" })).await;
    assert!(ok, "{text}");
    assert!(
        json_of(&text)["next"]
            .as_str()
            .unwrap()
            .contains("submit_play_address")
    );

    // Each tool asks for what it needs: files for a download, the address for a browser game.
    let (ok, text) = tool(
        "submit_release",
        json!({ "slug": "game", "version": "1.0.0", "commit_sha": "a".repeat(40) }),
    )
    .await;
    assert!(!ok && text.contains("missing argument: files"), "{text}");
    let (ok, text) = tool(
        "submit_play_address",
        json!({ "slug": "game", "version": "1.0.0" }),
    )
    .await;
    assert!(
        !ok && text.contains("missing argument: play_address"),
        "{text}"
    );
    let mut with_files = release_body(&app, "1.0.0", "game-windows.zip");
    with_files["slug"] = json!("game");
    let (ok, text) = tool("submit_release", with_files).await;
    assert!(
        !ok && text.contains("a browser game has no files"),
        "{text}"
    );

    let address = play_page(&app, "play");
    let (ok, text) = tool(
        "submit_play_address",
        json!({ "slug": "game", "play_address": address, "version": "1.0.0",
                "commit_sha": "a".repeat(40), "notes": "The first level." }),
    )
    .await;
    assert!(ok, "{text}");
    let submitted = json_of(&text);
    assert_eq!(submitted["status"], "fetching");
    assert_eq!(submitted["play_address"], address);
    assert_eq!(submitted["notes"], "The first level.");
    let todo = steps().await;
    assert_eq!(todo[0].0, "slaporslop");
    assert!(
        todo[0]
            .1
            .contains("checking the play address of Game 1.0.0"),
        "{todo:?}"
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let todo = steps().await;
    assert!(
        todo[0].1.starts_with(
            "Game 1.0.0's play address passed SLAPorSLOP's checks and waits for a person"
        ),
        "{todo:?}"
    );

    // A failed check: what went wrong, and to give the address again.
    let (ok, text) = tool(
        "submit_play_address",
        json!({ "slug": "game", "play_address": play_page(&app, "nope"), "version": "1.1.0",
                "commit_sha": "a".repeat(40) }),
    )
    .await;
    assert!(ok, "{text}");
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let todo = steps().await;
    assert_eq!(todo[0].0, "assistant");
    assert!(
        todo[0].1.contains("didn't go through") && todo[0].1.contains("submit_play_address"),
        "{todo:?}"
    );
    let (ok, text) = tool("get_fixes", json!({ "slug": "game" })).await;
    assert!(ok, "{text}");
    let fixes = json_of(&text);
    assert_eq!(fixes["fixes"][0]["kind"], "release:failed");
    assert!(
        fixes["fixes"][0]["instruction"]
            .as_str()
            .unwrap()
            .contains("submit_play_address")
    );
    assert!(
        fixes["next"]
            .as_str()
            .unwrap()
            .contains("submit_play_address")
    );

    // Moving on waits for a person's approval of an address, and says so.
    let (ok, text) = tool(
        "move_to_stage",
        json!({ "slug": "game", "stage": "released" }),
    )
    .await;
    assert!(!ok && text.contains("approved play address"), "{text}");
}

/// A maker's data export has each release's play address and what its checks found, the part of
/// a browser game they'd take elsewhere.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_export_has_their_play_addresses(db: PgPool) {
    let app = test_app(db).await;
    let address = "https://games.example.com/play/";
    let (_, maker, _) = listed_browser_game(&app, "released", address).await;
    let res = app
        .send(req(Method::GET, "/v1/me/export").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let export = body_json(res).await;
    let release = &export["app_releases"][0];
    assert_eq!(release["app"], "game");
    assert_eq!(release["play_address"], address);
    assert_eq!(release["address_checks"]["ok"], true);
    assert!(release["signed_manifest"].is_null());
}
