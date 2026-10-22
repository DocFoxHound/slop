// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 2 of the admin tests: the simulation sandbox, the command line, accounts and tokens for
//! agents, the admin MCP tools, shelves and mediation, the router's layers and the background
//! jobs.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// A signed-in staff member with one role (staff.rs).
async fn staff(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
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

/// A new admin API token made by this signed-in admin, as an Authorization header value.
async fn token(app: &TestApp, cookie: &str, scope: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(cookie)
                .json(json!({ "name": format!("Agent {scope}"), "scope": scope, "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
}

/// Calls one admin MCP tool with this Authorization header: (isError, the text it answered).
async fn tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/mcp")
                .header("authorization", auth.to_string())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                              "params": { "name": name, "arguments": args } })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    let result = &body["result"];
    assert!(result.is_object(), "{name}: {body}");
    (
        result["isError"].as_bool().unwrap(),
        result["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

async fn audit_count(app: &TestApp, action: &str) -> i64 {
    sqlx::query_scalar("select count(*) from audit_log where action = $1")
        .bind(action)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn notices(app: &TestApp, user: Uuid, title_like: &str) -> i64 {
    sqlx::query_scalar("select count(*) from notifications where user_id = $1 and title like $2")
        .bind(user)
        .bind(title_like)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The test server's Postgres address for another database on it.
fn database_url(name: &str) -> String {
    let mut url: url::Url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL")
        .parse()
        .unwrap();
    url.set_path(&format!("/{name}"));
    url.to_string()
}

/// This test's own database's address.
async fn own_database_url(db: &PgPool) -> String {
    let name: String = sqlx::query_scalar("select current_database()::text")
        .fetch_one(db)
        .await
        .unwrap();
    database_url(&name)
}

/// A fresh empty database (dropped first if an earlier run left it), and a pool on it.
async fn scratch_database(db: &PgPool, name: &str) -> PgPool {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "drop database if exists \"{name}\" with (force)"
    )))
    .execute(db)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("create database \"{name}\"")))
        .execute(db)
        .await
        .unwrap();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url(name))
        .await
        .unwrap()
}

async fn drop_database(db: &PgPool, pool: PgPool, name: &str) {
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "drop database if exists \"{name}\" with (force)"
    )))
    .execute(db)
    .await
    .unwrap();
}

/// A user that `sim::generate` would have made, with the table it keeps them in.
async fn sim_user(app: &TestApp, name: &str) -> Uuid {
    sqlx::query(
        "create table if not exists sim_users (user_id uuid primary key, personas text not null, taste jsonb not null, activity real not null)",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let (id, _) = app.user(name, "US").await;
    sqlx::query(
        "insert into sim_users (user_id, personas, taste, activity) values ($1, 'cozy', '{}', 0.5)",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

// ---------------------------------------------------------------------------------------------
// The simulation sandbox

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn viewing_as_a_simulated_user_needs_the_sandbox_switch(db: PgPool) {
    // A real server (SLOP_SIMULATION off) never mints a session, even for a simulated user.
    let real = test_app(db.clone()).await;
    let player = sim_user(&real, "Sim player 1").await;
    let res = real
        .send(req(Method::POST, format!("/v1/sim/login/{player}")))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(set_cookie(&res, "slop_session").is_none());
    assert_eq!(
        real.send(req(Method::GET, "/v1/sim/users")).await.status(),
        StatusCode::NOT_FOUND
    );

    // The sandbox: simulated players and makers only, never a real account.
    let app = test_app_with(db, |c| c.simulation = true).await;
    let (person, _) = app.user("Real Person", "US").await;
    let (maker, _) = app.user("Sim maker 3", "US").await;
    for id in [person, Uuid::now_v7()] {
        let res = app
            .send(req(Method::POST, format!("/v1/sim/login/{id}")))
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(set_cookie(&res, "slop_session").is_none());
    }
    let sessions_before: i64 = sqlx::query_scalar("select count(*) from sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for (id, name) in [(player, "Sim player 1"), (maker, "Sim maker 3")] {
        let res = app
            .send(req(Method::POST, format!("/v1/sim/login/{id}")))
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let cookie = set_cookie(&res, "slop_session").unwrap();
        let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&cookie)).await).await;
        assert_eq!(me["display_name"], name);
    }
    let sessions_after: i64 = sqlx::query_scalar("select count(*) from sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(sessions_after, sessions_before + 2);
    let users = body_json(app.send(req(Method::GET, "/v1/sim/users")).await).await;
    let names: Vec<&str> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["display_name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Sim player 1") && names.contains(&"Sim maker 3"),
        "{names:?}"
    );
    assert!(!names.contains(&"Real Person"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_sandbox_not_generated_yet_has_nobody_to_view_as(db: PgPool) {
    // SLOP_SIMULATION on, but `simulate generate` hasn't made its tables yet.
    let app = test_app_with(db, |c| c.simulation = true).await;
    let (person, _) = app.user("Real Person", "US").await;
    let res = app
        .send(req(Method::POST, format!("/v1/sim/login/{person}")))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app.send(req(Method::GET, "/v1/sim/users")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_sandbox_guard_refuses_any_other_database(db: PgPool) {
    // Test databases count as sandboxes (that's how the simulation test runs).
    let name = slop_api::sim::ensure_sandbox(&db).await.unwrap();
    assert!(name.starts_with("_sqlx_test"), "{name}");

    let sandbox = scratch_database(&db, "slop_admin2_guard_sim").await;
    assert_eq!(
        slop_api::sim::ensure_sandbox(&sandbox).await.unwrap(),
        "slop_admin2_guard_sim"
    );
    drop_database(&db, sandbox, "slop_admin2_guard_sim").await;

    // Anything else is refused before the simulation touches a table.
    for other in [
        "slop_admin2_guard",
        "slop_admin2_guard_sim_old",
        "slop_admin2_sqlx_test",
    ] {
        let pool = scratch_database(&db, other).await;
        let err = slop_api::sim::ensure_sandbox(&pool).await.unwrap_err();
        assert!(
            err.to_string().contains("its name must end in _sim")
                && err.to_string().contains(other),
            "{err}"
        );
        let err = slop_api::sim::generate(
            &pool,
            slop_api::sim::Settings {
                apps: 1,
                users: 1,
                days: 1,
                seed: 1,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("sandbox"), "{err}");
        let err = slop_api::sim::pulse_tick(&pool, &mut slop_api::sim::Rng::new(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sandbox"), "{err}");
        drop_database(&db, pool, other).await;
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn simulated_live_players_replace_the_last_set_and_are_counted(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Sim maker 1", "US").await;
    listed_with(&app, maker, "busy", "released", "").await;
    listed_with(&app, maker, "quiet", "released", "").await;
    let (busy, quiet) = (app_id(&app, "busy").await, app_id(&app, "quiet").await);
    let mut players = vec![];
    for i in 0..12 {
        let (id, _) = app.user(&format!("Player {i}"), "US").await;
        sqlx::query(
            "insert into user_app_usage (user_id, app_id, playtime_seconds) values ($1, $2, 600)",
        )
        .bind(id)
        .bind(busy)
        .execute(&app.db)
        .await
        .unwrap();
        players.push(id);
    }
    // Busy had about ten players at its daily peak this week; quiet had none.
    sqlx::query(
        "insert into pulse_counts (app_id, bucket, players)
         select $1, date_trunc('hour', now()) - make_interval(days => d), 10 from generate_series(1, 5) d",
    )
    .bind(busy)
    .execute(&app.db)
    .await
    .unwrap();
    // A session left from the last tick, on the quiet app: replaced.
    sqlx::query(
        "insert into pulse_sessions (id, token_hash, app_id, user_id, started_at, last_beat_at)
         values ($1, 'old-token', $2, $3, now(), now())",
    )
    .bind(Uuid::now_v7())
    .bind(quiet)
    .bind(players[0])
    .execute(&app.db)
    .await
    .unwrap();

    let live = slop_api::sim::pulse_tick(&app.db, &mut slop_api::sim::Rng::new(3))
        .await
        .unwrap();
    let busy_live = live[&busy];
    assert!((1..=12).contains(&busy_live), "{live:?}");
    assert!(!live.contains_key(&quiet));
    let sessions: Vec<(Uuid, i64)> =
        sqlx::query_as("select app_id, count(*) from pulse_sessions group by app_id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(sessions, [(busy, busy_live)], "only the new set");
    // The real sampler counted them (pulse::sample).
    let counted: i32 = sqlx::query_scalar(
        "select players from pulse_counts where app_id = $1 order by bucket desc limit 1",
    )
    .bind(busy)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(i64::from(counted), busy_live);
}

// ---------------------------------------------------------------------------------------------
// Accounts, for staff and their agents

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_find_accounts_by_name_handle_or_id(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, nobody) = app.user("Nobody Special", "US").await;
    let (percent, _) = app.user("100% Real", "US").await;
    app.user("100 Real", "US").await;
    let (under, _) = app.user("a_b", "US").await;
    app.user("axb", "US").await;
    let (artist, _) = app.user("Quiet Artist", "US").await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'pixel-witch', 'Pixel Witch', 'none', false)",
    )
    .bind(artist)
    .execute(&app.db)
    .await
    .unwrap();
    let find = |q: &str| {
        req(
            Method::GET,
            format!(
                "/v1/admin/users?q={}",
                url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>()
            ),
        )
        .cookie(&owner)
    };
    let ids = |v: Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_str().unwrap().to_string())
            .collect()
    };

    for q in ["", "   ", &"x".repeat(101)] {
        let res = app.send(find(q)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{q:?}");
        assert_eq!(body_json(res).await["error"]["field"], "q");
    }
    assert_eq!(
        app.send(find(&"x".repeat(100))).await.status(),
        StatusCode::OK
    );
    // By account ID, and by People handle (shown with it).
    let found = body_json(app.send(find(&artist.to_string())).await).await;
    assert_eq!(ids(found), [artist.to_string()]);
    let found = body_json(app.send(find("PIXEL-wit")).await).await;
    assert_eq!(found[0]["id"], artist.to_string());
    assert_eq!(found[0]["handle"], "pixel-witch");
    // % and _ are letters, not wildcards.
    assert_eq!(
        ids(body_json(app.send(find("100%")).await).await),
        [percent.to_string()]
    );
    assert_eq!(
        ids(body_json(app.send(find("a_b")).await).await),
        [under.to_string()]
    );
    // Not staff: the admin API isn't there.
    let res = app
        .send(req(Method::GET, "/v1/admin/users?q=real").cookie(&nobody))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/users?q=real"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reading_an_account_needs_moderation_rights_and_is_logged(db: PgPool) {
    let app = test_app(db).await;
    let (target, _) = app.user("Tess Target", "US").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, finance) = staff(&app, "Fay Finance", "finance").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let read =
        |cookie: &str, id: Uuid| req(Method::GET, format!("/v1/admin/users/{id}")).cookie(cookie);

    for cookie in [&reviewer, &finance] {
        assert_eq!(
            app.send(read(cookie, target)).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        audit_count(&app, "user.view").await,
        0,
        "refusals aren't views"
    );
    assert_eq!(
        app.send(read(&support, Uuid::now_v7())).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(read(&support, target)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let details = body_json(res).await;
    assert_eq!(details["display_name"], "Tess Target");
    assert_eq!(details["decisions"], json!([]));
    let (actor, subject): (String, String) =
        sqlx::query_as("select actor, subject from audit_log where action = 'user.view'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("Sue Support"), "{actor}");
    assert_eq!(subject, target.to_string());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn lifting_a_suspension_early_takes_a_reason_and_sensitive_rights(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (_, moderator) = staff(&app, "Mo Moderator", "moderator").await;
    let (troll, _) = app.user("Troll", "US").await;
    let review = token(&app, &owner, "review").await;
    let lift = |reason: &str| {
        req(
            Method::POST,
            format!("/v1/admin/users/{troll}/lift-suspension"),
        )
        .json(json!({ "reason": reason }))
    };
    let reason = "The reports were a misunderstanding.";

    // Not suspended: nothing to lift.
    let res = app.send(lift(reason).cookie(&moderator)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{troll}/decide"))
                .cookie(&owner)
                .json(json!({ "action": "suspend_account", "rule": "harassment", "facts": "Insulted twelve people." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    for cookie in [&reviewer, &support] {
        assert_eq!(
            app.send(lift(reason).cookie(cookie)).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        app.send(lift(reason).header("authorization", review))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for short_or_long in ["Too short", &"x".repeat(1001), "          x         "] {
        let res = app.send(lift(short_or_long).cookie(&moderator)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{troll}/lift-suspension"),
            )
            .cookie(&moderator)
            .json(json!({ "reason": reason, "extra": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let suspended: bool =
        sqlx::query_scalar("select suspended_at is not null from users where id = $1")
            .bind(troll)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(suspended, "nothing changed yet");

    // 10 characters exactly is enough; one approval is configured, so it happens now.
    let res = app.send(lift(" Apologised ").cookie(&moderator)).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (suspended, reversed): (bool, bool) = sqlx::query_as(
        "select u.suspended_at is not null, d.reversed_at is not null
         from users u join moderation_decisions d on d.user_id = u.id and d.action = 'suspend_account'
         where u.id = $1",
    )
    .bind(troll)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(!suspended && reversed);
    assert_eq!(
        notices(&app, troll, "Your account is no longer suspended").await,
        1
    );
    let (actor, detail): (String, Value) = sqlx::query_as(
        "select actor, detail from audit_log where action = 'moderation.lift_suspension'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(actor.contains("Mo Moderator"), "{actor}");
    assert_eq!(detail["reason"], "Apologised");
    assert_eq!(
        app.send(lift(reason).cookie(&moderator)).await.status(),
        StatusCode::CONFLICT,
        "once"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_passkey_grace_week_needs_a_reason_an_open_account_and_sensitive_rights(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (closed, _) = app.user("Gone Person", "US").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed)
        .execute(&app.db)
        .await
        .unwrap();
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let review = token(&app, &owner, "review").await;
    let moderate = token(&app, &owner, "moderate").await;
    let grace = |id: Uuid, reason: &str| {
        req(Method::POST, format!("/v1/admin/users/{id}/passkey-grace"))
            .json(json!({ "reason": reason }))
    };
    let reason = "Video call with photo ID; matches the account.";

    assert_eq!(
        app.send(grace(maker, reason).header("authorization", review))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(grace(maker, reason).cookie(&reviewer))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for bad in ["Trust me", &"x".repeat(1001)] {
        let res = app.send(grace(maker, bad).cookie(&owner)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    for id in [closed, Uuid::now_v7()] {
        assert_eq!(
            app.send(grace(id, reason).cookie(&owner)).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    let until: Option<OffsetDateTime> =
        sqlx::query_scalar("select passkey_grace_until from users where id = $1")
            .bind(maker)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(until.is_none());

    // A moderate token may ask; with one approval configured it's done at once.
    let res = app
        .send(grace(maker, reason).header("authorization", moderate))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["until"].is_string());
    let until: OffsetDateTime =
        sqlx::query_scalar("select passkey_grace_until from users where id = $1")
            .bind(maker)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let left = until - OffsetDateTime::now_utc();
    assert!(
        left > time::Duration::days(6) && left <= time::Duration::days(7),
        "{left}"
    );
    assert_eq!(notices(&app, maker, "Add a new passkey this week").await, 1);
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'account.passkey_grace'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("Agent moderate"), "{actor}");
}

// ---------------------------------------------------------------------------------------------
// Admin API tokens

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_security_staff_signed_in_themselves_list_and_revoke_tokens(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, security) = staff(&app, "Sid Security", "security").await;
    let agent = token(&app, &owner, "moderate").await;
    let list = body_json(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&owner))
            .await,
    )
    .await;
    let id = list[0]["id"].as_str().unwrap().to_string();
    assert_eq!(list[0]["admin"], "Olive Owner");
    assert!(list[0]["revoked_at"].is_null());
    let revoke = || req(Method::POST, format!("/v1/admin/tokens/{id}/revoke"));

    // A token never sees or manages tokens, not even its own.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/tokens").header("authorization", agent.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(revoke().header("authorization", agent.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Staff without the security role.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&reviewer))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(revoke().cookie(&reviewer)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(audit_count(&app, "admin_token.revoke").await, 0);

    // Security staff see every admin's tokens and revoke them, once.
    let seen = body_json(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&security))
            .await,
    )
    .await;
    assert_eq!(seen[0]["id"], id.as_str());
    assert_eq!(
        app.send(revoke().cookie(&security)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(revoke().cookie(&security)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/tokens/{}/revoke", Uuid::now_v7())
            )
            .cookie(&owner)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let (actor, subject, detail): (String, String, Value) = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'admin_token.revoke'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(actor.contains("Sid Security"), "{actor}");
    assert_eq!(subject, "Agent moderate");
    assert_eq!(detail["token"], id.as_str());
    let list = body_json(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&owner))
            .await,
    )
    .await;
    assert!(list[0]["revoked_at"].is_string());
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/status").header("authorization", agent))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// MCP

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_mcp_endpoint_has_no_event_stream(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let agent = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/mcp").cookie(&owner))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/mcp").header("authorization", agent))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    for r in [
        req(Method::GET, "/v1/admin/mcp").cookie(&player),
        req(Method::GET, "/v1/admin/mcp"),
        req(Method::GET, "/v1/admin/mcp").header("authorization", "Bearer slopadm_forged"),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
}

/// Every admin tool, with arguments that fit its route, the token scope it needs (read, review or
/// moderate) and which arguments go in the path.
fn tool_cases(ids: &ToolIds) -> Vec<(&'static str, Value, &'static str, &'static [&'static str])> {
    let any = Uuid::now_v7().to_string();
    let troll = ids.troll.to_string();
    vec![
        ("slop_status", json!({}), "read", &[]),
        ("review_guide", json!({}), "read", &[]),
        ("review_queue", json!({ "kind": "release" }), "read", &[]),
        ("admin_queue", json!({}), "read", &[]),
        (
            "add_note",
            json!({ "kind": "listing", "id": ids.game.to_string(), "verdict": "pass", "note": "Looks fine." }),
            "read",
            &["kind", "id"],
        ),
        ("approve_release", json!({ "id": any }), "review", &["id"]),
        (
            "reject_release",
            json!({ "id": any, "reason": "Crashes at start." }),
            "review",
            &["id"],
        ),
        (
            "withdraw_release",
            json!({ "id": any, "reason": "Ships a miner." }),
            "moderate",
            &["id"],
        ),
        (
            "clear_scan_finding",
            json!({ "id": any, "platform": "windows", "reason": "A false positive." }),
            "review",
            &["id", "platform"],
        ),
        (
            "keep_rescanned_release",
            json!({ "id": any, "platform": "windows", "reason": "A false positive." }),
            "review",
            &["id", "platform"],
        ),
        (
            "decide_change",
            json!({ "id": any, "decision": "reject", "reason": "Not the same game." }),
            "review",
            &["id"],
        ),
        (
            "decide_pictures",
            json!({ "id": any, "decision": "approve" }),
            "review",
            &["id"],
        ),
        (
            "decide_portfolio",
            json!({ "id": any, "decision": "approve" }),
            "review",
            &["id"],
        ),
        ("list_app", json!({ "slug": "game" }), "review", &["slug"]),
        (
            "delist_app",
            json!({ "slug": "game", "rule": "spam", "reason": "Copies of one page." }),
            "moderate",
            &["slug"],
        ),
        (
            "act_on_report",
            json!({ "id": any, "action": "warn", "rule": "spam", "facts": "Spam.", "outcome": "We warned them." }),
            "review",
            &["id"],
        ),
        (
            "dismiss_report",
            json!({ "id": any, "outcome": "Nothing wrong." }),
            "review",
            &["id"],
        ),
        (
            "decide_appeal",
            json!({ "id": any, "reverse": false, "reason": "The rule applies." }),
            "review",
            &["id"],
        ),
        (
            "close_abuse_signal",
            json!({ "id": any, "note": "A launch day.", "acted": false }),
            "review",
            &["id"],
        ),
        ("find_users", json!({ "q": "Troll" }), "read", &[]),
        ("user_details", json!({ "id": troll }), "review", &["id"]),
        (
            "moderate_user",
            json!({ "id": troll, "action": "warn", "rule": "spam", "facts": "Posted the same link 40 times." }),
            "review",
            &["id"],
        ),
        (
            "lift_suspension",
            json!({ "id": troll, "reason": "The reports were mistaken." }),
            "moderate",
            &["id"],
        ),
        (
            "passkey_grace",
            json!({ "id": troll, "reason": "Checked their photo ID on a call." }),
            "moderate",
            &["id"],
        ),
        ("pending_actions", json!({ "status": "all" }), "read", &[]),
        (
            "audit_log",
            json!({ "q": "Troll", "before": 999_999 }),
            "read",
            &[],
        ),
        (
            "find_apps",
            json!({ "q": "game", "status": "draft" }),
            "read",
            &[],
        ),
        ("app_record", json!({ "slug": "game" }), "read", &["slug"]),
        (
            "edit_store_page",
            json!({ "slug": "game", "title": "Game", "min_age": 12, "content_labels": [], "reason": "Fixed the age." }),
            "review",
            &["slug"],
        ),
        (
            "set_app_stage",
            json!({ "slug": "game", "stage": "coming_soon" }),
            "review",
            &["slug"],
        ),
        (
            "freeze_app",
            json!({ "slug": "game", "reason": "The repository is gone." }),
            "review",
            &["slug"],
        ),
        (
            "unfreeze_app",
            json!({ "slug": "game", "note": "It's back." }),
            "review",
            &["slug"],
        ),
        ("donation_pauses", json!({ "show": "all" }), "read", &[]),
        (
            "pause_donations",
            json!({ "slug": "game", "kind": "review", "reason": "The page promises rewards.", "ground": "misleading" }),
            "review",
            &["slug"],
        ),
        (
            "lift_donation_pause",
            json!({ "id": ids.hold.to_string(), "note": "Fixed." }),
            "review",
            &["id"],
        ),
        (
            "answer_donation_request",
            json!({ "id": ids.request.to_string(), "lift": false, "answer": "The page still promises rewards." }),
            "review",
            &["id"],
        ),
        ("user_profile", json!({ "id": troll }), "review", &["id"]),
        (
            "edit_profile",
            json!({ "id": troll, "display_name": "Troll", "creative": { "remove_links": true }, "reason": "Rude name." }),
            "review",
            &["id"],
        ),
        (
            "edit_team",
            json!({ "slug": "no-team", "name": "Team", "about": "", "reason": "Rude name." }),
            "review",
            &["slug"],
        ),
        ("featured_apps", json!({}), "read", &[]),
        (
            "set_featured_apps",
            json!({ "apps": [{ "slug": "game", "blurb": "Good." }] }),
            "review",
            &[],
        ),
        ("store_shelves", json!({}), "read", &[]),
        (
            "create_shelf",
            json!({ "slug": "cozy", "title": "Cozy", "blurb": "", "on_front": true, "apps": [] }),
            "review",
            &[],
        ),
        (
            "update_shelf",
            json!({ "slug": "cozy", "title": "Cozy", "blurb": "", "on_front": false, "apps": [] }),
            "review",
            &["slug"],
        ),
        (
            "delete_shelf",
            json!({ "slug": "cozy" }),
            "review",
            &["slug"],
        ),
        ("order_shelves", json!({ "slugs": ["cozy"] }), "review", &[]),
        (
            "hide_front_shelf",
            json!({ "key": "popular", "hidden": true }),
            "review",
            &["key"],
        ),
        ("legal_documents", json!({}), "read", &[]),
        (
            "publish_legal_document",
            json!({ "kind": "nonsense", "summary": "Clearer words.", "body_md": "# Terms" }),
            "moderate",
            &["kind"],
        ),
        (
            "withdraw_legal_document",
            json!({ "kind": "terms", "version": "1999-01-01", "reason": "A mistake." }),
            "moderate",
            &["kind", "version"],
        ),
        ("staff", json!({}), "read", &[]),
        (
            "change_staff_role",
            json!({ "user_id": troll, "role": "support", "grant": true }),
            "moderate",
            &["user_id"],
        ),
        (
            "remove_from_staff",
            json!({ "user_id": troll, "reason": "Left the team last week." }),
            "moderate",
            &["user_id"],
        ),
        (
            "cancel_pending_action",
            json!({ "id": ids.action.to_string() }),
            "moderate",
            &["id"],
        ),
    ]
}

struct ToolIds {
    troll: Uuid,
    game: Uuid,
    hold: Uuid,
    request: Uuid,
    action: Uuid,
}

/// Owner Olive with read, review and moderate tokens; Troll; the maker's draft "game"; donation
/// pauses on "held-game" and (with the maker's request to lift it) "asked-game"; and Ben's
/// pending request to suspend Troll. Two approvals, so sensitive tools only ask.
async fn tool_fixtures(db: PgPool) -> (TestApp, [String; 3], ToolIds) {
    let app = test_app_with(db, |c| {
        c.approvals_required = 2;
        c.rate_limits.write_per_minute = 100_000;
        c.rate_limits.read_per_minute = 100_000;
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (troll, _) = app.user("Troll", "US").await;
    let (maker, _) = maker_with_app(&app, "released").await;
    let (hold, asked, request) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (id, slug) in [(hold, "held-game"), (asked, "asked-game")] {
        listed_with(&app, maker, slug, "released", "").await;
        sqlx::query(
            "insert into donation_holds (id, app_id, kind, reason, placed_by) values ($1, $2, 'review', 'Check the page.', 'test')",
        )
        .bind(id)
        .bind(app_id(&app, slug).await)
        .execute(&app.db)
        .await
        .unwrap();
    }
    sqlx::query(
        "insert into donation_hold_requests (id, hold_id, message) values ($1, $2, 'Fixed it.')",
    )
    .bind(request)
    .bind(asked)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{troll}/decide"))
                .cookie(&ben)
                .json(json!({ "action": "suspend_account", "rule": "harassment", "facts": "Threats." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let action = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let tokens = [
        token(&app, &owner, "read").await,
        token(&app, &owner, "review").await,
        token(&app, &owner, "moderate").await,
    ];
    let ids = ToolIds {
        troll,
        game: app_id(&app, "game").await,
        hold,
        request,
        action,
    };
    (app, tokens, ids)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn every_admin_tool_reaches_its_route_within_its_tokens_scope(db: PgPool) {
    let (app, tokens, ids) = tool_fixtures(db).await;
    let cases = tool_cases(&ids);
    // The table covers every tool the server lists.
    let listed = body_json(
        app.send(
            req(Method::POST, "/v1/admin/mcp")
                .header("authorization", tokens[0].clone())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await,
    )
    .await;
    let mut names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut covered: Vec<&str> = cases.iter().map(|c| c.0).collect();
    names.sort_unstable();
    covered.sort_unstable();
    assert_eq!(names, covered);

    let level = |scope: &str| {
        ["read", "review", "moderate"]
            .iter()
            .position(|s| *s == scope)
            .unwrap()
    };
    for (name, args, scope, _) in &cases {
        // Below its scope: refused by the route itself (the router's fallback would say
        // not_found; a wrong method, HTTP 405; a body that doesn't fit, a deserialize error).
        for below in &tokens[..level(scope)] {
            let (error, text) = tool(&app, below, name, args.clone()).await;
            assert!(
                error && text.contains("\"forbidden\""),
                "{name} below {scope}: {text}"
            );
        }
        // At its scope: past the permission check.
        let (error, text) = tool(&app, &tokens[level(scope)], name, args.clone()).await;
        for wrong in [
            "\"forbidden\"",
            "HTTP 405",
            "deserialize",
            "missing argument",
        ] {
            assert!(!text.contains(wrong), "{name} at {scope}: {text}");
        }
        if *scope == "read" {
            assert!(!error, "{name}: {text}");
        }
    }
    // What the tools did, in the owner's name through each token.
    let actors: Vec<String> = sqlx::query_scalar(
        "select actor from audit_log where action in ('moderation.warn', 'donations.hold_kept', 'staff_action.cancel', 'shelf.created')",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert!(
        actors
            .iter()
            .all(|a| a.starts_with("AI reviewer \"Agent ") && a.contains("Olive Owner")),
        "{actors:?}"
    );
    let note: String = sqlx::query_scalar("select note from review_notes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(note, "Looks fine.");
    let (warned, cancelled): (i64, String) = sqlx::query_as(
        "select (select count(*) from moderation_decisions where action = 'warn'),
                (select status from staff_actions where id = $1)",
    )
    .bind(ids.action)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((warned, cancelled.as_str()), (1, "cancelled"));
    let answered: String =
        sqlx::query_scalar("select outcome from donation_hold_requests where id = $1")
            .bind(ids.request)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(answered, "kept");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn tool_arguments_stay_inside_their_path_segment(db: PgPool) {
    let (app, tokens, ids) = tool_fixtures(db).await;
    let moderate = &tokens[2];
    let audit_before: i64 = sqlx::query_scalar("select count(*) from audit_log")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for (name, args, _, path_args) in tool_cases(&ids) {
        for arg in path_args {
            for evil in ["../../status", "../../tokens", "game?q=1#x", "a/b"] {
                let mut args = args.clone();
                args[*arg] = json!(evil);
                let (error, text) = tool(&app, moderate, name, args).await;
                assert!(error, "{name}.{arg} = {evil}: {text}");
                // Never another route's answer: the status overview, or the token list.
                assert!(
                    !text.contains("\"waiting\"") && !text.contains("slopadm_"),
                    "{name}.{arg} = {evil}: {text}"
                );
            }
        }
    }
    let audit_after: i64 = sqlx::query_scalar("select count(*) from audit_log")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(audit_after, audit_before, "nothing was done");

    // A decision is approve or reject, and . or .. never stand alone.
    let (error, text) = tool(
        &app,
        moderate,
        "decide_change",
        json!({ "id": Uuid::now_v7().to_string(), "decision": "delete" }),
    )
    .await;
    assert!(
        error && text == "decision must be approve or reject",
        "{text}"
    );
    for dots in [".", ".."] {
        let (error, text) = tool(&app, moderate, "app_record", json!({ "slug": dots })).await;
        assert!(error && text == "slug isn't a valid value", "{text}");
    }
    let (error, text) = tool(&app, moderate, "reject_release", json!({ "id": "x" })).await;
    assert!(error && text == "missing argument: reason", "{text}");
    // A path argument may be a number; it's written as text.
    let (error, text) = tool(&app, moderate, "app_record", json!({ "slug": 42 })).await;
    assert!(error && text.contains("not_found"), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_read_token_cannot_cancel_its_admins_pending_actions(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (troll, _) = app.user("Troll", "US").await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{troll}/decide"))
                .cookie(&owner)
                .json(json!({ "action": "suspend_account", "rule": "harassment", "facts": "Threats." })),
        )
        .await;
    let action = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    // A read token looks and leaves notes; turning down a request is a decision.
    let read = token(&app, &owner, "read").await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/actions/{action}/cancel"))
                .header("authorization", read),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let status: String = sqlx::query_scalar("select status from staff_actions where id = $1::uuid")
        .bind(&action)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    // Olive herself, signed in, withdraws her own request.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/cancel")).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------------------------
// What staff and agents see

/// A folder under the system's temporary folder, new for each call.
fn temp_folder(what: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("slop-admin2-{what}-{}", Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn report(app: &TestApp, priority: i16, due: &str, status: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into reports (id, reference, subject_kind, subject_id, subject_label, reason, details, priority, due_at, status)
         values ($1, $2, 'app', $1, 'App: game', 'spam', 'Spam.', $3, now() + interval '{due}', $4)"
    )))
    .bind(Uuid::now_v7())
    .bind(format!("R-{}", Uuid::now_v7().simple()))
    .bind(priority)
    .bind(status)
    .execute(&app.db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_status_overview_counts_whats_waiting_and_reads_the_servers_folders(db: PgPool) {
    let backups = temp_folder("backups");
    let deploys = temp_folder("deploys");
    std::fs::write(backups.join("last-success"), "2026-10-21T03:00:00Z\n").unwrap();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    std::fs::create_dir_all(deploys.join("builds").join(commit)).unwrap();
    std::os::unix::fs::symlink(deploys.join("builds").join(commit), deploys.join("current"))
        .unwrap();
    let app = test_app_with(db, |c| {
        c.ops.backup_dir = Some(backups.clone());
        c.ops.deploy_dir = Some(deploys.clone());
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    submit_and_fetch(&app, &maker, "1.0.0").await;
    listed_with(&app, maker_id, "one", "released", "").await;
    listed_with(&app, maker_id, "two", "released", "").await;
    report(&app, 0, "1 hour", "open").await;
    report(&app, 1, "-1 hour", "open").await;
    report(&app, 2, "7 days", "open").await;
    report(&app, 2, "-7 days", "dismissed").await;
    for (key, status) in [("a", "open"), ("b", "dismissed")] {
        sqlx::query(
            "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail, status)
             values ($1, 'vote_burst', $2, 'app', $1, 'App: one', '{}', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(key)
        .bind(status)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let (closed, _) = app.user("Gone", "US").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed)
        .execute(&app.db)
        .await
        .unwrap();

    let reviewer = staff(&app, "Rex Reviewer", "reviewer").await.1;
    for cookie in [&owner, &reviewer] {
        let res = app
            .send(req(Method::GET, "/v1/admin/status").cookie(cookie))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let s = body_json(res).await;
        let w = &s["waiting"];
        assert_eq!(w["releases_to_review"], 1, "{s}");
        assert_eq!(
            (
                &w["reports_urgent"],
                &w["reports_serious"],
                &w["reports_other"],
                &w["reports_overdue"]
            ),
            (&json!(1), &json!(1), &json!(1), &json!(1)),
            "{s}"
        );
        assert_eq!(w["abuse_signals"], 1);
        assert_eq!(s["apps_listed"], 2);
        // Olive, Mia Maker, Rex; not the closed account.
        assert_eq!(s["accounts"], 3);
        assert_eq!(s["last_backup"], "2026-10-21T03:00:00Z");
        assert!(s["last_restore_check"].is_null());
        assert_eq!(s["deployed"], "0123456789ab");
        assert_eq!(
            (s["signing_key"].as_bool(), s["scanning"].as_bool()),
            (Some(true), Some(false))
        );
    }
    let (_, player) = app.user("Pat Player", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/status").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Without the folders: nothing to say about backups or the deploy.
    let plain = test_app(app.db.clone()).await;
    let s = body_json(
        plain
            .send(req(Method::GET, "/v1/admin/status").cookie(&owner))
            .await,
    )
    .await;
    assert!(s["last_backup"].is_null() && s["deployed"].is_null(), "{s}");
    std::fs::remove_dir_all(backups).unwrap();
    std::fs::remove_dir_all(deploys).unwrap();
}

/// A creative, a client and a job between them on the maker's app, with an open problem and a
/// disputed review. Returns (job, problem).
async fn job_with_trouble(app: &TestApp, app_slug: &str) -> (Uuid, Uuid) {
    let (creative, _) = app.user("Cass Creative", "US").await;
    let (client, _) = app.user("Cleo Client", "US").await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'cass', 'Cass', 'none', false)",
    )
    .bind(creative)
    .execute(&app.db)
    .await
    .unwrap();
    let (job, problem) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md)
         values ($1, $2, $3, $4, 'art', 'completed', 'Draw a cat.')",
    )
    .bind(job)
    .bind(creative)
    .bind(client)
    .bind(app_id(app, app_slug).await)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into job_problems (id, job_id, opened_by, statement) values ($1, $2, $3, 'The cat has five legs.')",
    )
    .bind(problem)
    .bind(job)
    .bind(client)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into creative_reviews (job_id, creative_id, client_id, vote, body) values ($1, $2, $3, -1, 'Five legs!')",
    )
    .bind(job)
    .bind(creative)
    .bind(client)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into review_disputes (job_id, reason, statement) values ($1, 'false_fact', 'It has four legs.')",
    )
    .bind(job)
    .execute(&app.db)
    .await
    .unwrap();
    (job, problem)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_review_feed_filters_by_kind_and_says_who_can_decide(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let release = submit_and_fetch(&app, &maker, "1.0.0").await;
    listed_with(&app, maker_id, "other", "released", "").await;
    let (job, problem) = job_with_trouble(&app, "other").await;
    let read = token(&app, &owner, "read").await;
    let review = token(&app, &owner, "review").await;
    let feed = |auth: &str, kind: &str| {
        req(Method::GET, format!("/v1/admin/review{kind}"))
            .header("authorization", auth.to_string())
    };
    let kinds = |v: &Value| -> Vec<String> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["kind"].as_str().unwrap().to_string())
            .collect()
    };

    let all = body_json(app.send(feed(&review, "")).await).await;
    assert_eq!(all["can_decide"], true);
    let mut all_kinds = kinds(&all);
    all_kinds.sort();
    assert_eq!(all_kinds, ["problem", "release", "review_dispute"], "{all}");

    let releases = body_json(app.send(feed(&read, "?kind=release")).await).await;
    assert_eq!(releases["can_decide"], false);
    assert_eq!(kinds(&releases), ["release"]);
    assert_eq!(releases["items"][0]["id"], release.as_str());
    assert_eq!(
        releases["items"][0]["reject"]["path"],
        format!("/v1/admin/releases/{release}/reject")
    );
    let problems = body_json(app.send(feed(&review, "?kind=problem")).await).await;
    assert_eq!(kinds(&problems), ["problem"]);
    assert_eq!(problems["items"][0]["id"], problem.to_string());
    assert_eq!(
        problems["items"][0]["creative"]["record"],
        format!("/v1/admin/jobs/{job}/record")
    );
    assert_eq!(
        problems["items"][0]["reject"]["path"],
        format!("/v1/admin/problems/{problem}/close")
    );
    let disputes = body_json(app.send(feed(&review, "?kind=review_dispute")).await).await;
    assert_eq!(kinds(&disputes), ["review_dispute"]);
    for empty in ["?kind=change", "?kind=nonsense"] {
        let res = app.send(feed(&review, empty)).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["items"], json!([]), "{empty}");
    }
    // Signed in: whether the staff member's roles let them decide.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/admin/review").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(mine["can_decide"], true);
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/admin/review").cookie(&support))
            .await,
    )
    .await;
    assert_eq!(
        theirs["can_decide"], false,
        "support staff don't review releases"
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{release}/approve")
            )
            .cookie(&support)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn review_notes_need_a_real_subject_a_known_verdict_and_some_words(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let release = submit_and_fetch(&app, &maker, "1.0.0").await;
    let read = token(&app, &owner, "read").await;
    let note = |kind: &str, id: &str, body: Value| {
        req(Method::POST, format!("/v1/admin/review/{kind}/{id}/notes"))
            .header("authorization", read.clone())
            .json(body)
    };
    let fine = json!({ "verdict": "concern", "note": "The installer asks for admin rights." });

    assert_eq!(
        app.send(note("bounty", &release, fine.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for kind in [
        "release",
        "change",
        "media",
        "listing",
        "portfolio",
        "problem",
        "review_dispute",
    ] {
        assert_eq!(
            app.send(note(kind, &Uuid::now_v7().to_string(), fine.clone()))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{kind}"
        );
    }
    let res = app
        .send(note(
            "release",
            &release,
            json!({ "verdict": "great", "note": "Fine." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "verdict");
    for words in ["", "   ", &"x".repeat(4001)] {
        let res = app
            .send(note("release", &release, json!({ "note": words })))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "note");
    }
    assert_eq!(
        app.send(note(
            "release",
            &release,
            json!({ "note": "x", "score": 3 })
        ))
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let count: i64 = sqlx::query_scalar("select count(*) from review_notes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // No verdict is fine; 4000 characters is the most.
    let res = app
        .send(note(
            "release",
            &release,
            json!({ "note": "y".repeat(4000) }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let saved = body_json(res).await;
    assert!(saved["verdict"].is_null());
    let res = app.send(note("release", &release, fine)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let feed = body_json(
        app.send(
            req(Method::GET, "/v1/admin/review?kind=release").header("authorization", read.clone()),
        )
        .await,
    )
    .await;
    let notes = feed["items"][0]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1]["verdict"], "concern");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_see_every_featured_slide_including_unlisted_ones(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker, player) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "shiny", "released", "").await;
    listed_with(&app, maker, "fading", "released", "").await;
    let res = app
        .send(req(Method::PUT, "/v1/admin/featured").cookie(&owner).json(
            json!({ "apps": [{ "slug": "fading", "blurb": "Was great." }, { "slug": "shiny" }] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    sqlx::query("update apps set status = 'delisted' where slug = 'fading'")
        .execute(&app.db)
        .await
        .unwrap();

    let public = body_json(app.send(req(Method::GET, "/v1/featured")).await).await;
    assert_eq!(public.as_array().unwrap().len(), 1);
    let slides = body_json(
        app.send(req(Method::GET, "/v1/admin/featured").cookie(&support))
            .await,
    )
    .await;
    assert_eq!(
        slides,
        json!([
            { "slug": "fading", "title": "fading", "blurb": "Was great.", "listed": false },
            { "slug": "shiny", "title": "shiny", "blurb": "", "listed": true }
        ])
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/featured").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Support staff look but don't change the front page.
    assert_eq!(
        app.send(
            req(Method::PUT, "/v1/admin/featured")
                .cookie(&support)
                .json(json!({ "apps": [] }))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_shelves_keep_their_address_their_order_and_their_reviewers(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker, player) = app.user("Mia Maker", "US").await;
    for slug in ["cozy-farm", "tea-shop", "dark-dungeon"] {
        listed_with(&app, maker, slug, "released", "").await;
    }
    sqlx::query("update apps set status = 'draft' where slug = 'dark-dungeon'")
        .execute(&app.db)
        .await
        .unwrap();
    let send = |method: Method, path: &str, cookie: &str, body: Option<Value>| {
        let r = req(method, format!("/v1/admin{path}")).cookie(cookie);
        app.send(match body {
            Some(b) => r.json(b),
            None => r,
        })
    };
    for (slug, title) in [
        ("cozy", "Cozy games"),
        ("dark", "Dark games"),
        ("tea", "Tea"),
    ] {
        let res = send(
            Method::POST,
            "/shelves",
            &owner,
            Some(json!({ "slug": slug, "title": title, "apps": ["tea-shop", "dark-dungeon"] })),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    // The staff list, with apps that aren't in the store marked.
    let list = body_json(send(Method::GET, "/shelves", &support, None).await).await;
    let slugs: Vec<&str> = list["shelves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["cozy", "dark", "tea"]);
    assert_eq!(
        list["shelves"][0]["apps"],
        json!([
            { "slug": "tea-shop", "title": "tea-shop", "listed": true },
            { "slug": "dark-dungeon", "title": "dark-dungeon", "listed": false }
        ])
    );
    assert!(
        list["built_in"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["hidden"] == false)
    );
    assert_eq!(
        send(Method::GET, "/shelves", &player, None).await.status(),
        StatusCode::NOT_FOUND
    );

    // Support staff can't change shelves.
    let shelf = json!({ "title": "Cozy", "apps": [] });
    for (method, path, body) in [
        (
            Method::POST,
            "/shelves",
            Some(json!({ "slug": "x1", "title": "x", "apps": [] })),
        ),
        (Method::PUT, "/shelves/cozy", Some(shelf.clone())),
        (Method::DELETE, "/shelves/cozy", None),
        (
            Method::PUT,
            "/shelves/order",
            Some(json!({ "slugs": ["tea"] })),
        ),
        (
            Method::POST,
            "/front-shelves/popular",
            Some(json!({ "hidden": true })),
        ),
    ] {
        assert_eq!(
            send(method, path, &support, body).await.status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    // A shelf's address doesn't change; one that isn't there can't be changed.
    let res = send(
        Method::PUT,
        "/shelves/cozy",
        &owner,
        Some(json!({ "slug": "cosy", "title": "Cozy", "apps": [] })),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "slug");
    assert_eq!(
        send(
            Method::PUT,
            "/shelves/cozy",
            &owner,
            Some(json!({ "slug": "cozy", "title": "Cozy", "apps": ["cozy-farm"] }))
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(Method::PUT, "/shelves/nope", &owner, Some(shelf.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(Method::DELETE, "/shelves/nope", &owner, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // The new order is the front page's; an address that isn't a shelf is passed over.
    let res = send(
        Method::PUT,
        "/shelves/order",
        &owner,
        Some(json!({ "slugs": ["tea", "nope", "cozy", "dark"] })),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    let slugs: Vec<&str> = list["shelves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["tea", "cozy", "dark"]);
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    let titles: Vec<&str> = front["shelves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Tea", "Cozy", "Dark games"]);
    let (actor, detail): (String, Value) =
        sqlx::query_as("select actor, detail from audit_log where action = 'shelf.reordered'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.starts_with("Olive Owner"), "{actor}");
    assert_eq!(detail["order"][1], "nope");

    // Built-in shelves: only the known ones, hidden and shown again.
    assert_eq!(
        send(
            Method::POST,
            "/front-shelves/nope",
            &owner,
            Some(json!({ "hidden": true }))
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for hidden in [true, true, false] {
        let res = send(
            Method::POST,
            "/front-shelves/popular",
            &owner,
            Some(json!({ "hidden": hidden })),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let list = body_json(res).await;
        let popular = list["built_in"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["key"] == "popular")
            .unwrap()
            .clone();
        assert_eq!(popular["hidden"], hidden);
    }
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!([]));
    assert_eq!(audit_count(&app, "shelf.built_in_shown").await, 1);
    assert_eq!(audit_count(&app, "shelf.built_in_hidden").await, 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn mediators_suggest_close_and_decide_disputes_on_the_record(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "cats", "released", "").await;
    let (job, problem) = job_with_trouble(&app, "cats").await;
    let suggest = |cookie: &str, text: &str| {
        req(
            Method::POST,
            format!("/v1/admin/problems/{problem}/suggest"),
        )
        .cookie(cookie)
        .json(json!({ "suggestion": text }))
    };
    let close = |cookie: &str, text: &str| {
        req(Method::POST, format!("/v1/admin/problems/{problem}/close"))
            .cookie(cookie)
            .json(json!({ "note": text }))
    };
    let decide = |cookie: &str, body: Value| {
        req(
            Method::POST,
            format!("/v1/admin/review-disputes/{job}/decide"),
        )
        .cookie(cookie)
        .json(body)
    };
    let record =
        |cookie: &str| req(Method::GET, format!("/v1/admin/jobs/{job}/record")).cookie(cookie);

    // Reviewers don't mediate.
    for r in [
        suggest(&reviewer, "Redraw one leg."),
        close(&reviewer, "Not settled."),
        decide(
            &reviewer,
            json!({ "outcome": "keep", "reason": "Within the rules." }),
        ),
        record(&reviewer),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(audit_count(&app, "job.record_read").await, 0);

    // The record, while there's trouble, and every look logged.
    let res = app.send(record(&support)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let text =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(text.contains("Draw a cat."), "{text}");
    let (actor, detail): (String, Value) =
        sqlx::query_as("select actor, detail from audit_log where action = 'job.record_read'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.starts_with("Sue Support"), "{actor}");
    assert_eq!(detail["because"], "problem");

    // A suggestion, then closing it unresolved; each only while it's open.
    assert_eq!(
        app.send(suggest(&support, "")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(suggest(&support, "Redraw one leg."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(close(&support, "")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(close(&support, &"x".repeat(2001))).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(close(&support, "They didn't agree; the record is theirs."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let (status, note, closed): (String, String, bool) = sqlx::query_as(
        "select status, closing_note, closed_at is not null from job_problems where id = $1",
    )
    .bind(problem)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), note.as_str(), closed),
        (
            "unresolved",
            "They didn't agree; the record is theirs.",
            true
        )
    );
    let events: Vec<String> =
        sqlx::query_scalar("select kind from job_events where job_id = $1 order by created_at")
            .bind(job)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(events, ["mediator_suggestion", "problem_closed_unresolved"]);
    assert_eq!(audit_count(&app, "problem.close").await, 1);
    assert_eq!(
        app.send(close(&support, "Again.")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(suggest(&support, "Late.")).await.status(),
        StatusCode::CONFLICT
    );

    // The disputed review: an outcome from the list, a reason, and for "note" the note.
    for bad in [
        json!({ "outcome": "delete", "reason": "Because." }),
        json!({ "outcome": "keep", "reason": "" }),
        json!({ "outcome": "note", "reason": "Partly false." }),
    ] {
        assert_eq!(
            app.send(decide(&owner, bad.clone())).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}"
        );
    }
    let res = app
        .send(decide(
            &owner,
            json!({ "outcome": "note", "reason": "The record shows four legs.", "note": "The delivered cat has four legs." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (hidden, mediator_note): (bool, Option<String>) = sqlx::query_as(
        "select hidden_at is not null, mediator_note from creative_reviews where job_id = $1",
    )
    .bind(job)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(!hidden);
    assert_eq!(
        mediator_note.as_deref(),
        Some("The delivered cat has four legs.")
    );
    let status: String = sqlx::query_scalar("select status from review_disputes where job_id = $1")
        .bind(job)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "noted");
    assert_eq!(
        app.send(decide(
            &owner,
            json!({ "outcome": "keep", "reason": "Again." })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    // Nothing open any more: the record is private again.
    assert_eq!(
        app.send(record(&support)).await.status(),
        StatusCode::NOT_FOUND
    );

    // "keep" leaves the review as it was, with no decision against the reviewer.
    sqlx::query("update review_disputes set status = 'pending' where job_id = $1")
        .bind(job)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update creative_reviews set mediator_note = null where job_id = $1")
        .bind(job)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(decide(
            &owner,
            json!({ "outcome": "keep", "reason": "An opinion, within the rules." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (hidden, mediator_note): (bool, Option<String>) = sqlx::query_as(
        "select hidden_at is not null, mediator_note from creative_reviews where job_id = $1",
    )
    .bind(job)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(!hidden && mediator_note.is_none());
    let decisions: i64 = sqlx::query_scalar("select count(*) from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(decisions, 0);
    let outcomes: Vec<Value> = sqlx::query_scalar(
        "select detail->'outcome' from audit_log where action = 'review_dispute.decide' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(outcomes, [json!("noted"), json!("kept")]);
}
