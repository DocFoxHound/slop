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

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_tokens_need_no_fresh_sign_in_and_come_without_a_command(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (pat, _) = app.user("Pat Player", "US").await;
    // Signed in an hour ago, never confirmed since: too old for step-up.
    sqlx::query("update sessions set created_at = now() - interval '1 hour', stepped_up_at = null")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&owner)
                .json(json!({ "name": "Claude on my laptop", "scope": "moderate", "days": 90 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let made = body_json(res).await;
    let secret = made["token"].as_str().unwrap().to_string();
    assert!(secret.starts_with("slopadm_"), "{made}");
    // The MCP address, and nothing else with the token in it: the website shows how to keep it
    // in an environment variable.
    assert!(
        made["mcp_url"].as_str().unwrap().ends_with("/v1/admin/mcp"),
        "{made}"
    );
    let fields: Vec<&String> = made
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, v)| v.as_str().is_some_and(|v| v.contains(&secret)))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(fields, ["token"], "{made}");
    assert!(!made.to_string().contains("claude mcp"), "{made}");
    // It works.
    assert_eq!(
        app.send(
            req(Method::GET, "/v1/admin/status")
                .header("authorization", format!("Bearer {secret}"))
        )
        .await
        .status(),
        StatusCode::OK
    );
    // The lifetimes are still checked: moderate tokens at most 90 days, others 365.
    for (scope, days) in [("moderate", 91), ("review", 366), ("read", 0)] {
        let res = app
            .send(
                req(Method::POST, "/v1/admin/tokens")
                    .cookie(&owner)
                    .json(json!({ "name": "Too long", "scope": scope, "days": days })),
            )
            .await;
        assert_eq!(
            res.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{scope} {days}"
        );
    }
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&owner)
                .json(json!({ "name": "A year", "scope": "review", "days": 365 }))
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    // Other sensitive steps still ask for a fresh sign-in: changing staff roles.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{pat}/roles"))
                .cookie(&owner)
                .json(json!({ "role": "reviewer", "grant": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "step_up_required");
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
            // Withdrawing your own request needs only a token that may decide.
            "review",
            &["id"],
        ),
        // Messages from AI agents (admin_messages.rs): Troll's thread, then Olive's own.
        (
            "list_ai_messages",
            json!({ "category": "report" }),
            "read",
            &[],
        ),
        (
            "read_ai_message",
            json!({ "id": ids.thread.to_string() }),
            "read",
            &["id"],
        ),
        (
            "reply_to_ai_message",
            json!({ "id": ids.thread.to_string(), "body": "Thanks." }),
            "review",
            &["id"],
        ),
        (
            "file_ai_message",
            json!({ "id": ids.thread.to_string(), "category": "other" }),
            "review",
            &["id"],
        ),
        (
            "close_ai_message",
            json!({ "id": ids.thread.to_string() }),
            "review",
            &["id"],
        ),
        (
            "reopen_ai_message",
            json!({ "id": ids.thread.to_string() }),
            "review",
            &["id"],
        ),
        (
            "block_ai_sender",
            json!({ "user_id": troll, "reason": "Junk." }),
            "moderate",
            &["user_id"],
        ),
        (
            "unblock_ai_sender",
            json!({ "user_id": troll }),
            "moderate",
            &["user_id"],
        ),
        (
            "delete_ai_message",
            json!({ "id": ids.thread.to_string() }),
            "moderate",
            &["id"],
        ),
        ("list_staff_contacts", json!({}), "read", &[]),
        ("list_my_admin_messages", json!({}), "read", &[]),
        (
            "reply_to_admin_message",
            json!({ "id": ids.own_thread.to_string(), "body": "More detail." }),
            "read",
            &["id"],
        ),
        (
            // After the reply above: one post an hour, so this one meets the limit.
            "send_admin_message",
            json!({ "category": "suggestion", "subject": "An idea", "body": "Batch approvals." }),
            "read",
            &[],
        ),
    ]
}

struct ToolIds {
    troll: Uuid,
    game: Uuid,
    hold: Uuid,
    request: Uuid,
    action: Uuid,
    /// Troll's agent's thread to the admins, and Olive's own agent's.
    thread: Uuid,
    own_thread: Uuid,
}

/// Owner Olive with read, review and moderate tokens; Troll; the maker's draft "game"; donation
/// pauses on "held-game" and (with the maker's request to lift it) "asked-game"; and Olive's
/// own pending request to suspend Troll (a token may withdraw only its owner's requests). Two
/// approvals, so sensitive tools only ask.
async fn tool_fixtures(db: PgPool) -> (TestApp, [String; 3], ToolIds) {
    let app = test_app_with(db, |c| {
        c.approvals_required = 2;
        c.rate_limits.write_per_minute = 100_000;
        c.rate_limits.read_per_minute = 100_000;
    })
    .await;
    let (olive, owner) = make_admin(&app, "Olive Owner").await;
    let (troll, _) = app.user("Troll", "US").await;
    let (maker, _) = maker_with_app(&app, "released").await;
    let (hold, asked, request) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (thread, own_thread) = (Uuid::now_v7(), Uuid::now_v7());
    for (id, sender) in [(thread, troll), (own_thread, olive)] {
        sqlx::query(
            "insert into admin_threads (id, sender_id, via, category, subject) values ($1, $2, 'maker_agent', 'report', 'It broke')",
        )
        .bind(id)
        .bind(sender)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into admin_thread_messages (id, thread_id, author_id, from_staff, by_agent, body,
                    token_kind, token_id, token_name, token_scope, token_created_at)
             values ($1, $2, $3, false, true, 'Details.', 'maker', $1, 'Agent', 'read', now() - interval '1 day')",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(sender)
        .execute(&app.db)
        .await
        .unwrap();
    }
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
                .cookie(&owner)
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
    // Writing to the admins takes an account a week old and a token an hour old.
    sqlx::query("update users set created_at = now() - interval '30 days' where id = $1")
        .bind(olive)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update admin_tokens set created_at = now() - interval '2 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    let ids = ToolIds {
        troll,
        game: app_id(&app, "game").await,
        hold,
        request,
        action,
        thread,
        own_thread,
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
            let limited = *name == "send_admin_message" && text.contains("rate_limited");
            assert!(!error || limited, "{name}: {text}");
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
    // Support staff decide the People problems and review disputes in the feed, though not
    // releases (refused below).
    assert_eq!(theirs["can_decide"], true);
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

    // The new order names every shelf once: an address that isn't a shelf, or a missing one,
    // is refused rather than leaving two shelves at one place.
    for slugs in [
        json!(["tea", "nope", "cozy", "dark"]),
        json!(["tea", "cozy"]),
    ] {
        let res = send(
            Method::PUT,
            "/shelves/order",
            &owner,
            Some(json!({ "slugs": slugs })),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{slugs}");
    }
    // The new order is the front page's.
    let res = send(
        Method::PUT,
        "/shelves/order",
        &owner,
        Some(json!({ "slugs": ["tea", "cozy", "dark"] })),
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
    assert_eq!(detail["order"][1], "cozy");

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

// ---------------------------------------------------------------------------------------------
// The router: health, security.txt and the layers every request passes

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_health_check_says_when_the_database_is_unreachable(db: PgPool) {
    let app = test_app(db).await;
    let res = app.send(req(Method::GET, "/healthz")).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let unreachable = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect_lazy("postgres://slop@127.0.0.1:1/nowhere")
        .unwrap();
    let state = AppState::new(unreachable, app.config.clone()).unwrap();
    let res = router(state)
        .unwrap()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header(header::HOST, "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn security_txt_is_served_only_with_a_contact(db: PgPool) {
    let app = test_app(db.clone()).await;
    let res = app
        .send(req(Method::GET, "/.well-known/security.txt"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/plain; charset=utf-8");
    let text =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    let line = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("{name}: {text}"))
            .to_string()
    };
    assert_eq!(line("Contact"), "mailto:security@slop.example");
    assert_eq!(
        line("Canonical"),
        "http://localhost:8080/.well-known/security.txt"
    );
    assert_eq!(line("Preferred-Languages"), "en");
    let expires = OffsetDateTime::parse(&line("Expires"), &Rfc3339).unwrap();
    let left = expires - OffsetDateTime::now_utc();
    assert!(
        left > time::Duration::days(179) && left <= time::Duration::days(180),
        "{left}"
    );

    // Without a security contact, the published contact address stands in.
    let fallback = test_app_with(db.clone(), |c| c.security_contact = None).await;
    let res = fallback
        .send(req(Method::GET, "/.well-known/security.txt"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let text =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(
        text.contains("Contact: mailto:hello@slop.example\n"),
        "{text}"
    );

    let none = test_app_with(db, |c| {
        c.security_contact = None;
        c.contact_email = None;
    })
    .await;
    let res = none
        .send(req(Method::GET, "/.well-known/security.txt"))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_contact_page_shows_the_published_address(db: PgPool) {
    let app = test_app(db.clone()).await;
    let res = app.send(req(Method::GET, "/v1/contact")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["email"], "hello@slop.example");

    let unset = test_app_with(db, |c| c.contact_email = None).await;
    let res = unset.send(req(Method::GET, "/v1/contact")).await;
    assert!(body_json(res).await["email"].is_null());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn every_response_carries_the_browser_policies_and_https_adds_hsts(db: PgPool) {
    let app = test_app(db.clone()).await;
    for path in ["/v1/categories", "/v1/nope", "/healthz"] {
        let res = app.send(req(Method::GET, path)).await;
        let h = res.headers();
        assert_eq!(h["cross-origin-opener-policy"], "same-origin", "{path}");
        assert!(
            h["permissions-policy"]
                .to_str()
                .unwrap()
                .contains("camera=()"),
            "{path}"
        );
        assert_eq!(h["cache-control"], "no-store", "{path}");
        assert_eq!(h["referrer-policy"], "strict-origin-when-cross-origin");
        assert!(
            h.get("strict-transport-security").is_none(),
            "plain http: {path}"
        );
    }
    let secure = test_app_with(db, |c| {
        c.public_url = "https://slop.example".parse().unwrap()
    })
    .await;
    let res = secure.send(req(Method::GET, "/v1/categories")).await;
    assert_eq!(
        res.headers()["strict-transport-security"],
        "max-age=63072000; includeSubDomains; preload"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ordinary_routes_take_small_bodies_and_trusted_origins_may_write(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.trusted_origins = vec!["https://partner.example".into()]
    })
    .await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let mut big = new_app("big-page");
    big["description_md"] = json!("x".repeat(64 * 1024));
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&maker).json(big))
        .await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let apps: i64 = sqlx::query_scalar("select count(*) from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(apps, 0);

    // A cross-site write from the allow-listed origin goes through; from anywhere else it doesn't.
    let write = |origin: &str, slug: &str| {
        req(Method::POST, "/v1/apps")
            .cookie(&maker)
            .header("sec-fetch-site", "cross-site")
            .header("origin", origin.to_string())
            .json(new_app(slug))
    };
    assert_eq!(
        app.send(write("https://evil.example", "from-evil"))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(write(
            "https://partner.example.evil.example",
            "from-lookalike"
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(write("https://partner.example", "from-partner"))
            .await
            .status(),
        StatusCode::CREATED
    );
}

// ---------------------------------------------------------------------------------------------
// The command line (the slop-api binary, run with only the environment each test gives it)

struct Cli {
    ok: bool,
    /// What it printed, and its log (tracing writes to standard output).
    out: String,
    /// The error it ended with, if any.
    err: String,
}

async fn cli(args: &[&str], env: &[(&str, &str)]) -> Cli {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_slop-api"));
    cmd.args(args)
        .env_clear()
        .env("RUST_LOG", "slop_api=info")
        .env("NO_COLOR", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(300), cmd.output())
        .await
        .expect("the command finished")
        .unwrap();
    Cli {
        ok: out.status.success(),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The first JSON value in a command's output, after any log lines.
fn json_in(out: &str) -> Value {
    let start = out.find("\n{").map_or(0, |i| i + 1);
    serde_json::Deserializer::from_str(&out[start..])
        .into_iter::<Value>()
        .next()
        .unwrap_or_else(|| panic!("no JSON in {out}"))
        .unwrap()
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// `KEY=value` from a command's output.
fn printed(out: &str, key: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("{key} in {out}"))
        .to_string()
}

#[tokio::test]
async fn generated_keys_round_trip_through_the_settings_that_use_them() {
    let first = cli(&["keys", "generate"], &[]).await;
    assert!(first.ok, "{}", first.err);
    let secret = printed(&first.out, "SLOP_SIGNING_KEY");
    let key = slop_manifest::signing_key_from_base64(&secret).unwrap();
    assert_eq!(
        printed(&first.out, "SLOP_TRUSTED_KEYS"),
        slop_manifest::verifying_key_to_base64(&key.verifying_key())
    );
    assert!(
        first.out.contains(&format!(
            "key id: {}",
            slop_manifest::key_id(&key.verifying_key())
        )),
        "{}",
        first.out
    );
    assert!(first.err.contains("Keep SLOP_SIGNING_KEY secret"));
    let second = cli(&["keys", "generate"], &[]).await;
    assert_ne!(
        printed(&second.out, "SLOP_SIGNING_KEY"),
        secret,
        "a new key each time"
    );

    let updater = cli(&["keys", "generate-updater"], &[]).await;
    assert!(updater.ok, "{}", updater.err);
    slop_api::desktop_updates::updater_key_from_base64(&printed(&updater.out, "SLOP_UPDATER_KEY"))
        .unwrap();
    assert!(updater.out.contains("updater pubkey: "));
    let vapid = cli(&["keys", "generate-vapid"], &[]).await;
    assert!(vapid.ok, "{}", vapid.err);
    slop_api::push::parse_key(&printed(&vapid.out, "SLOP_VAPID_PRIVATE_KEY")).unwrap();
    assert!(vapid.out.contains("public key: "));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_command_line_grants_admin_rights_by_id_and_takes_them_by_any_name(db: PgPool) {
    let app = test_app(db).await;
    let url = own_database_url(&app.db).await;
    let env = [("DATABASE_URL", url.as_str())];
    let (ada, _) = app.user("Ada", "US").await;
    let (sam1, _) = app.user("Sam", "US").await;
    let (sam2, _) = app.user("Sam", "GB").await;
    let ada_id = ada.to_string();

    // Granting takes the account ID: anyone can sign up under a display name.
    let run = cli(&["admin", "grant", "Ada", "--reviewer", "ops"], &env).await;
    assert!(
        !run.ok && run.err.contains("give the account's ID"),
        "{}",
        run.err
    );
    let run = cli(
        &[
            "admin",
            "grant",
            &Uuid::now_v7().to_string(),
            "--reviewer",
            "ops",
        ],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("no account matches"),
        "{}",
        run.err
    );
    let run = cli(&["admin", "grant", &ada_id, "--reviewer", "  "], &env).await;
    assert!(
        !run.ok && run.err.contains("say who you are"),
        "{}",
        run.err
    );
    let run = cli(&["admin", "grant", &ada_id], &env).await;
    assert!(!run.ok && run.err.contains("--reviewer"), "{}", run.err);
    assert_eq!(audit_count(&app, "admin.grant").await, 0);

    let run = cli(
        &[
            "admin",
            "grant",
            &format!(" {ada_id} "),
            "--reviewer",
            "ops",
        ],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains("Ada is now an admin."), "{}", run.out);
    let (is_admin, roles): (bool, Vec<String>) = sqlx::query_as(
        "select is_admin, array(select role from staff_roles where user_id = $1) from users where id = $1",
    )
    .bind(ada)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(is_admin);
    assert_eq!(roles, ["owner"]);
    let (actor, subject, detail): (String, String, Value) =
        sqlx::query_as("select actor, subject, detail from audit_log where action = 'admin.grant'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((actor.as_str(), subject.as_str()), ("ops", ada_id.as_str()));
    assert_eq!(detail["name"], "Ada");
    // Again: still one owner role.
    assert!(
        cli(&["admin", "grant", &ada_id, "--reviewer", "ops"], &env)
            .await
            .ok
    );
    let owners: i64 = sqlx::query_scalar("select count(*) from staff_roles where user_id = $1")
        .bind(ada)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owners, 1);

    // Taking them away: by ID, email or exact name, but never a guess between two.
    for id in [sam1, sam2] {
        admin::set_admin(&app.db, &id.to_string(), true, "test")
            .await
            .unwrap();
    }
    let run = cli(&["admin", "revoke", "Sam", "--reviewer", "ops"], &env).await;
    assert!(!run.ok, "{}", run.out);
    assert!(
        run.err.contains("2 accounts match")
            && run.err.contains(&sam1.to_string())
            && run.err.contains(&sam2.to_string()),
        "{}",
        run.err
    );
    let run = cli(&["admin", "revoke", "Nobody", "--reviewer", "ops"], &env).await;
    assert!(
        !run.ok && run.err.contains("no account matches"),
        "{}",
        run.err
    );
    sqlx::query("update users set email = 'Ada@Example.com' where id = $1")
        .bind(ada)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'security', 'test')",
    )
    .bind(ada)
    .execute(&app.db)
    .await
    .unwrap();
    let run = cli(
        &["admin", "revoke", "ada@example.com", "--reviewer", "ops"],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(
        run.out.contains("Ada is no longer an admin."),
        "{}",
        run.out
    );
    let (is_admin, roles): (bool, i64) = sqlx::query_as(
        "select is_admin, (select count(*) from staff_roles where user_id = $1) from users where id = $1",
    )
    .bind(ada)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((is_admin, roles), (false, 0), "every role goes");
    assert_eq!(audit_count(&app, "admin.revoke").await, 1);
    let run = cli(
        &["admin", "revoke", &sam2.to_string(), "--reviewer", "ops"],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    let still: bool = sqlx::query_scalar("select is_admin from users where id = $1")
        .bind(sam1)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(still, "only the one named");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_command_line_reviews_rejects_and_withdraws_releases(db: PgPool) {
    let app = test_app(db).await;
    let url = own_database_url(&app.db).await;
    let env = [("DATABASE_URL", url.as_str())];

    let run = cli(&["review", "pending"], &env).await;
    assert!(run.ok, "{}", run.err);
    assert!(
        run.out.contains("Nothing waiting for review."),
        "{}",
        run.out
    );
    assert!(
        run.out
            .contains("Note: scanning is off on this server; files are NOT scanned.")
    );

    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    sqlx::query("update releases set commit_verified = true where id = $1::uuid")
        .bind(&second)
        .execute(&app.db)
        .await
        .unwrap();
    let run = cli(&["review", "pending"], &env).await;
    let lines: Vec<&str> = run.out.lines().filter(|l| l.contains(" game ")).collect();
    let repo = format!("{}octo/game", app.fake_base);
    assert_eq!(
        lines,
        [
            format!("{first}  game 1.0.0  commit aaaaaaaaaaaa (NOT checked)  {repo}"),
            format!("{second}  game 1.1.0  commit aaaaaaaaaaaa (found in repo)  {repo}"),
        ],
        "{}",
        run.out
    );
    assert!(!run.out.contains("Nothing waiting"));

    // Rejecting takes a reviewer and a reason for the maker.
    let run = cli(
        &[
            "review",
            "reject",
            &first,
            "--reviewer",
            "ops",
            "--reason",
            "  ",
        ],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("give the maker a reason"),
        "{}",
        run.err
    );
    let run = cli(
        &[
            "review",
            "reject",
            &first,
            "--reviewer",
            "",
            "--reason",
            "Crashes.",
        ],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("say who you are"),
        "{}",
        run.err
    );
    let run = cli(
        &[
            "review",
            "reject",
            &first,
            "--reviewer",
            "ops",
            "--reason",
            " Crashes at start. ",
        ],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains(&format!("Rejected {first}.")));
    let (status, reason, by): (String, String, String) = sqlx::query_as(
        "select status::text, status_reason, reviewed_by from releases where id = $1::uuid",
    )
    .bind(&first)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), reason.as_str(), by.as_str()),
        ("rejected", "Crashes at start.", "ops")
    );
    assert_eq!(audit_count(&app, "release.reject").await, 1);
    assert!(notices(&app, maker_id, "%1.0.0%").await >= 1);
    let run = cli(
        &[
            "review",
            "reject",
            &first,
            "--reviewer",
            "ops",
            "--reason",
            "Again.",
        ],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("no release waiting for review"),
        "{}",
        run.err
    );

    // Withdrawing: only an approved release, with a reason players see; a pin to it goes.
    let run = cli(
        &["revoke", &second, "--reviewer", "ops", "--reason", "Miner."],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("no approved release with that ID"),
        "{}",
        run.err
    );
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        second.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    sqlx::query("update apps set pinned_release_id = $1::uuid where slug = 'game'")
        .bind(&second)
        .execute(&app.db)
        .await
        .unwrap();
    let run = cli(
        &["revoke", &second, "--reviewer", "ops", "--reason", " "],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("say why with --reason"),
        "{}",
        run.err
    );
    let run = cli(
        &[
            "revoke",
            &second,
            "--reviewer",
            "ops",
            "--reason",
            "Ships a miner.",
        ],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains("Clients stop launching it"), "{}", run.out);
    let (status, pinned): (String, Option<Uuid>) = sqlx::query_as(
        "select r.status::text, a.pinned_release_id from releases r join apps a on a.id = r.app_id where r.id = $1::uuid",
    )
    .bind(&second)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((status.as_str(), pinned), ("revoked", None));
    let (actor, detail): (String, Value) =
        sqlx::query_as("select actor, detail from audit_log where action = 'release.revoke'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(actor, "ops");
    assert_eq!(detail["reason"], "Ships a miner.");
    let run = cli(
        &["revoke", &second, "--reviewer", "ops", "--reason", "Again."],
        &env,
    )
    .await;
    assert!(!run.ok, "once");
    let run = cli(
        &["revoke", "not-an-id", "--reviewer", "ops", "--reason", "x"],
        &env,
    )
    .await;
    assert!(!run.ok && run.err.contains("invalid value"), "{}", run.err);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_command_line_moves_and_delists_apps(db: PgPool) {
    let app = test_app(db).await;
    let url = own_database_url(&app.db).await;
    let env = [("DATABASE_URL", url.as_str())];
    let (maker_id, _) = maker_with_app(&app, "beta").await;
    listed_with(&app, maker_id, "listed-one", "released", "").await;

    // Stages by their command-line names.
    let run = cli(
        &["review", "stage", "game", "alpha", "--reviewer", "ops"],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("use coming-soon, beta or released"),
        "{}",
        run.err
    );
    let run = cli(
        &["review", "stage", "game", "released", "--reviewer", "ops"],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("approve a release first"),
        "{}",
        run.err
    );
    let run = cli(
        &[
            "review",
            "stage",
            "game",
            "coming-soon",
            "--reviewer",
            "ops",
        ],
        &env,
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains("game moved to ComingSoon."), "{}", run.out);
    let run = cli(
        &[
            "review",
            "stage",
            "game",
            "coming_soon",
            "--reviewer",
            "ops",
        ],
        &env,
    )
    .await;
    assert!(
        !run.ok && run.err.contains("already in that stage"),
        "{}",
        run.err
    );
    let run = cli(
        &["review", "stage", "nope", "beta", "--reviewer", "ops"],
        &env,
    )
    .await;
    assert!(!run.ok && run.err.contains("no such app"), "{}", run.err);
    let stage: String = sqlx::query_scalar("select stage::text from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "coming_soon");
    assert_eq!(audit_count(&app, "app.stage").await, 1);

    // Delisting: a Content Policy rule, the facts, and only an app in the store.
    let delist = |slug: &'static str, rule: &'static str, reason: &'static str| async move {
        cli(
            &[
                "review",
                "delist",
                slug,
                "--reviewer",
                "ops",
                "--rule",
                rule,
                "--reason",
                reason,
            ],
            &env,
        )
        .await
    };
    let run = delist("listed-one", "ugly", "Copies another game's art.").await;
    assert!(
        !run.ok && run.err.contains("choose the rule it breaks"),
        "{}",
        run.err
    );
    let run = delist("listed-one", "copyright", " ").await;
    assert!(
        !run.ok && run.err.contains("say why with --reason"),
        "{}",
        run.err
    );
    let run = delist("game", "copyright", "Copies another game's art.").await;
    assert!(
        !run.ok && run.err.contains("no listed app with that address"),
        "{}",
        run.err
    );
    assert_eq!(audit_count(&app, "app.delist").await, 0);
    let run = delist("listed-one", "copyright", "Copies another game's art.").await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains("listed-one is out of the store."));
    let (status, action, rule, by): (String, String, String, String) = sqlx::query_as(
        "select a.status::text, d.action, d.rule, d.decided_by from apps a
         join moderation_decisions d on d.subject_id = a.id where a.slug = 'listed-one'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), action.as_str(), rule.as_str(), by.as_str()),
        ("delisted", "delist_app", "copyright", "ops")
    );
    assert_eq!(audit_count(&app, "app.delist").await, 1);
    assert_eq!(notices(&app, maker_id, "%listed-one%").await, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_builds_are_published_from_the_command_line_only_when_set_up(db: PgPool) {
    let app = test_app(db).await;
    let url = own_database_url(&app.db).await;
    let key = printed(
        &cli(&["keys", "generate-updater"], &[]).await.out,
        "SLOP_UPDATER_KEY",
    );
    let dir = temp_folder("desktop");
    let file = dir.join("upload.deb");
    std::fs::write(&file, b"a desktop build").unwrap();
    let out_dir = dir.join("published");
    let commit = "c".repeat(40);
    let publish = |version: &str| {
        vec![
            "desktop".to_string(),
            "publish".into(),
            "--platform".into(),
            "linux-deb".into(),
            "--version".into(),
            version.into(),
            "--commit".into(),
            commit.clone(),
            file.to_string_lossy().into_owned(),
        ]
    };

    let p = publish("1.2.0");
    let run = cli(&strs(&p), &[("DATABASE_URL", &url)]).await;
    assert!(
        !run.ok
            && run
                .err
                .contains("set SLOP_DESKTOP_DIR and SLOP_UPDATER_KEY first"),
        "{}",
        run.err
    );
    let run = cli(
        &strs(&p),
        &[
            ("DATABASE_URL", &url),
            ("SLOP_DESKTOP_DIR", out_dir.to_str().unwrap()),
        ],
    )
    .await;
    assert!(
        !run.ok && run.err.contains("set SLOP_DESKTOP_DIR"),
        "{}",
        run.err
    );
    let env = [
        ("DATABASE_URL", url.as_str()),
        ("SLOP_DESKTOP_DIR", out_dir.to_str().unwrap()),
        ("SLOP_UPDATER_KEY", key.as_str()),
    ];
    let bad = publish("1.x");
    let run = cli(&strs(&bad), &env).await;
    assert!(
        !run.ok && run.err.contains("unexpected character"),
        "{}",
        run.err
    );
    let run = cli(&strs(&p), &env).await;
    assert!(run.ok, "{}", run.err);
    assert!(run.out.contains("Published linux-deb 1.2.0"), "{}", run.out);
    let (version, size): (String, i64) = sqlx::query_as(
        "select version, size_bytes from desktop_builds where platform = 'linux-deb'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((version.as_str(), size), ("1.2.0", 15));
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn migrations_run_as_the_owner_and_a_stale_schema_is_refused(db: PgPool) {
    let url = own_database_url(&db).await;
    let run = cli(&["migrate"], &[("DATABASE_URL", &url)]).await;
    assert!(
        !run.ok
            && run
                .err
                .contains("SLOP_MIGRATE_DATABASE_URL (the tables' owner) isn't set"),
        "{}",
        run.err
    );
    let run = cli(
        &["migrate"],
        &[
            ("SLOP_MIGRATE_DATABASE_URL", &url),
            ("SLOP_APP_ROLE", "slop_admin2_no_such_role"),
        ],
    )
    .await;
    assert!(run.ok, "{}", run.err);
    assert!(
        run.out.contains("the API's role doesn't exist yet"),
        "{}",
        run.out
    );
    assert!(run.out.contains("migrations are up to date"));

    // Migrations that ran as their own step are only checked.
    let external = [
        ("DATABASE_URL", url.as_str()),
        ("SLOP_MIGRATIONS", "external"),
    ];
    let run = cli(&["review", "pending"], &external).await;
    assert!(run.ok, "{}", run.err);
    let newest: i64 = sqlx::query_scalar("delete from _sqlx_migrations where version = (select max(version) from _sqlx_migrations) returning version")
        .fetch_one(&db)
        .await
        .unwrap();
    let run = cli(&["review", "pending"], &external).await;
    assert!(!run.ok, "{}", run.out);
    assert!(
        run.err
            .contains(&format!("needs {newest}: run `slop-api migrate`")),
        "{}",
        run.err
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_simulation_commands_print_json_and_refuse_a_real_database(db: PgPool) {
    let url = own_database_url(&db).await;
    let run = cli(
        &[
            "simulate", "generate", "--apps", "30", "--users", "60", "--days", "20", "--seed", "5",
        ],
        &[("DATABASE_URL", &url)],
    )
    .await;
    assert!(run.ok, "{}", run.err);
    let made = json_in(&run.out);
    assert_eq!(
        (made["apps"].as_u64(), made["users"].as_u64()),
        (Some(30), Some(60)),
        "{made}"
    );
    assert!(made["vote_events"].as_u64().unwrap() > 0, "{made}");
    let apps: i64 = sqlx::query_scalar("select count(*) from sim_apps")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(apps, 30);
    let run = cli(
        &["simulate", "report", "--sample", "20", "--seed", "1"],
        &[("DATABASE_URL", &url)],
    )
    .await;
    assert!(run.ok, "{}", run.err);
    let report = json_in(&run.out);
    assert!(report["for_you"]["hit_rate"].is_number(), "{report}");
    assert_eq!(report["shown_dead"], 0);

    // A database that isn't a sandbox: refused after connecting, before anything is wiped.
    let name = "slop_admin2_real";
    let real = scratch_database(&db, name).await;
    slop_api::MIGRATOR.run(&real).await.unwrap();
    let (keep,): (Uuid,) = sqlx::query_as(
        "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, 'Real Person', 'US', now()) returning id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&real)
    .await
    .unwrap();
    let real_url = database_url(name);
    for args in [
        &["simulate", "generate", "--apps", "5", "--users", "5"][..],
        &["simulate", "report"][..],
    ] {
        let run = cli(args, &[("DATABASE_URL", &real_url)]).await;
        assert!(!run.ok, "{args:?}: {}", run.out);
        assert!(
            run.err
                .contains("the simulation only runs on a sandbox database")
                && run.err.contains(name),
            "{}",
            run.err
        );
    }
    let still: bool = sqlx::query_scalar("select exists (select 1 from users where id = $1)")
        .bind(keep)
        .fetch_one(&real)
        .await
        .unwrap();
    assert!(still);
    let refused =
        serve_until_listening(&[("DATABASE_URL", &real_url), ("SLOP_SIMULATION", "true")])
            .await
            .err()
            .expect("refused");
    assert!(
        refused.contains("SLOP_SIMULATION is only allowed on a sandbox database"),
        "{refused}"
    );
    drop_database(&db, real, name).await;
}

/// A running `slop-api serve`, stopped when dropped.
struct Server {
    _child: tokio::process::Child,
    base: String,
    log: std::sync::Arc<std::sync::Mutex<String>>,
}

impl Server {
    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

/// Starts `slop-api serve` on a free port with these settings (and no outside feeds), and waits
/// for it to listen. Err: what it said when it stopped instead.
async fn serve_until_listening(env: &[(&str, &str)]) -> Result<Server, String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let demos = temp_folder("demos");
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_slop-api"));
    cmd.arg("serve")
        .env_clear()
        .env("RUST_LOG", "slop_api=info")
        .env("NO_COLOR", "1")
        .env("SLOP_BIND", format!("127.0.0.1:{port}"))
        .env("SLOP_BAD_LINK_FEEDS", "off")
        .env("SLOP_VIES_URL", "off")
        .env("SLOP_DEMO_DIR", &demos)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let sink = log.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let mut log = sink.lock().unwrap();
            log.push_str(&line);
            log.push('\n');
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if log.lock().unwrap().contains("Slop API listening") {
            return Ok(Server {
                _child: child,
                base: format!("http://127.0.0.1:{port}"),
                log,
            });
        }
        if child.try_wait().unwrap().is_some() {
            let mut err = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut err)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            return Err(format!("{}{err}", log.lock().unwrap()));
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{}",
            log.lock().unwrap()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn repo_checked(db: &PgPool) -> bool {
    sqlx::query_scalar("select repo_checked_at is not null from apps where slug = 'watched'")
        .fetch_one(db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_server_checks_its_settings_and_starts_its_jobs(db: PgPool) {
    let app = test_app(db).await;
    let url = own_database_url(&app.db).await;
    let fake = app.fake_base.clone();
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "watched",
        "released",
        "repo_url = 'https://github.com/octo/game'",
    )
    .await;
    let (admin_id, _) = make_admin(&app, "Olive Owner").await;
    let base = [
        ("DATABASE_URL", url.as_str()),
        ("GITHUB_API_BASE", fake.as_str()),
        ("OSV_API_BASE", fake.as_str()),
    ];
    let with = |more: &[(&'static str, &'static str)]| -> Vec<(&str, &str)> {
        base.iter().copied().chain(more.iter().copied()).collect()
    };
    let client = reqwest::Client::new();

    // Simulated People payments never on a public (https) site.
    let refused = serve_until_listening(&with(&[
        ("SLOP_PUBLIC_URL", "https://slop.example"),
        ("SLOP_PAYMENTS", "simulated"),
    ]))
    .await
    .err()
    .expect("refused");
    assert!(
        refused.contains("SLOP_PAYMENTS=simulated is only for development servers"),
        "{refused}"
    );

    // The sandbox (this test database is one): simulated payments, and no repository checks,
    // since its repositories are invented.
    let sim = serve_until_listening(&with(&[
        ("SLOP_SIMULATION", "true"),
        ("SLOP_SECURITY_CONTACT", " https://slop.example/security "),
        ("SLOP_SOURCE_URL", "https://github.com/slop/slop/"),
    ]))
    .await
    .unwrap();
    let res = client
        .get(format!("{}/healthz", sim.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let txt = client
        .get(format!("{}/.well-known/security.txt", sim.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        txt.contains("Contact: https://slop.example/security\n"),
        "{txt}"
    );
    let source: Value = client
        .get(format!("{}/v1/source", sim.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(source["url"], "https://github.com/slop/slop");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(
        !repo_checked(&app.db).await,
        "no repository checks in the sandbox"
    );
    let log = sim.log();
    assert!(log.contains("simulation sandbox"), "{log}");
    assert!(!log.contains("People payments are off"), "{log}");
    drop(sim);

    // A public site: the proxy warning, repository checks, and each setting as read.
    let live = serve_until_listening(&with(&[
        ("SLOP_PUBLIC_URL", "https://slop.example"),
        ("SLOP_RATE_AUTH_PER_MIN", "0"),
        ("SLOP_APPROVALS_REQUIRED", "2"),
        ("SLOP_SECURITY_CONTACT", "http://slop.example/security"),
        ("SLOP_SOURCE_URL", "http://github.com/slop/slop"),
        (
            "SLOP_SOURCE_HOSTS",
            " Git.Example = gitea , codeberg.org=gitea",
        ),
    ]))
    .await
    .unwrap();
    let log = live.log();
    assert!(
        log.contains("Rate limits can't tell visitors apart"),
        "{log}"
    );
    assert!(log.contains("People payments are off"), "{log}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !repo_checked(&app.db).await {
        assert!(
            std::time::Instant::now() < deadline,
            "the repository check ran"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let get = |path: &str| client.get(format!("{}{path}", live.base)).send();
    let res = get("/healthz").await.unwrap();
    assert_eq!(
        res.headers()["strict-transport-security"],
        "max-age=63072000; includeSubDomains; preload"
    );
    // Only mailto: and https: contacts and https source links are used.
    assert_eq!(
        get("/.well-known/security.txt").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let source: Value = get("/v1/source").await.unwrap().json().await.unwrap();
    assert!(source["url"].is_null(), "{source}");
    // A rate limit of 0 is taken as 1.
    assert_ne!(
        get("/v1/auth/github/start").await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        get("/v1/auth/github/start").await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let mut https = app.config.clone();
    https.public_url = "https://slop.example".parse().unwrap();
    let cookie = session::create(&app.db, &https, admin_id, None)
        .await
        .unwrap();
    let actions: Value = client
        .get(format!("{}/v1/admin/actions", live.base))
        .header("cookie", format!("{}={}", cookie.name(), cookie.value()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(actions["approvals_required"], 2, "{actions}");
}

#[tokio::test]
async fn settings_that_cant_be_right_stop_the_server_before_it_starts() {
    let short_key = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 16]);
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![], "DATABASE_URL is required"),
        (
            vec![("SLOP_PUBLIC_URL", "https://slop.example/store")],
            "SLOP_PUBLIC_URL must be an origin with no path",
        ),
        (vec![("SLOP_PUBLIC_URL", "not a url")], "SLOP_PUBLIC_URL"),
        (
            vec![("SLOP_SIGNING_KEY", "not-a-key")],
            "SLOP_SIGNING_KEY must be a base64 32-byte Ed25519 key",
        ),
        (
            vec![("SLOP_FIELD_KEY", "%%%")],
            "SLOP_FIELD_KEY must be base64",
        ),
        (
            vec![("SLOP_FIELD_KEY", &short_key)],
            "SLOP_FIELD_KEY must be 32 bytes",
        ),
        (
            vec![("SLOP_REPO_TOKEN_KEY", &short_key)],
            "SLOP_REPO_TOKEN_KEY must be 32 bytes",
        ),
        (
            vec![("SLOP_APPROVALS_REQUIRED", "two")],
            "SLOP_APPROVALS_REQUIRED",
        ),
        (
            vec![("SLOP_APPROVALS_REQUIRED", "-1")],
            "SLOP_APPROVALS_REQUIRED",
        ),
        (
            vec![("SLOP_RATE_WRITE_PER_MIN", "lots")],
            "SLOP_RATE_WRITE_PER_MIN must be a number",
        ),
        (
            vec![("SLOP_SOURCE_HOSTS", "gitlab.example")],
            "isn't host=kind",
        ),
        (
            vec![("SLOP_SOURCE_HOSTS", "svn.example=svn")],
            "must be gitlab, gitea or bitbucket",
        ),
        (
            vec![("SLOP_UPDATER_KEY", "nope")],
            "SLOP_UPDATER_KEY isn't a valid updater key",
        ),
        (vec![("SLOP_BIND", "everywhere")], "SLOP_BIND"),
    ];
    // Nothing listens on port 1: a run that got past its settings would fail to connect instead.
    for (env, expected) in cases {
        let mut all = vec![];
        if !env.iter().any(|(k, _)| *k == "DATABASE_URL") && !expected.starts_with("DATABASE_URL") {
            all.push(("DATABASE_URL", "postgres://slop@127.0.0.1:1/nowhere"));
        }
        all.extend(env.iter().copied());
        let run = cli(&["serve"], &all).await;
        assert!(!run.ok, "{env:?}");
        assert!(run.err.contains(expected), "{env:?}: {}", run.err);
        assert!(
            !run.err.contains("connecting to Postgres"),
            "{env:?}: {}",
            run.err
        );
    }
    // And with nothing wrong, it gets as far as the database.
    let run = cli(
        &["serve"],
        &[
            ("DATABASE_URL", "postgres://slop@127.0.0.1:1/nowhere"),
            (
                "SLOP_FIELD_KEY",
                &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 32]),
            ),
            ("SLOP_APPROVALS_REQUIRED", "3"),
            ("SLOP_SOURCE_HOSTS", ""),
        ],
    )
    .await;
    assert!(
        !run.ok && run.err.contains("connecting to Postgres"),
        "{}",
        run.err
    );
}

// ---------------------------------------------------------------------------------------------
// Background jobs

/// Waits up to `secs` seconds for a condition read from the database.
async fn eventually<F, Fut>(secs: u64, what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while !check().await {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn release_state(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("select status::text from releases where id = $1::uuid")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_fetcher_wakes_for_new_releases_and_retakes_abandoned_ones(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let submit = |version: &str| {
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(release_body(&app, version, "game-windows.zip"))
    };

    // Another fetcher took it 10 minutes ago: still its lease, so it's left alone.
    let res = app.send(submit("1.0.0")).await;
    let first = body_json(res).await["id"].as_str().unwrap().to_string();
    sqlx::query("update releases set fetch_started_at = now() - interval '10 minutes'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(!releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_state(&app, &first).await, "fetching");
    // That fetcher died: after 30 minutes the release is taken again.
    sqlx::query("update releases set fetch_started_at = now() - interval '31 minutes'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_state(&app, &first).await, "awaiting_review");
    assert!(
        !releases::fetch_next(&app.state).await.unwrap(),
        "nothing left"
    );

    // The running fetcher, idle (it would next look in 30 seconds), wakes for a new release.
    let fetcher = tokio::spawn(releases::run_fetcher(app.state.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let res = app.send(submit("1.1.0")).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let second = body_json(res).await["id"].as_str().unwrap().to_string();
    eventually(10, "the woken fetcher fetched it", || async {
        release_state(&app, &second).await == "awaiting_review"
    })
    .await;
    fetcher.abort();
}

/// A bounty on the maker's app with two backers' saved (simulated) cards, delivered four days
/// ago: the bigger backer voted to accept, the other didn't vote. Returns (bounty, claim, builder).
async fn delivered_bounty(app: &TestApp) -> (Uuid, Uuid, Uuid) {
    let (_, maker) = maker_with_app(app, "released").await;
    let release = submit_and_fetch(app, &maker, "1.0.0").await;
    let (builder, _) = app.user("Bo Builder", "US").await;
    let (big, _) = app.user("Big Backer", "US").await;
    let (small, _) = app.user("Small Backer", "US").await;
    let (bounty, claim) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into bounties (id, app_id, created_by, title, body, currency, status, created_at)
         values ($1, $2, $3, 'Gamepad support', 'Please.', 'usd', 'claimed', now() - interval '10 days')",
    )
    .bind(bounty)
    .bind(app_id(app, "game").await)
    .bind(big)
    .execute(&app.db)
    .await
    .unwrap();
    for (backer, amount) in [(big, 3000), (small, 1000)] {
        sqlx::query(
            "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status, simulated, created_at)
             values ($1, $2, $3, $4, 'active', true, now() - interval '5 days')",
        )
        .bind(Uuid::now_v7())
        .bind(bounty)
        .bind(backer)
        .bind(amount)
        .execute(&app.db)
        .await
        .unwrap();
    }
    sqlx::query(
        "insert into bounty_claims (id, bounty_id, claimed_by, app_id, release_id, stripe_account_id, note, created_at)
         values ($1, $2, $3, $4, $5::uuid, 'acct_builder', 'Done.', now() - interval '4 days')",
    )
    .bind(claim)
    .bind(bounty)
    .bind(builder)
    .bind(app_id(app, "game").await)
    .bind(&release)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("insert into bounty_votes (claim_id, user_id, accept) values ($1, $2, true)")
        .bind(claim)
        .bind(big)
        .execute(&app.db)
        .await
        .unwrap();
    (bounty, claim, builder)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_charger_settles_votes_after_the_window_and_runs_one_at_a_time(db: PgPool) {
    let app = test_app(db).await;
    let (bounty, claim, builder) = delivered_bounty(&app).await;
    let pledges = || async {
        sqlx::query_scalar::<_, String>(
            "select string_agg(status, ',' order by amount_minor desc) from bounty_pledges",
        )
        .fetch_one(&app.db)
        .await
        .unwrap()
    };

    // Another copy of the API holds the charger's lock (a deploy overlapping the old one).
    let mut other = app.db.acquire().await.unwrap();
    sqlx::query("select pg_advisory_lock(7341001)")
        .execute(&mut *other)
        .await
        .unwrap();
    let charger = tokio::spawn(slop_api::bounties::run_charger(app.state.clone()));
    let claim_status = || async {
        sqlx::query_scalar::<_, String>("select status from bounty_claims where id = $1")
            .bind(claim)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    // Four days after the delivery the voting window has passed, but the maker has two weeks to
    // answer before the backers decide.
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert_eq!(claim_status().await, "pending");
    sqlx::query("update bounty_claims set created_at = now() - interval '15 days'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update bounty_pledges set created_at = now() - interval '16 days'")
        .execute(&app.db)
        .await
        .unwrap();
    app.state.bounty_wakeup.notify_one();
    // The vote is settled (more than half the pledged money said yes, the window passed)...
    eventually(10, "the claim was accepted", || async {
        claim_status().await == "accepted"
    })
    .await;
    let decided_by: String =
        sqlx::query_scalar("select decided_by from bounty_claims where id = $1")
            .bind(claim)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(decided_by, "backers");
    assert_eq!(
        notices(
            &app,
            builder,
            "Your delivery for \"Gamepad support\" was accepted"
        )
        .await,
        1
    );
    // ...but nothing is charged while the other copy holds the lock.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(pledges().await, "due,due");
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(pledges().await, "due,due");

    // Released: the next run charges each pledge once.
    sqlx::query("select pg_advisory_unlock(7341001)")
        .execute(&mut *other)
        .await
        .unwrap();
    drop(other);
    app.state.bounty_wakeup.notify_one();
    eventually(10, "the bounty was paid", || async {
        sqlx::query_scalar::<_, String>("select status from bounties where id = $1")
            .bind(bounty)
            .fetch_one(&app.db)
            .await
            .unwrap()
            == "paid"
    })
    .await;
    charger.abort();
    assert_eq!(pledges().await, "paid,paid");
    let paid: Vec<(i32, i32, String)> = sqlx::query_as(
        "select amount_minor, slop_fee_minor, stripe_account_id from payments where kind = 'bounty' order by amount_minor",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(paid.len(), 2, "{paid:?}");
    assert!(paid.iter().all(|(_, _, acct)| acct == "acct_builder"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_fill_job_records_repository_ids_for_apps_linked_before(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "old-game",
        "released",
        "repo_url = 'https://github.com/octo/game'",
    )
    .await;
    listed_with(
        &app,
        maker,
        "gone-game",
        "released",
        "repo_url = 'https://github.com/octo/gone-game'",
    )
    .await;
    listed_with(
        &app,
        maker,
        "lab-game",
        "released",
        "repo_url = 'https://gitlab.com/octo/game'",
    )
    .await;
    listed_with(
        &app,
        maker,
        "known-game",
        "released",
        "repo_url = 'https://github.com/octo/known', github_repo_id = 7, github_owner_id = 8",
    )
    .await;
    let ids = |slug: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
                "select github_repo_id, github_owner_id from apps where slug = $1",
            )
            .bind(slug)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };

    // Its first pass runs at once, then hourly.
    let fill = tokio::spawn(slop_api::repo_identity::run_fill(app.state.clone()));
    eventually(10, "old-game's IDs", || async {
        ids("old-game").await.0.is_some()
    })
    .await;
    fill.abort();
    assert_eq!(ids("old-game").await, (Some(1001), Some(42)));
    // A repository GitHub doesn't have, and one that isn't on GitHub: left empty, no error.
    assert_eq!(ids("gone-game").await, (None, None));
    assert_eq!(ids("lab-game").await, (None, None));
    // Recorded ones never change.
    assert_eq!(ids("known-game").await, (Some(7), Some(8)));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_pulse_sampler_counts_live_players_when_it_starts(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "live", "released", "").await;
    let live = app_id(&app, "live").await;
    for (i, beat) in ["now()", "now()", "now() - interval '2 hours'"]
        .iter()
        .enumerate()
    {
        let (player, _) = app.user(&format!("Player {i}"), "US").await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into pulse_sessions (id, token_hash, app_id, user_id, started_at, last_beat_at)
             values ($1, $2, $3, $4, now() - interval '3 hours', {beat})"
        )))
        .bind(Uuid::now_v7())
        .bind(format!("token-{i}").into_bytes())
        .bind(live)
        .bind(player)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let sampler = tokio::spawn(slop_api::pulse::run_sampler(app.state.clone()));
    eventually(10, "a count", || async {
        sqlx::query_scalar::<_, i64>("select count(*) from pulse_counts")
            .fetch_one(&app.db)
            .await
            .unwrap()
            > 0
    })
    .await;
    sampler.abort();
    let players: i32 = sqlx::query_scalar("select players from pulse_counts where app_id = $1")
        .bind(live)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(players, 2, "the quiet session isn't live");
}

/// A fake SMTP server: takes mail for anyone but refused@…, and keeps every line it was sent.
async fn fake_smtp() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = tokio::io::BufReader::new(read).lines();
                write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
                let mut data = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    log.lock().unwrap().push(line.clone());
                    let upper = line.to_ascii_uppercase();
                    let reply: &[u8] = if data {
                        if line != "." {
                            continue;
                        }
                        data = false;
                        b"250 queued\r\n"
                    } else if upper.starts_with("EHLO") {
                        b"250-fake\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n"
                    } else if upper.starts_with("AUTH") {
                        b"235 accepted\r\n"
                    } else if upper.starts_with("RCPT") && upper.contains("REFUSED@") {
                        b"550 no such mailbox\r\n"
                    } else if upper.starts_with("DATA") {
                        data = true;
                        b"354 go on\r\n"
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
    (port, seen)
}

/// A user with an email address and one notification waiting to be emailed.
async fn notified(app: &TestApp, name: &str, email: Option<&str>, prefs: &str) -> Uuid {
    let (id, _) = app.user(name, "US").await;
    sqlx::query("update users set email = $2, email_prefs = $3::jsonb where id = $1")
        .bind(id)
        .bind(email)
        .bind(prefs)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into notifications (id, user_id, category, title, body, link) values ($1, $2, 'moderation', $3, 'Details.', '/account#decisions')",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(format!("News for {name}"))
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// (attempts, sent, given up, last error) of the email to this address; none yet is (0, …).
async fn outbox(app: &TestApp, to: &str) -> (i32, bool, bool, Option<String>) {
    sqlx::query_as(
        "select attempts, sent_at is not null, failed_at is not null, last_error from email_outbox where to_address = $1",
    )
    .bind(to)
    .fetch_optional(&app.db)
    .await
    .unwrap()
    .unwrap_or((0, false, false, None))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn notification_emails_are_sent_retried_and_given_up(db: PgPool) {
    let (port, seen) = fake_smtp().await;
    let app = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: format!("smtp://127.0.0.1:{port}"),
            from: "Slop <notifications@slop.example>".into(),
        })
    })
    .await;
    let gwen = notified(&app, "Gwen", Some("gwen@example.com"), "{}").await;
    notified(&app, "Ray", Some("refused@example.com"), "{}").await;
    let quiet = notified(&app, "Quinn", None, "{}").await;
    let opted_out = notified(
        &app,
        "Otto",
        Some("otto@example.com"),
        r#"{"moderation": false}"#,
    )
    .await;
    // One that has failed four times already: this is its last try.
    sqlx::query(
        "insert into email_outbox (id, to_address, subject, body, attempts) values ($1, 'refused@old.example', 'Old', 'Old.', 4)",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();

    let job = tokio::spawn(slop_api::notify::run_email(app.state.clone()));
    eventually(15, "Gwen's email was sent", || async {
        outbox(&app, "gwen@example.com").await.1
    })
    .await;
    eventually(5, "the old one was tried", || async {
        outbox(&app, "refused@old.example").await.0 == 5
    })
    .await;
    job.abort();

    assert_eq!(
        outbox(&app, "gwen@example.com").await,
        (1, true, false, None)
    );
    let (attempts, sent, failed, error) = outbox(&app, "refused@example.com").await;
    assert_eq!(
        (attempts, sent, failed),
        (1, false, false),
        "tried again later"
    );
    assert!(error.unwrap().contains("no such mailbox"));
    let (attempts, sent, failed, _) = outbox(&app, "refused@old.example").await;
    assert_eq!(
        (attempts, sent, failed),
        (5, false, true),
        "given up after five"
    );
    let status = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "select email_status from notifications where user_id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(status(gwen).await, "queued");
    assert_eq!(status(quiet).await, "skipped", "no address");
    assert_eq!(status(opted_out).await, "skipped", "switched off");
    let lines = seen.lock().unwrap().join("\n");
    assert!(lines.contains("RCPT TO:<gwen@example.com>"), "{lines}");
    assert!(lines.contains("Subject: News for Gwen"), "{lines}");
    assert!(lines.contains("Hi Gwen,"), "{lines}");
    assert!(
        lines.contains("Open it: http://localhost:8080/account#decisions"),
        "{lines}"
    );
    assert!(
        lines.contains("From: Slop <notifications@slop.example>"),
        "{lines}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn email_stays_off_without_a_usable_smtp_address(db: PgPool) {
    for url in [None, Some("ftp://mail.example"), Some("not a url")] {
        let app = test_app_with(db.clone(), |c| {
            c.email = url.map(|u| slop_api::config::EmailConfig {
                smtp_url: u.into(),
                from: "Slop <notifications@slop.example>".into(),
            })
        })
        .await;
        let job = tokio::spawn(slop_api::notify::run_email(app.state.clone()));
        tokio::time::timeout(std::time::Duration::from_secs(5), job)
            .await
            .expect("it stops at once")
            .unwrap();
    }
    let app = test_app(db).await;
    let id = notified(&app, "Gwen", Some("gwen@example.com"), "{}").await;
    let status: String =
        sqlx::query_scalar("select email_status from notifications where user_id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "pending");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_smtp_password_never_crosses_the_network_in_the_clear(db: PgPool) {
    // smtp://user:password@host:587 is offered next to smtps:// (.env.example), and its doc
    // comment says STARTTLS: the password must not be sent to a server that can't do TLS.
    let (port, seen) = fake_smtp().await;
    let app = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: format!("smtp://slop:hunter2@127.0.0.1:{port}"),
            from: "Slop <notifications@slop.example>".into(),
        })
    })
    .await;
    notified(&app, "Gwen", Some("gwen@example.com"), "{}").await;
    let job = tokio::spawn(slop_api::notify::run_email(app.state.clone()));
    eventually(15, "a try", || async {
        sqlx::query_scalar::<_, i32>("select coalesce(max(attempts), 0) from email_outbox")
            .fetch_one(&app.db)
            .await
            .unwrap()
            > 0
    })
    .await;
    job.abort();
    let lines = seen.lock().unwrap().clone();
    assert!(!lines.is_empty(), "it connected");
    assert!(
        !lines
            .iter()
            .any(|l| l.to_ascii_uppercase().starts_with("AUTH")),
        "{lines:?}"
    );
    let (_, sent, _, error) = outbox(&app, "gwen@example.com").await;
    assert!(!sent, "nothing goes out without TLS");
    assert!(error.is_some());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn vat_numbers_stored_before_the_key_are_sealed_once(db: PgPool) {
    let key = [11u8; 32];
    let plain_app = test_app_with(db.clone(), |c| c.field_key = None).await;
    let mut users = vec![];
    for (name, vat) in [("Anna", "DE123456789"), ("Bert", "GB123456789")] {
        let (id, _) = plain_app.user(name, "DE").await;
        sqlx::query("update users set vat_number = $2 where id = $1")
            .bind(id)
            .bind(vat)
            .execute(&plain_app.db)
            .await
            .unwrap();
        users.push((id, vat));
    }
    let (sealed_id, _) = plain_app.user("Cleo", "DE").await;
    let already = slop_api::fields::seal(Some(&key), "users.vat_number", "FR12345678901").unwrap();
    sqlx::query("update users set vat_number = $2 where id = $1")
        .bind(sealed_id)
        .bind(&already)
        .execute(&plain_app.db)
        .await
        .unwrap();
    // Without the key there's nothing to do.
    assert_eq!(
        slop_api::fields::seal_existing(&plain_app.state)
            .await
            .unwrap(),
        0
    );

    let app = test_app(db).await;
    assert_eq!(app.config.field_key, Some(key));
    assert_eq!(
        slop_api::fields::seal_existing(&app.state).await.unwrap(),
        2
    );
    for (id, vat) in &users {
        let stored: String = sqlx::query_scalar("select vat_number from users where id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert!(
            stored.starts_with("enc1:") && !stored.contains(&vat[2..]),
            "{stored}"
        );
        assert_eq!(
            slop_api::fields::open(Some(&key), "users.vat_number", &stored).unwrap(),
            *vat
        );
    }
    let untouched: String = sqlx::query_scalar("select vat_number from users where id = $1")
        .bind(sealed_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(untouched, already);
    assert_eq!(
        slop_api::fields::seal_existing(&app.state).await.unwrap(),
        0,
        "once"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_push_job_does_nothing_without_a_push_key(db: PgPool) {
    let app = test_app(db).await;
    assert!(app.config.push.is_none());
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::spawn(slop_api::push::run(app.state.clone())),
    )
    .await
    .expect("it stops at once")
    .unwrap();
}

/// A fake Stripe that lists yesterday's application fees, two to a page.
async fn fake_fee_list() -> String {
    let fees = |axum::extract::Query(q): axum::extract::Query<
        std::collections::HashMap<String, String>,
    >| async move {
        let day = OffsetDateTime::now_utc().replace_time(time::Time::MIDNIGHT);
        assert_eq!(
            q["created[gte]"],
            (day - time::Duration::days(1)).unix_timestamp().to_string()
        );
        assert_eq!(q["created[lt]"], day.unix_timestamp().to_string());
        match q.get("starting_after").map(String::as_str) {
            None => axum::Json(json!({ "has_more": true, "data": [
                { "id": "fee_1", "account": "acct_a", "amount": 50, "amount_refunded": 0 },
                { "id": "fee_2", "account": "acct_b", "amount": 40, "amount_refunded": 10 }
            ] })),
            Some("fee_2") => axum::Json(json!({ "has_more": false, "data": [
                { "id": "fee_3", "account": "acct_c", "amount": 20, "amount_refunded": 0 }
            ] })),
            Some(other) => panic!("starting_after={other}"),
        }
    };
    let app = Router::new().route("/v1/application_fees", get(fees));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn daily_reconciliation_flags_accounts_whose_fees_dont_match_the_records(db: PgPool) {
    // Without Stripe there's nothing to compare.
    let plain = test_app(db.clone()).await;
    assert_eq!(
        slop_api::donations::reconcile(&plain.state).await.unwrap(),
        0
    );

    let stripe = fake_fee_list().await;
    let app = test_app_with(db, |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: "whsec_test".into(),
            api_base: stripe.parse().unwrap(),
        })
    })
    .await;
    // Yesterday's records: a matches Stripe; b has less than Stripe kept; c has none. A
    // simulated payment and one from today don't count.
    for (acct, fee, simulated, when) in [
        ("acct_a", 50, false, "now()::date - interval '12 hours'"),
        ("acct_b", 20, false, "now()::date - interval '1 hour'"),
        ("acct_d", 15, true, "now()::date - interval '2 hours'"),
        ("acct_a", 99, false, "now()::date + interval '1 minute'"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into payments (id, kind, stripe_account_id, amount_minor, slop_fee_minor, currency, simulated, created_at)
             values ($1, 'donation', $2, 1000, $3, 'usd', $4, {when})"
        )))
        .bind(Uuid::now_v7())
        .bind(acct)
        .bind(fee)
        .bind(simulated)
        .execute(&app.db)
        .await
        .unwrap();
    }
    assert_eq!(slop_api::donations::reconcile(&app.state).await.unwrap(), 2);
    let (actor, subject, detail): (String, String, Value) = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'payments.mismatch'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(actor, "reconciliation");
    let yesterday = (OffsetDateTime::now_utc() - time::Duration::days(1))
        .date()
        .to_string();
    assert_eq!(subject, yesterday);
    let mut accounts: Vec<Value> = detail["accounts"].as_array().unwrap().clone();
    accounts.sort_by_key(|a| a[0].as_str().unwrap().to_string());
    assert_eq!(
        accounts,
        [json!(["acct_b", 30, 20]), json!(["acct_c", 20, 0])]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn official_badges_are_rechecked_daily_and_removed_when_a_check_fails(db: PgPool) {
    let app = test_app_with(db, |config| {
        config.releases.github_app = Some(slop_api::config::GithubAppConfig {
            app_id: "123".into(),
            slug: "slop-test".into(),
            private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                "../../fixtures/github-app-test-key.pem"
            ))
            .unwrap(),
            webhook_secret: "hook-secret".into(),
        });
    })
    .await;
    let (owner_id, owner) = app.user("Ada Acme", "US").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', '42', $1)",
    )
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/teams").cookie(&owner).json(json!({ "slug": "acme", "name": "Acme Games", "kind": "official", "github_org": "acme" })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    // Verified ten days ago, last checked two days ago. Its domain no longer checks out.
    sqlx::query(
        "update teams set verified_at = now() - interval '10 days', checked_at = now() - interval '2 days' where slug = 'acme'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    // Another official team, checked an hour ago: not due.
    let (other_owner, _) = app.user("Bea Beta", "US").await;
    let other = Uuid::now_v7();
    sqlx::query(
        "insert into teams (id, slug, name, kind, created_by, github_org, verified_at, checked_at)
         values ($1, 'beta-studio', 'Beta Studio', 'official', $2, 'globex', now() - interval '3 days', now() - interval '1 hour')",
    )
    .bind(other)
    .bind(other_owner)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("insert into team_members (team_id, user_id, role) values ($1, $2, 'owner')")
        .bind(other)
        .bind(other_owner)
        .execute(&app.db)
        .await
        .unwrap();

    let job = tokio::spawn(slop_api::teams::run(app.state.clone()));
    // Checked, and its owner told: the notice comes after the check is saved.
    eventually(15, "acme was checked and its owner told", || async {
        sqlx::query_scalar::<_, bool>(
            "select checked_at > now() - interval '1 minute' from teams where slug = 'acme'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap()
            && notices(&app, owner_id, "Your team's official badge was removed").await > 0
    })
    .await;
    job.abort();
    let (verified, checks): (bool, Value) =
        sqlx::query_as("select verified_at is not null, checks from teams where slug = 'acme'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!verified, "{checks}");
    let domain = checks
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "domain")
        .unwrap()
        .clone();
    assert_eq!(domain["ok"], false);
    assert_eq!(
        notices(&app, owner_id, "Your team's official badge was removed").await,
        1
    );
    let (verified, recent): (bool, bool) = sqlx::query_as(
        "select verified_at is not null, checked_at < now() - interval '50 minutes' from teams where id = $1",
    )
    .bind(other)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(verified && recent, "not due, not checked");
    assert_eq!(notices(&app, other_owner, "%badge%").await, 0);
}
