// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dedicated servers (DESIGN §5, migrations 0093 and 0094, docs/DEDICATED-SERVERS.md): the
//! Servers category right after Utilities, with the Dedicated Server tag. A dedicated server is
//! linked to its game by the game's maker and shown on the game's page, or (from before that) an
//! app in the Servers category. The Content Policy's rule on code from outside SLAPorSLOP doesn't
//! cover dedicated servers ("Browser games and dedicated servers"): their fixes, label and report
//! reasons say so, and everything else is as for any download.

use crate::*;

fn scan_with_code_loading() -> Value {
    json!({
        "status": "checked",
        "licence": { "declared": "MIT", "kinds": [], "problems": [] },
        "save_folder": "slop",
        "pulse": "heartbeat",
        "monetization": [],
        "network": [{ "rule": "sockets", "label": "Network sockets (TCP, UDP)", "count": 1, "examples": ["src/net.rs:3"] }],
        "code_loading": [{ "rule": "load_native", "label": "Loads native libraries at run time", "count": 1,
                           "examples": ["src/plugins.rs:40"] }],
    })
}

async fn app_id(app: &TestApp, slug: &str) -> Uuid {
    sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The Servers category comes right after Utilities, with one tag, Dedicated Server, and holds
/// apps to no age. A store page, game or app, can take the tag, and is listed under both.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn servers_is_a_category_after_utilities_with_the_dedicated_server_tag(db: PgPool) {
    let app = test_app(db).await;
    let body = body_json(app.send(req(Method::GET, "/v1/categories")).await).await;
    let categories = body.as_array().unwrap();
    let at = |slug: &str| categories.iter().position(|c| c["slug"] == slug).unwrap();
    assert_eq!(at("servers"), at("utilities") + 1);
    assert_eq!(at("learning"), at("servers") + 1);
    let servers = &categories[at("servers")];
    assert_eq!(servers["name"], "Servers");
    assert_eq!(
        servers["description"],
        "Games with a dedicated server you run on your own computer or a host"
    );
    assert_eq!(
        servers["tags"],
        json!([{ "slug": "dedicated-server", "name": "Dedicated Server" }])
    );
    assert!(servers.get("max_age").is_none());

    let (maker_id, maker) = app.user("Maker", "US").await;
    for (slug, title, kind) in [
        ("castle-server", "Castle Server", "game"),
        ("relay-server", "Relay Server", "app"),
    ] {
        let mut body = new_app(slug);
        body["title"] = json!(title);
        body["kind"] = json!(kind);
        body["tags"] = json!(["dedicated-server", "multiplayer"]);
        let res = app
            .send(req(Method::POST, "/v1/apps").cookie(&maker).json(body))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED, "{slug}");
    }
    sqlx::query("update apps set status = 'listed' where maker_id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let get =
        |q: &str| req(Method::GET, format!("/v1/apps?sort=title&{q}")).header("x-country", "US");
    assert_eq!(
        list_slugs(&app, get("category=servers")).await,
        ["castle-server", "relay-server"]
    );
    assert_eq!(
        list_slugs(&app, get("tag=dedicated-server")).await,
        ["castle-server", "relay-server"]
    );
    let page = body_json(app.send(get("category=utilities")).await).await;
    assert!(page["apps"].as_array().unwrap().is_empty());
}

/// One definition of a dedicated server (`catalog::is_dedicated_server`): an app with a tag in the
/// Servers category. Other tags, networking ones included, or none, don't make one; a tag staff
/// add to Servers later would.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_dedicated_server_is_an_app_in_the_servers_category(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    for slug in ["plain", "server", "networked", "panel"] {
        listed_with(&app, maker, slug, "released", "").await;
    }
    tag_app(&app, "server", &["dedicated-server", "multiplayer"]).await;
    tag_app(&app, "networked", &["networking", "multiplayer"]).await;
    sqlx::query(
        "insert into tags (slug, name, category_id)
         select 'server-panel', 'Server Panel', id from categories where slug = 'servers'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    tag_app(&app, "panel", &["server-panel"]).await;
    for (slug, server) in [
        ("plain", false),
        ("server", true),
        ("networked", false),
        ("panel", true),
    ] {
        let id = app_id(&app, slug).await;
        assert_eq!(
            slop_api::catalog::is_dedicated_server(&app.db, id)
                .await
                .unwrap(),
            server,
            "{slug}"
        );
    }
}

/// A dedicated server's code loading is a `should` in its fixes (say what it loads), never a
/// `must`, since the rule on code from outside SLAPorSLOP doesn't cover it; going online unsaid
/// stays a `must`, as for any download. The same release of a game that isn't a server must stop
/// loading code, and a browser game gets no code-loading fix at all. Only the maker sees them.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_dedicated_servers_fixes_dont_ask_it_to_stop_loading_code(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, repo_scan)
         select $1, a.id, '1.0.0', repeat('a', 40), 'awaiting_review', a.maker_id, $2
         from apps a where a.slug = 'game'",
    )
    .bind(Uuid::now_v7())
    .bind(scan_with_code_loading())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update releases set repo_scan = repo_scan || '{\"online_undeclared\": true}'")
        .execute(&app.db)
        .await
        .unwrap();
    let fixes = || async {
        let res = app
            .send(req(Method::GET, "/v1/apps/game/fixes").cookie(&maker))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        body["fixes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["kind"].as_str().unwrap().to_string(),
                    f["severity"].as_str().unwrap().to_string(),
                    f["guide_section"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    let fix = |kind: &str, severity: &str, guide: &str| {
        (kind.to_string(), severity.to_string(), guide.to_string())
    };
    assert_eq!(
        fixes().await,
        [
            fix("code_loading", "must", "Rules for the code"),
            fix("online_undeclared", "must", "Online play and mods")
        ]
    );

    tag_app(&app, "game", &["dedicated-server"]).await;
    assert_eq!(
        fixes().await,
        [
            fix("code_loading", "should", "Dedicated servers"),
            fix("online_undeclared", "must", "Online play and mods")
        ]
    );
    let res = app
        .send(req(Method::GET, "/v1/apps/game/fixes").cookie(&stranger))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // A browser game (tagged a server or not): nothing about code loading, and its site is on the
    // web anyway.
    sqlx::query(
        "update apps set platforms = '{browser}', watch_releases = false where slug = 'game'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        fixes().await,
        [fix("online_undeclared", "should", "Online play and mods")]
    );
}

/// A dedicated server whose source check found code loading says on its label that servers may
/// load mods, maps and plugins from other places, as SLAPorSLOP's own finding; a clean one says
/// it loads none, like any app.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_dedicated_servers_label_says_servers_may_load_code(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    for slug in ["relay", "castle"] {
        listed_with(&app, maker_id, slug, "released", "platforms = '{linux}'").await;
        approved_release(&app, slug, maker_id).await;
    }
    tag_app(&app, "relay", &["dedicated-server"]).await;
    sqlx::query("update releases set repo_scan = $1")
        .bind(scan_with_code_loading())
        .execute(&app.db)
        .await
        .unwrap();
    let outside_code = |slug: &'static str| {
        let app = &app;
        async move {
            let res = app
                .send(req(Method::GET, format!("/v1/apps/{slug}/label")))
                .await;
            assert_eq!(res.status(), StatusCode::OK);
            let label = body_json(res).await;
            label["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["key"] == "outside_code")
                .cloned()
                .unwrap_or_else(|| panic!("no outside_code item: {label}"))
        }
    };
    let relay = outside_code("relay").await;
    assert_eq!(relay["title"], "Loads code from outside SLAPorSLOP");
    assert_eq!(
        relay["detail"],
        "Dedicated servers may load mods, maps and plugins from other places."
    );
    assert_eq!(
        (&relay["checked"], &relay["tone"]),
        (&json!(true), &json!("note"))
    );
    let castle = outside_code("castle").await;
    assert_eq!(castle["title"], "Loads code from outside SLAPorSLOP");
    assert_eq!(
        castle["detail"],
        "Loads native libraries at run time. A person reviewed it before approving."
    );

    sqlx::query("update releases set repo_scan = jsonb_set(repo_scan, '{code_loading}', '[]')")
        .execute(&app.db)
        .await
        .unwrap();
    let relay = outside_code("relay").await;
    assert_eq!(relay["title"], "Loads no code from outside SLAPorSLOP");
    assert_eq!(relay["tone"], "good");
    assert!(relay.get("detail").is_none());
}

/// Players aren't offered "Runs code or mods from outside SLAPorSLOP" for an app that rule
/// doesn't cover, a dedicated server or a browser game they can see. Anyone who can't see the app
/// gets the usual reasons, so a draft's tags stay unknown; other kinds of report are unchanged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn outside_code_isnt_a_reason_to_report_a_server_or_a_browser_game(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}'",
    )
    .await;
    listed_with(&app, maker_id, "relay", "released", "platforms = '{linux}'").await;
    listed_with(
        &app,
        maker_id,
        "web-game",
        "released",
        "platforms = '{browser}'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "secret-relay",
        "beta",
        "status = 'draft', platforms = '{linux}'",
    )
    .await;
    tag_app(&app, "relay", &["dedicated-server"]).await;
    tag_app(&app, "secret-relay", &["dedicated-server"]).await;
    let codes = |query: &str, cookie: Option<&str>| {
        let mut r = req(Method::GET, format!("/v1/reports/reasons?{query}"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res)
                .await
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["code"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    let usual = codes("kind=app", None).await;
    assert!(usual.iter().any(|c| c == "outside_code"));
    let without: Vec<String> = usual
        .iter()
        .filter(|c| *c != "outside_code")
        .cloned()
        .collect();
    assert_eq!(codes("kind=app&app=castle", None).await, usual);
    assert_eq!(codes("kind=app&app=relay", None).await, without);
    assert_eq!(codes("kind=app&app=web-game", None).await, without);
    assert_eq!(
        codes("kind=app&app=secret-relay", None).await,
        usual,
        "a draft"
    );
    assert_eq!(
        codes("kind=app&app=secret-relay", Some(&maker)).await,
        without
    );
    assert_eq!(
        codes("kind=review&app=relay", None).await,
        codes("kind=review", None).await
    );

    // Nor taken if it's sent anyway (an older app, an agent): a moderator would only dismiss it.
    // Who can't see the app learns nothing (404, as for any report on it).
    let (_, pat) = app.user("Pat Player", "US").await;
    let report = |subject: &str| {
        app.send(req(Method::POST, "/v1/reports").cookie(&pat).json(json!({
            "kind": "app", "subject": subject, "reason": "outside_code",
            "details": "It downloads plugins from a mod site.", "good_faith": true
        })))
    };
    for subject in ["relay", "web-game"] {
        let res = report(subject).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{subject}");
        let body = body_json(res).await;
        assert_eq!(body["error"]["field"], "reason", "{subject}");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("doesn't cover browser games or dedicated servers"),
            "{body}"
        );
    }
    assert_eq!(report("secret-relay").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(report("castle").await.status(), StatusCode::OK);
    let reasons: Vec<String> = sqlx::query_scalar("select reason from reports")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(reasons, ["outside_code"], "only castle's");
}

/// The maker chooses the Dedicated Server tag, and the rule on code from outside SLAPorSLOP
/// doesn't cover servers, so an app that has become one since its last approved release waits for
/// a person, who checks it really is a server. Each release's source check records whether it was
/// one; once a person approved it as one, updates go through on their own as before.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_app_that_becomes_a_dedicated_server_waits_for_a_person(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let approve = |id: &str| {
        admin::approve(
            &app.db,
            Some(&app.signing_key),
            lax(),
            id.parse().unwrap(),
            "rev",
        )
    };
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    approve(&first).await.unwrap();
    let update = submit_and_fetch(&app, &maker, "1.1.0").await;
    assert_eq!(release_status(&app, &update).await.0, "approved");

    tag_app(&app, "game", &["dedicated-server"]).await;
    let server = submit_and_fetch(&app, &maker, "1.2.0").await;
    assert_eq!(
        release_status(&app, &server).await,
        (
            "awaiting_review".into(),
            Some(
                "Waiting for a person: it's now in the Servers category, so a person checks it really is a dedicated server"
                    .into()
            ),
            None
        )
    );
    let recorded: Vec<(String, Value)> = sqlx::query_as(
        "select version, repo_scan -> 'dedicated_server' from releases order by version",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        recorded,
        [
            ("1.0.0".to_string(), json!(false)),
            ("1.1.0".to_string(), json!(false)),
            ("1.2.0".to_string(), json!(true)),
        ]
    );

    approve(&server).await.unwrap();
    let next = submit_and_fetch(&app, &maker, "1.3.0").await;
    assert_eq!(release_status(&app, &next).await.0, "approved");
}

// ---------------------------------------------------------------------------------------------
// Dedicated servers linked to their games (migration 0094, docs/DEDICATED-SERVERS.md)

fn new_server(slug: &str, repo: &str, platforms: Value) -> Value {
    json!({
        "slug": slug, "title": format!("{slug} title"), "summary": "Run your own server so others can play.",
        "repo_url": repo, "license_spdx": "MIT", "platforms": platforms
    })
}

/// The game's maker adds dedicated servers to it, each from its own repository; they're shown on
/// the game's page, with the file to download and its SHA-256, and are never listed on their own,
/// treated as community builds, or signed as a build of the game.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_games_maker_adds_dedicated_servers_shown_on_its_page(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Bo Brancher", "US").await;
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "platforms = '{windows}', repo_url = 'https://github.com/mia/castle', min_age = 12,
         content_labels = '{violence}', online = '{player_servers}'",
    )
    .await;
    let add = |who: &str, body: Value| {
        req(Method::POST, "/v1/apps/castle/servers")
            .cookie(who)
            .json(body)
    };
    let server_repo = format!("{}octo/game", app.fake_base);
    let windows = json!(["windows"]);

    // Only the game's maker, never from the game's repository, and a download.
    let res = app
        .send(add(
            &other,
            new_server("castle-server", &server_repo, windows.clone()),
        ))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "kind");
    let res = app
        .send(add(
            &maker,
            new_server(
                "castle-server",
                "https://github.com/mia/castle",
                windows.clone(),
            ),
        ))
        .await;
    let err = body_json(res).await;
    assert_eq!(err["error"]["field"], "repo_url");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("its own repository")
    );
    let res = app
        .send(add(
            &maker,
            new_server("castle-server", &server_repo, json!(["browser"])),
        ))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "platforms");
    // Through the branches route too: kind server is the maker's alone.
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/branches").cookie(&other).json(json!({
            "slug": "bo-server", "title": "Bo's server", "summary": "A server.", "repo_url": "https://github.com/bo/srv",
            "license_spdx": "MIT", "kind": "server", "platforms": ["windows"]
        })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "kind");

    // Several, each its own app.
    for (slug, repo, platforms) in [
        ("castle-server", server_repo.as_str(), windows.clone()),
        (
            "castle-lobby",
            "https://github.com/mia/castle-lobby",
            json!(["linux", "linux-deb"]),
        ),
    ] {
        let res = app
            .send(add(&maker, new_server(slug, repo, platforms)))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED, "{slug}");
    }
    let own = body_json(
        app.send(req(Method::GET, "/v1/apps/castle-server").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        own["server_game"],
        json!({ "slug": "castle", "title": "castle" })
    );
    assert_eq!(own["server_for"], "castle");
    assert!(own["branch"].is_null(), "not a community build");
    assert!(own["branch_of"].is_null());
    assert_eq!(own["kind"], "app");
    assert_eq!(
        own["tags"],
        json!(["dedicated-server"]),
        "its own tag, not the game's"
    );
    assert_eq!(own["min_age"], 12, "the game's rating, as for a mod");
    assert_eq!(own["content_labels"], json!(["violence"]));
    assert!(own["mods_folder"].is_null());
    assert_eq!(own["status"], "draft");
    let server_id = app_id(&app, "castle-server").await;
    assert!(
        slop_api::catalog::is_dedicated_server(&app.db, server_id)
            .await
            .unwrap(),
        "a dedicated server everywhere one is told apart"
    );

    // The maker sees both drafts on the game's page; nobody else sees any yet.
    let page = |who: Option<&str>| {
        let r = req(Method::GET, "/v1/apps/castle").header("x-country", "US");
        match who {
            Some(c) => r.cookie(c),
            None => r,
        }
    };
    let mine = body_json(app.send(page(Some(&maker))).await).await;
    let slugs = |p: &Value| -> Vec<String> {
        p["servers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["slug"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(slugs(&mine), ["castle-lobby", "castle-server"]);
    assert_eq!(mine["servers"][1]["status"], "draft");
    assert!(mine["servers"][1]["release"].is_null());
    assert!(slugs(&body_json(app.send(page(None)).await).await).is_empty());

    // Released, approved and signed as an app of its own, then listed by a reviewer.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/castle-server/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "server-windows.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    admin::approve(&app.db, Some(&app.signing_key), lax(), release, "rev")
        .await
        .unwrap();
    admin::list_app(&app.db, "castle-server", "rev")
        .await
        .unwrap();
    let signed: slop_manifest::Signed = serde_json::from_value(
        body_json(
            app.send(req(Method::GET, "/v1/apps/castle-server/manifest").header("x-country", "US"))
                .await,
        )
        .await,
    )
    .unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert!(
        manifest.build_of.is_none(),
        "not a build of the game to play instead"
    );
    assert!(manifest.mod_for.is_none());

    // Everyone sees it on the game's page, with what to download and check it with.
    let page = body_json(app.send(page(None)).await).await;
    assert_eq!(
        slugs(&page),
        ["castle-server"],
        "the lobby is still a draft"
    );
    let server = &page["servers"][0];
    assert_eq!(server["title"], "castle-server title");
    assert_eq!(server["platforms"], json!(["windows"]));
    assert!(server.get("status").is_none(), "listed");
    assert_eq!(server["release"]["version"], "1.0.0");
    assert_eq!(server["release"]["id"], release.to_string());
    let file = &server["release"]["files"][0];
    assert_eq!(file["platform"], "windows");
    assert_eq!(file["file_name"], "server-windows.zip");
    assert_eq!(file["size_bytes"], BUILD_BYTES.len());
    assert_eq!(
        file["sha256"],
        hex::encode(<sha2::Sha256 as sha2::Digest>::digest(BUILD_BYTES))
    );
    assert_eq!(file["parts"], 0);
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/apps/castle-server/releases/{release}/download/windows"),
            )
            .header("x-country", "US"),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "downloadable without the desktop app"
    );
    assert!(
        page["community"].as_array().unwrap().is_empty(),
        "not a community build"
    );

    // Not in the store on its own, nor in Community. The Servers category (and its tag) lists
    // the game; a server's platforms don't make the game match them.
    let get =
        |q: &str| req(Method::GET, format!("/v1/apps?sort=title&{q}")).header("x-country", "US");
    assert_eq!(list_slugs(&app, get("")).await, ["castle"]);
    assert!(list_slugs(&app, get("community=true")).await.is_empty());
    assert_eq!(list_slugs(&app, get("category=servers")).await, ["castle"]);
    assert_eq!(
        list_slugs(&app, get("tag=dedicated-server")).await,
        ["castle"]
    );
    sqlx::query(
        "update apps set platforms = '{windows,linux,linux-deb}' where slug = 'castle-server'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert!(list_slugs(&app, get("platform=linux")).await.is_empty());
    let reel = |page: &Value, slug: &str| -> Vec<String> {
        page["reels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["slug"] == slug)
            .unwrap()["apps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap().to_string())
            .collect()
    };
    let browse = body_json(
        app.send(req(Method::GET, "/v1/browse").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(reel(&browse, "servers"), ["castle"]);
    let browse = body_json(
        app.send(req(Method::GET, "/v1/browse?category=servers").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(reel(&browse, "dedicated-server"), ["castle"]);

    // Not another version of the game to play, and running it isn't playtime.
    let updates = body_json(
        app.send(req(Method::GET, "/v1/library/updates?apps=castle").header("x-country", "US"))
            .await,
    )
    .await;
    assert!(
        updates["castle"]["builds"]
            .as_array()
            .is_none_or(|b| b.is_empty()),
        "{updates}"
    );
    let (player_id, player) = app.user("Pat Player", "US").await;
    let launched = body_json(
        app.send(
            req(
                Method::POST,
                "/v1/apps/castle-server/launch?platform=windows",
            )
            .cookie(&player),
        )
        .await,
    )
    .await;
    assert!(launched["token"].is_null(), "{launched}");
    let used: i64 = sqlx::query_scalar("select count(*) from user_app_usage where user_id = $1")
        .bind(player_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(used, 0, "starting a server isn't playing");

    // Taking the game out of the store takes its servers with it.
    sqlx::query("update apps set status = 'delisted' where slug = 'castle'")
        .execute(&app.db)
        .await
        .unwrap();
    for path in ["/v1/apps/castle-server", "/v1/apps/castle-server/manifest"] {
        assert_eq!(
            app.send(req(Method::GET, path).header("x-country", "US"))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert!(list_slugs(&app, get("category=servers")).await.is_empty());
}

/// A private game's dedicated server is shared with whoever the game is, and goes public (and in
/// the store, once ready) with it. Community builds and mods still can't be made of a private game.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_private_games_server_is_shared_and_goes_public_with_it(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (tester_id, tester) = app.user("Tess Tester", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "status = 'draft', visibility = 'private', repo_url = 'https://github.com/mia/secret', platforms = '{windows}'",
    )
    .await;
    approved_release(&app, "secret", maker_id).await;
    sqlx::query(
        "insert into app_shares (app_id, user_id) select id, $1 from apps where slug = 'secret'",
    )
    .bind(tester_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret/servers")
                .cookie(&maker)
                .json(new_server(
                    "secret-server",
                    "https://github.com/mia/secret-server",
                    json!(["windows"]),
                )),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let visibility: String =
        sqlx::query_scalar("select visibility from apps where slug = 'secret-server'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(visibility, "private");
    let see = |who: &str| {
        req(Method::GET, "/v1/apps/secret-server")
            .cookie(who)
            .header("x-country", "US")
    };
    // Shared once it has an approved release, like any private app.
    assert_eq!(app.send(see(&tester)).await.status(), StatusCode::NOT_FOUND);
    approved_release(&app, "secret-server", maker_id).await;
    assert_eq!(app.send(see(&tester)).await.status(), StatusCode::OK);
    assert_eq!(
        app.send(see(&stranger)).await.status(),
        StatusCode::NOT_FOUND
    );
    let page = body_json(
        app.send(
            req(Method::GET, "/v1/apps/secret")
                .cookie(&tester)
                .header("x-country", "US"),
        )
        .await,
    )
    .await;
    assert_eq!(page["servers"][0]["slug"], "secret-server");
    // Others still can't branch or mod it.
    let res = app
        .send(req(Method::POST, "/v1/apps/secret/branches").cookie(&tester).json(json!({
            "slug": "secret-fix", "title": "Fix", "summary": "A fix.", "repo_url": "https://github.com/t/f",
            "license_spdx": "MIT", "kind": "fix", "note": "Fixes it.", "platforms": ["windows"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Public: the server too, and in the store with the game.
    sqlx::query("update apps set public_requested_at = now() where slug = 'secret'")
        .execute(&app.db)
        .await
        .unwrap();
    admin::make_public(&app.db, "secret", "rev").await.unwrap();
    let (visibility, status): (String, String) =
        sqlx::query_as("select visibility, status::text from apps where slug = 'secret-server'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((visibility.as_str(), status.as_str()), ("public", "listed"));
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/secret").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(page["servers"][0]["slug"], "secret-server");
}

/// A browser game has nothing to host, and a server belongs to the game itself, not to one of
/// its community builds.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn browser_games_and_community_builds_have_no_dedicated_servers(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "web-game",
        "released",
        "platforms = '{browser}'",
    )
    .await;
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
        "castle-port",
        "released",
        "parent_app_id = (select id from apps where slug = 'castle'), branch_kind = 'port', branch_note = 'Linux'",
    )
    .await;
    for (game, want) in [
        ("web-game", StatusCode::CONFLICT),
        ("castle-port", StatusCode::CONFLICT),
    ] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/apps/{game}/servers"))
                    .cookie(&maker)
                    .json(new_server(
                        "a-server",
                        "https://github.com/mia/a-server",
                        json!(["windows"]),
                    )),
            )
            .await;
        assert_eq!(res.status(), want, "{game}");
    }
}
