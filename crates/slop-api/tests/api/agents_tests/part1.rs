// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 1 of the agents tests: admin API tokens, the review feed, the agent's account routes and
//! the admin MCP endpoint with its tools.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// An app whose rate limits don't get in the way of a long test (every request comes from one
/// address), with this many approvals for sensitive actions.
async fn agents_app(db: PgPool, approvals: u8) -> TestApp {
    test_app_with(db, |c| {
        c.approvals_required = approvals;
        c.rate_limits.write_per_minute = 10_000;
        c.rate_limits.read_per_minute = 10_000;
        c.rate_limits.auth_per_minute = 10_000;
    })
    .await
}

/// A staff member with exactly these roles (none: staff who can only look).
async fn staff(app: &TestApp, name: &str, roles: &[&str]) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    for role in roles {
        sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
            .bind(id)
            .bind(role)
            .execute(&app.db)
            .await
            .unwrap();
    }
    (id, cookie)
}

/// An admin API token made with this staff member's cookie, as an Authorization header value.
async fn admin_token(app: &TestApp, cookie: &str, scope: &str) -> String {
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

/// A request with an admin token.
fn with_token(r: Req, auth: &str) -> Req {
    r.header("authorization", auth.to_string())
}

/// One JSON-RPC request to the admin MCP endpoint; the whole reply.
async fn mcp(app: &TestApp, auth: &str, body: Value) -> Value {
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/admin/mcp").json(body),
            auth,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// Calls one admin MCP tool: (whether it worked, the text it gave back).
async fn tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    let reply = mcp(
        app,
        auth,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": args } }),
    )
    .await;
    let result = &reply["result"];
    assert!(result.is_object(), "{name}: {reply}");
    (
        result["isError"] == false,
        result["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

/// A tool call that must work; its text parsed as JSON (null for an empty reply).
async fn tool_ok(app: &TestApp, auth: &str, name: &str, args: Value) -> Value {
    let (ok, text) = tool(app, auth, name, args).await;
    assert!(ok, "{name}: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// A tool call that must be refused; the text says why.
async fn tool_refused(app: &TestApp, auth: &str, name: &str, args: Value) -> String {
    let (ok, text) = tool(app, auth, name, args).await;
    assert!(!ok, "{name} should be refused: {text}");
    text
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn audit_actors(app: &TestApp, action: &str) -> Vec<String> {
    sqlx::query_scalar("select actor from audit_log where action = $1 order by id")
        .bind(action)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

async fn notice_titles(app: &TestApp, user: Uuid) -> Vec<String> {
    sqlx::query_scalar("select title from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

async fn is_suspended(app: &TestApp, user: Uuid) -> bool {
    sqlx::query_scalar("select suspended_at is not null from users where id = $1")
        .bind(user)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// Suspends an account the way staff do (one approval is enough in these apps).
async fn suspend(app: &TestApp, by_cookie: &str, user: Uuid) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{user}/decide"))
                .cookie(by_cookie)
                .json(json!({ "action": "suspend_account", "rule": "harassment",
                              "facts": "Sent the same insult to twelve people in a day." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(is_suspended(app, user).await);
}

// ---------------------------------------------------------------------------------------------
// Admin API tokens

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_security_staff_signed_in_themselves_list_admin_tokens(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (_, sam) = staff(&app, "Sam Security", &["security"]).await;
    let (_, player) = app.user("Pat Player", "US").await;
    let ada_read = admin_token(&app, &ada, "read").await;
    let ben_review = admin_token(&app, &ben, "review").await;

    // Not staff, or a token: the route doesn't exist. Staff without the security role: refused.
    for r in [
        req(Method::GET, "/v1/admin/tokens"),
        req(Method::GET, "/v1/admin/tokens").cookie(&player),
        with_token(req(Method::GET, "/v1/admin/tokens"), &ada_read),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    for (name, roles) in [
        ("Rex Reviewer", &["reviewer"][..]),
        ("Mo Moderator", &["moderator"]),
        ("Sue Support", &["support"]),
        ("Fin Finance", &["finance"]),
        ("Nora Nobody", &[]),
    ] {
        let (_, cookie) = staff(&app, name, roles).await;
        assert_eq!(
            app.send(req(Method::GET, "/v1/admin/tokens").cookie(&cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{name}"
        );
    }

    // Security staff see everyone's tokens, without their secrets.
    let res = app
        .send(req(Method::GET, "/v1/admin/tokens").cookie(&sam))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    let text = list.to_string();
    for secret in [&ada_read, &ben_review] {
        assert!(!text.contains(secret.trim_start_matches("Bearer ")));
    }
    let tokens = list.as_array().unwrap();
    assert_eq!(tokens.len(), 2);
    let admins: Vec<&str> = tokens
        .iter()
        .map(|t| t["admin"].as_str().unwrap())
        .collect();
    assert!(admins.contains(&"Ada Owner") && admins.contains(&"Ben Owner"));
    for t in tokens {
        assert!(t.get("token").is_none() && t.get("token_hash").is_none());
        assert!(t["expires_at"].is_string() && t["created_at"].is_string());
        assert!(t["revoked_at"].is_null());
        // Ada's was used above (even on a route it may not call); Ben's never was.
        assert_eq!(
            t["last_used_at"].is_null(),
            t["admin"] == "Ben Owner",
            "{t}"
        );
    }

    // A revoked one sorts last, with when.
    let ben_id = tokens.iter().find(|t| t["admin"] == "Ben Owner").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/admin/tokens/{ben_id}/revoke")).cookie(&sam))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    // A newer token, so "revoked last" isn't just "oldest last".
    admin_token(&app, &ada, "review").await;
    let list = body_json(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&sam))
            .await,
    )
    .await;
    let tokens = list.as_array().unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[2]["id"], ben_id.as_str());
    assert!(tokens[2]["revoked_at"].is_string());
    assert!(tokens[..2].iter().all(|t| t["revoked_at"].is_null()));
    let used = tokens.iter().find(|t| t["scope"] == "read").unwrap();
    assert!(used["last_used_at"].is_string(), "{used}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn making_an_admin_token_checks_the_person_the_name_the_scope_and_the_days(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (ada_id, ada) = make_admin(&app, "Ada Owner").await;
    let (_, rex) = staff(&app, "Rex Reviewer", &["reviewer", "moderator"]).await;
    let make = |cookie: &str, body: Value| {
        req(Method::POST, "/v1/admin/tokens")
            .cookie(cookie)
            .json(body)
    };

    // Staff without the security role can't.
    assert_eq!(
        app.send(make(&rex, json!({ "name": "Claude", "scope": "read" })))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    // An old session must sign in again or confirm with a passkey first.
    sqlx::query("update sessions set created_at = now() - interval '1 hour' where user_id = $1")
        .bind(ada_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(make(&ada, json!({ "name": "Claude", "scope": "read" })))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "step_up_required");
    assert_eq!(count(&app, "select count(*) from admin_tokens").await, 0);
    sqlx::query("update sessions set stepped_up_at = now() where user_id = $1")
        .bind(ada_id)
        .execute(&app.db)
        .await
        .unwrap();

    // Bad input, field by field.
    for (body, field) in [
        (json!({ "name": "", "scope": "read" }), "name"),
        (json!({ "name": "   ", "scope": "read" }), "name"),
        (json!({ "name": "x".repeat(61), "scope": "read" }), "name"),
        (json!({ "name": "Clau\u{7}de", "scope": "read" }), "name"),
        (json!({ "name": "Claude", "scope": "admin" }), "scope"),
        (
            json!({ "name": "Claude", "scope": "read", "days": 0 }),
            "days",
        ),
        (
            json!({ "name": "Claude", "scope": "review", "days": 366 }),
            "days",
        ),
        (
            json!({ "name": "Claude", "scope": "moderate", "days": 91 }),
            "days",
        ),
        (
            json!({ "name": "Claude", "scope": "moderate", "days": -1 }),
            "days",
        ),
    ] {
        let res = app.send(make(&ada, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{body}");
    }
    let res = app
        .send(make(
            &ada,
            json!({ "name": "Claude", "scope": "read", "admin": true }),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown fields"
    );
    assert_eq!(count(&app, "select count(*) from admin_tokens").await, 0);

    // The limits themselves are fine; days default to 90.
    for (body, days) in [
        (
            json!({ "name": "x".repeat(60), "scope": "read", "days": 365 }),
            365,
        ),
        (
            json!({ "name": "Moderator", "scope": "moderate", "days": 90 }),
            90,
        ),
        (json!({ "name": "  Default  ", "scope": "review" }), 90),
    ] {
        let res = app.send(make(&ada, body.clone())).await;
        assert_eq!(res.status(), StatusCode::CREATED, "{body}");
        let made = body_json(res).await;
        let created =
            OffsetDateTime::parse(made["created_at"].as_str().unwrap(), &Rfc3339).unwrap();
        let expires =
            OffsetDateTime::parse(made["expires_at"].as_str().unwrap(), &Rfc3339).unwrap();
        assert_eq!((expires - created).whole_days(), days, "{body}");
        assert_eq!(made["admin"], "Ada Owner");
        // Slop keeps only the hash.
        let secret = made["token"].as_str().unwrap();
        assert!(secret.starts_with("slopadm_"));
        let stored: Vec<u8> =
            sqlx::query_scalar("select token_hash from admin_tokens where id = $1")
                .bind(made["id"].as_str().unwrap().parse::<Uuid>().unwrap())
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(stored, slop_api::tokens::hash_token(secret));
        assert_ne!(stored, secret.as_bytes());
    }
    let name: String = sqlx::query_scalar("select name from admin_tokens where scope = 'review'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(name, "Default", "the name is trimmed");

    // Each is in the audit log, with its scope and days.
    let rows: Vec<(String, String, Value)> = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'admin_token.create' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].0.starts_with("Ada Owner ("), "{rows:?}");
    assert_eq!(rows[1].1, "Moderator");
    assert_eq!(rows[1].2["scope"], "moderate");
    assert_eq!(rows[2].2["days"], 90);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn security_staff_revoke_any_admin_token_once(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, sam) = staff(&app, "Sam Security", &["security"]).await;
    let (_, rex) = staff(&app, "Rex Reviewer", &["reviewer"]).await;
    let ada_moderate = admin_token(&app, &ada, "moderate").await;
    let id: Uuid = sqlx::query_scalar("select id from admin_tokens")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let path = format!("/v1/admin/tokens/{id}/revoke");

    // A token can't revoke tokens (not even itself); staff without the role can't either.
    assert_eq!(
        app.send(with_token(req(Method::POST, &path), &ada_moderate))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::POST, &path).cookie(&rex))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/tokens/{}/revoke", Uuid::now_v7())
            )
            .cookie(&sam)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/admin/tokens/not-a-uuid/revoke").cookie(&sam))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let pong = mcp(
        &app,
        &ada_moderate,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
    )
    .await;
    assert_eq!(pong["result"], json!({}));

    // Someone else with the security role revokes Ada's token: logged, and it stops working
    // everywhere, MCP included.
    assert_eq!(
        app.send(req(Method::POST, &path).cookie(&sam))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::POST, &path).cookie(&sam))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "only once"
    );
    let rows: Vec<(String, String, Value)> = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'admin_token.revoke'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.starts_with("Sam Security ("));
    assert_eq!(rows[0].1, "Agent moderate");
    assert_eq!(rows[0].2["token"], id.to_string());
    for r in [
        req(Method::POST, "/v1/admin/mcp")
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })),
        req(Method::GET, "/v1/admin/status"),
    ] {
        assert_eq!(
            app.send(with_token(r, &ada_moderate)).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn admin_tokens_expire_and_stop_with_their_admins_staff_rights_or_account(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (ada_id, ada) = make_admin(&app, "Ada Owner").await;
    let (ben_id, ben) = make_admin(&app, "Ben Owner").await;
    let token = admin_token(&app, &ada, "read").await;
    let id: Uuid = sqlx::query_scalar("select id from admin_tokens")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let status = |auth: String| with_token(req(Method::GET, "/v1/admin/status"), &auth);
    let last_used = || async {
        sqlx::query_scalar::<_, Option<OffsetDateTime>>(
            "select last_used_at from admin_tokens where id = $1",
        )
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };

    // First use is recorded; within a minute it isn't written again; after that it is.
    assert!(last_used().await.is_none());
    assert_eq!(
        app.send(status(token.clone())).await.status(),
        StatusCode::OK
    );
    let first = last_used().await.unwrap();
    app.send(status(token.clone())).await;
    assert_eq!(last_used().await.unwrap(), first);
    sqlx::query("update admin_tokens set last_used_at = now() - interval '2 minutes'")
        .execute(&app.db)
        .await
        .unwrap();
    let old = last_used().await.unwrap();
    app.send(status(token.clone())).await;
    assert!(last_used().await.unwrap() > old);

    // Only "Bearer <token>" counts, extra spaces aside.
    let secret = token.trim_start_matches("Bearer ").to_string();
    assert_eq!(
        app.send(status(format!("Bearer   {secret} ")))
            .await
            .status(),
        StatusCode::OK
    );
    for header in [
        secret.clone(),
        format!("bearer {secret}"),
        format!("Token {secret}"),
    ] {
        assert_eq!(
            app.send(status(header.clone())).await.status(),
            StatusCode::NOT_FOUND,
            "{header}"
        );
    }

    // It acts for its admin on the admin API only: elsewhere it's nobody.
    let res = app
        .send(with_token(req(Method::GET, "/v1/me"), &token))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Notes and the audit log name the token and the admin.
    listed_with(&app, ada_id, "robot", "released", "").await;
    let listing = app_id(&app, "robot").await;
    let res = app
        .send(with_token(
            req(
                Method::POST,
                format!("/v1/admin/review/listing/{listing}/notes"),
            )
            .json(json!({ "note": "Looks fine." })),
            &token,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        body_json(res).await["author"],
        format!("AI reviewer \"Agent read\" for Ada Owner ({ada_id}, token {id})")
    );

    // Expired.
    sqlx::query("update admin_tokens set expires_at = now() - interval '1 minute'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(status(token.clone())).await.status(),
        StatusCode::NOT_FOUND
    );

    // Taken off staff: every token of theirs is revoked.
    let ada_token = admin_token(&app, &ada, "review").await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{ada_id}/remove"))
                .cookie(&ben)
                .json(json!({ "reason": "Left the team last week." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(status(ada_token)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(
            &app,
            &format!(
                "select count(*) from admin_tokens where user_id = '{ada_id}' and revoked_at is null"
            )
        )
        .await,
        0
    );

    // Closing the account deletes them.
    let ben_token = admin_token(&app, &ben, "read").await;
    assert_eq!(
        app.send(status(ben_token.clone())).await.status(),
        StatusCode::OK
    );
    slop_api::account::close_account(&app.db, ben_id, "test")
        .await
        .unwrap();
    assert_eq!(
        app.send(status(ben_token)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from admin_tokens where user_id = '{ben_id}'")
        )
        .await,
        0
    );
}

/// What each permission level looks like from outside: a request that needs it, and whether the
/// answer means it was let through (it then fails for its own reasons: nothing to act on).
fn perm_probes() -> Vec<(&'static str, Method, String, Option<Value>, StatusCode)> {
    let nobody = Uuid::now_v7();
    vec![
        (
            "read",
            Method::GET,
            "/v1/admin/status".into(),
            None,
            StatusCode::OK,
        ),
        (
            "review",
            Method::POST,
            format!("/v1/admin/releases/{nobody}/approve"),
            None,
            StatusCode::CONFLICT,
        ),
        (
            "moderate",
            Method::GET,
            format!("/v1/admin/users/{nobody}"),
            None,
            StatusCode::NOT_FOUND,
        ),
        (
            "sensitive",
            Method::POST,
            format!("/v1/admin/users/{nobody}/lift-suspension"),
            Some(json!({ "reason": "They apologised to everyone." })),
            StatusCode::CONFLICT,
        ),
        (
            "finance",
            Method::GET,
            "/v1/admin/money".into(),
            None,
            StatusCode::OK,
        ),
        (
            "security",
            Method::GET,
            "/v1/admin/tokens".into(),
            None,
            StatusCode::OK,
        ),
    ]
}

/// Which levels a caller gets through, as the probe names.
async fn levels(app: &TestApp, caller: impl Fn(Req) -> Req) -> Vec<&'static str> {
    let mut out = vec![];
    for (name, method, path, body, through) in perm_probes() {
        let mut r = req(method, path);
        if let Some(b) = body {
            r = r.json(b);
        }
        let status = app.send(caller(r)).await.status();
        if status == through {
            out.push(name);
        } else {
            // Refused: forbidden, or (for tokens on token management) not there at all.
            assert!(
                status == StatusCode::FORBIDDEN
                    || (name == "security" && status == StatusCode::NOT_FOUND),
                "{name}: {status}"
            );
        }
    }
    out
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn token_scopes_and_staff_roles_cap_each_other(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;

    // Signed in themselves, staff get what their roles allow.
    for (roles, want) in [
        (
            &["owner"][..],
            &[
                "read",
                "review",
                "moderate",
                "sensitive",
                "finance",
                "security",
            ][..],
        ),
        (&["moderator"], &["read", "review", "moderate", "sensitive"]),
        (&["reviewer"], &["read", "review"]),
        (&["support"], &["read", "moderate"]),
        (&["finance"], &["read", "finance"]),
        (&["security"], &["read", "security"]),
        (&[], &["read"]),
    ] {
        let (_, cookie) = staff(&app, &format!("Staff {roles:?}"), roles).await;
        assert_eq!(levels(&app, |r| r.cookie(&cookie)).await, want, "{roles:?}");
    }

    // An owner's tokens get what their scope allows, and never money or tokens.
    for (scope, want) in [
        ("read", &["read"][..]),
        ("review", &["read", "review", "moderate"]),
        ("moderate", &["read", "review", "moderate", "sensitive"]),
    ] {
        let token = admin_token(&app, &owner, scope).await;
        assert_eq!(
            levels(&app, |r| with_token(r, &token)).await,
            want,
            "{scope}"
        );
    }

    // A token is capped by its admin's roles too, as they are when it's used.
    let (rex_id, rex) = staff(&app, "Rex Reviewer", &["security", "reviewer"]).await;
    let rex_token = admin_token(&app, &rex, "moderate").await;
    assert_eq!(
        levels(&app, |r| with_token(r, &rex_token)).await,
        ["read", "review"]
    );
    let (_, sue) = staff(&app, "Sue Support", &["security", "support"]).await;
    let sue_token = admin_token(&app, &sue, "moderate").await;
    assert_eq!(
        levels(&app, |r| with_token(r, &sue_token)).await,
        ["read", "moderate"]
    );
    sqlx::query("delete from staff_roles where user_id = $1")
        .bind(rex_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(levels(&app, |r| with_token(r, &rex_token)).await, ["read"]);

    // Steps that need a person signed in lately never take a token, whatever its scope.
    let owner_moderate = admin_token(&app, &owner, "moderate").await;
    let res = app
        .send(with_token(
            req(Method::POST, format!("/v1/admin/staff/{rex_id}/remove"))
                .json(json!({ "reason": "Testing what a review token may do." })),
            &admin_token(&app, &owner, "review").await,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let pending = app
        .send(with_token(
            req(Method::POST, format!("/v1/admin/staff/{rex_id}/roles"))
                .json(json!({ "role": "support", "grant": true })),
            &owner_moderate,
        ))
        .await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED, "an agent only asks");
    let action = body_json(pending).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(with_token(
            req(Method::POST, format!("/v1/admin/actions/{action}/approve")),
            &owner_moderate,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from staff_roles where user_id = '{rex_id}'")
        )
        .await,
        0
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_suspended_staff_members_token_stops_working_and_nobody_frees_themselves(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (ada_id, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (troll_id, _) = app.user("Troll Account", "US").await;
    let token = admin_token(&app, &ada, "moderate").await;

    // Nobody lets themselves publish without a passkey, or lifts their own suspension.
    let res = app
        .send(with_token(
            req(
                Method::POST,
                format!("/v1/admin/users/{ada_id}/passkey-grace"),
            )
            .json(json!({ "reason": "I checked my own ID card, honestly." })),
            &token,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{ada_id}/passkey-grace"),
            )
            .cookie(&ada)
            .json(json!({ "reason": "I checked my own ID card, honestly." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(&app, "select count(*) from staff_actions").await, 0);

    // Ben suspends Ada. Her agent can't act for her any more: not on others, not to free her.
    suspend(&app, &ben, ada_id).await;
    let res = app
        .send(with_token(
            req(
                Method::POST,
                format!("/v1/admin/users/{ada_id}/lift-suspension"),
            )
            .json(json!({ "reason": "This was all a misunderstanding." })),
            &token,
        ))
        .await;
    assert!(
        matches!(
            res.status(),
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
        ),
        "{}",
        res.status()
    );
    assert!(is_suspended(&app, ada_id).await);
    let res = app
        .send(with_token(
            req(Method::POST, format!("/v1/admin/users/{troll_id}/decide")).json(json!({
                "action": "warn", "rule": "harassment", "facts": "Ada's agent, after she was suspended."
            })),
            &token,
        ))
        .await;
    assert!(
        matches!(res.status(), StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{}",
        res.status()
    );
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from moderation_decisions where user_id = '{troll_id}'")
        )
        .await,
        0
    );
    // Over MCP neither.
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/admin/mcp").json(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "lift_suspension", "arguments": { "id": ada_id.to_string(), "reason": "This was all a misunderstanding." } }
            })),
            &token,
        ))
        .await;
    if res.status() == StatusCode::OK {
        assert_eq!(body_json(res).await["result"]["isError"], true);
    }
    assert!(is_suspended(&app, ada_id).await);
    // Signed in herself she can't either (a suspended account only reads).
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{ada_id}/lift-suspension"),
            )
            .cookie(&ada)
            .json(json!({ "reason": "This was all a misunderstanding." })),
        )
        .await;
    assert!(!res.status().is_success());
    assert!(is_suspended(&app, ada_id).await);

    // Ben lifts it. Admin rights end with a suspension and don't come back on their own, so
    // her token stays dead until someone makes her staff again.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{ada_id}/lift-suspension"),
            )
            .cookie(&ben)
            .json(json!({ "reason": "Checked: the reports were a misunderstanding." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!is_suspended(&app, ada_id).await);
    assert_eq!(
        app.send(with_token(req(Method::GET, "/v1/admin/status"), &token))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Even an admin whose suspension was set some other way (not through a decision, which
    // also ends admin rights) can't use a token while suspended.
    sqlx::query("update users set is_admin = true, suspended_at = now() where id = $1")
        .bind(ada_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(with_token(req(Method::GET, "/v1/admin/status"), &token))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query("update users set suspended_at = null where id = $1")
        .bind(ada_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(with_token(req(Method::GET, "/v1/admin/status"), &token))
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_review_feed_says_who_may_decide(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let can_decide = |r: Req| async {
        let res = app.send(r).await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["can_decide"].as_bool().unwrap()
    };
    let feed = || req(Method::GET, "/v1/admin/review");
    for (roles, want) in [
        (&["owner"][..], true),
        (&["reviewer"], true),
        (&["moderator"], true),
        // Support decides People problems and review disputes in the feed.
        (&["support"], true),
        (&["finance"], false),
        (&["security"], false),
        (&[], false),
    ] {
        let (_, cookie) = staff(&app, &format!("Staff {roles:?}"), roles).await;
        assert_eq!(can_decide(feed().cookie(&cookie)).await, want, "{roles:?}");
    }
    for (scope, want) in [("read", false), ("review", true), ("moderate", true)] {
        let token = admin_token(&app, &owner, scope).await;
        assert_eq!(
            can_decide(with_token(feed(), &token)).await,
            want,
            "{scope}"
        );
    }
    // A moderate token of someone who may only look decides nothing.
    let (sid, sam) = staff(&app, "Sam Security", &["security"]).await;
    let token = admin_token(&app, &sam, "moderate").await;
    assert!(!can_decide(with_token(feed(), &token)).await);
    let _ = sid;
}

// ---------------------------------------------------------------------------------------------
// The review feed and notes

/// One of everything the review feed shows, by kind.
struct Waiting {
    release: String,
    /// Moving `game` to another repository, holding it at a version, and `other` following its
    /// newest approved release.
    repo_change: Uuid,
    hold_change: Uuid,
    follow_change: Uuid,
    media: Uuid,
    listing: Uuid,
    portfolio: Uuid,
    problem: Uuid,
    /// A review dispute is keyed by its job.
    dispute_job: Uuid,
}

/// A People job between a creative and a client, about an app.
async fn people_job(
    app: &TestApp,
    creative: Uuid,
    client: Uuid,
    app_id: Uuid,
    title: &str,
) -> Uuid {
    sqlx::query(
        "insert into creatives (user_id, handle, name, headline, ai_use, trader)
         values ($1, 'cora', 'Cora Creative', 'Pixel art', 'none', false) on conflict do nothing",
    )
    .bind(creative)
    .execute(&app.db)
    .await
    .unwrap();
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md, offer)
         values ($1, $2, $3, $4, 'pixel_art', 'in_progress', 'Twelve sprites, please.', $5)",
    )
    .bind(id)
    .bind(creative)
    .bind(client)
    .bind(app_id)
    .bind(json!({ "title": title }))
    .execute(&app.db)
    .await
    .unwrap();
    id
}

async fn seed_waiting(app: &TestApp) -> Waiting {
    let (maker_id, maker) = maker_with_app(app, "beta").await;
    let release = submit_and_fetch(app, &maker, "1.0.0").await;
    let game = app_id(app, "game").await;
    listed_with(app, maker_id, "other", "released", "status = 'draft'").await;
    let other = app_id(app, "other").await;
    let change = |app_id: Uuid, kind: &str, release: Option<Uuid>, repo: Option<&str>| {
        let id = Uuid::now_v7();
        let (kind, repo) = (kind.to_string(), repo.map(str::to_string));
        async move {
            sqlx::query(
                "insert into app_changes (id, app_id, kind, release_id, repo_url, note, requested_by)
                 values ($1, $2, $3, $4, $5, 'Please.', $6)",
            )
            .bind(id)
            .bind(app_id)
            .bind(kind)
            .bind(release)
            .bind(repo)
            .bind(maker_id)
            .execute(&app.db)
            .await
            .unwrap();
            id
        }
    };
    let repo_change = change(game, "repo", None, Some("https://github.com/octo/new-home")).await;
    let hold_change = change(game, "release", Some(release.parse().unwrap()), None).await;
    let follow_change = change(other, "release", None, None).await;
    // Pictures on a third app (pending pictures keep an app off the listing list).
    listed_with(app, maker_id, "pictured", "coming_soon", "status = 'draft'").await;
    let media = Uuid::now_v7();
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, fingerprint, source) values ($1, $2, $3, 'fp', 'maker')",
    )
    .bind(media)
    .bind(app_id(app, "pictured").await)
    .bind("d".repeat(40))
    .execute(&app.db)
    .await
    .unwrap();
    // Ready to list: a coming-soon draft.
    listed_with(app, maker_id, "soon", "coming_soon", "status = 'draft'").await;
    let listing = app_id(app, "soon").await;
    // A creative's portfolio, a problem on a job, and a disputed review.
    let (cora, _) = app.user("Cora Creative", "US").await;
    let (cliff, _) = app.user("Cliff Client", "US").await;
    let job = people_job(app, cora, cliff, game, "Sprites for Game").await;
    let portfolio = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, source)
         values ($1, $2, 'https://github.com/cora/portfolio', $3, '[]', 'fp', 'creative')",
    )
    .bind(portfolio)
    .bind(cora)
    .bind("e".repeat(40))
    .execute(&app.db)
    .await
    .unwrap();
    let problem = Uuid::now_v7();
    sqlx::query(
        "insert into job_problems (id, job_id, opened_by, statement) values ($1, $2, $3, 'Half the sprites are missing.')",
    )
    .bind(problem)
    .bind(job)
    .bind(cliff)
    .execute(&app.db)
    .await
    .unwrap();
    let dispute_job = people_job(app, cora, cliff, game, "Music for Game").await;
    sqlx::query(
        "insert into creative_reviews (job_id, creative_id, client_id, vote, body) values ($1, $2, $3, -1, 'Never delivered.')",
    )
    .bind(dispute_job)
    .bind(cora)
    .bind(cliff)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into review_disputes (job_id, reason, statement) values ($1, 'false_fact', 'I delivered on time.')",
    )
    .bind(dispute_job)
    .execute(&app.db)
    .await
    .unwrap();
    Waiting {
        release,
        repo_change,
        hold_change,
        follow_change,
        media,
        listing,
        portfolio,
        problem,
        dispute_job,
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_review_feed_shows_every_kind_with_the_requests_that_decide_it(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let w = seed_waiting(&app).await;
    let read = admin_token(&app, &owner, "read").await;
    let feed = |query: &str| {
        let r = with_token(req(Method::GET, format!("/v1/admin/review{query}")), &read);
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await
        }
    };

    // Staff only.
    for r in [
        req(Method::GET, "/v1/admin/review"),
        req(Method::GET, "/v1/admin/review").cookie(&player),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }

    let all = feed("").await;
    assert_eq!(all["guide"], "/v1/admin/review/guide");
    assert_eq!(
        all["checks"],
        json!({ "scanning": false, "maker_signatures_required": false })
    );
    let items = all["items"].as_array().unwrap();
    let kinds: Vec<&str> = items.iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "release",
            "change",
            "change",
            "change",
            "media",
            "listing",
            "portfolio",
            "problem",
            "review_dispute"
        ]
    );
    let item = |id: &str| {
        items
            .iter()
            .find(|i| i["id"] == id)
            .unwrap_or_else(|| panic!("{id} in {all}"))
            .clone()
    };

    // A release: its files and checks, approve or reject with a reason.
    let release = item(&w.release);
    assert_eq!(release["question"], "Approve version 1.0.0 of Game?");
    assert_eq!(release["app"]["slug"], "game");
    assert_eq!(
        release["app"]["store_url"],
        "http://localhost:8080/app/game"
    );
    assert_eq!(release["detail"]["id"], w.release.as_str());
    assert_eq!(release["detail"]["files"][0]["platform"], "windows");
    assert_eq!(
        release["approve"],
        json!({ "method": "POST", "path": format!("/v1/admin/releases/{}/approve", w.release) })
    );
    assert_eq!(
        release["reject"]["path"],
        format!("/v1/admin/releases/{}/reject", w.release)
    );
    assert!(release["reject"]["body"]["reason"].is_string());
    assert_eq!(
        release["note"]["path"],
        format!("/v1/admin/review/release/{}/notes", w.release)
    );

    // Changes: each of the three questions, and what players get now.
    let repo = item(&w.repo_change.to_string());
    assert_eq!(
        repo["question"],
        "Move Game to https://github.com/octo/new-home?"
    );
    assert_eq!(
        repo["detail"]["repo_url"],
        "https://github.com/octo/new-home"
    );
    assert!(
        repo["detail"]
            .as_object()
            .unwrap()
            .contains_key("players_get_now")
    );
    assert_eq!(
        repo["approve"]["path"],
        format!("/v1/admin/changes/{}/approve", w.repo_change)
    );
    let hold = item(&w.hold_change.to_string());
    assert_eq!(hold["question"], "Hold Game players at version 1.0.0?");
    assert_eq!(hold["detail"]["release_version"], "1.0.0");
    let follow = item(&w.follow_change.to_string());
    assert_eq!(
        follow["question"],
        "Have other follow its newest approved release?"
    );
    assert_eq!(
        follow["reject"]["path"],
        format!("/v1/admin/changes/{}/reject", w.follow_change)
    );

    // Pictures, a listing (no "no": leaving it unlisted is the no), a portfolio.
    let media = item(&w.media.to_string());
    assert_eq!(media["app"]["slug"], "pictured");
    assert!(media["detail"]["shown_now"].is_null());
    let listing = item(&w.listing.to_string());
    assert_eq!(listing["question"], "Put soon in the store?");
    assert_eq!(listing["approve"]["path"], "/v1/admin/apps/soon/list");
    assert!(listing.get("reject").is_none());
    assert_eq!(listing["detail"]["approved_releases"], 0);
    let portfolio = item(&w.portfolio.to_string());
    assert!(portfolio.get("app").is_none());
    assert_eq!(portfolio["creative"]["handle"], "cora");
    assert_eq!(portfolio["creative"]["page"], "/people/cora");

    // A problem on a job: suggest an outcome, or close it unresolved.
    let problem = item(&w.problem.to_string());
    let question = problem["question"].as_str().unwrap();
    assert!(
        question.contains("\"Sprites for Game\"")
            && question.contains("Cora Creative and Cliff Client"),
        "{question}"
    );
    assert_eq!(
        problem["approve"]["path"],
        format!("/v1/admin/problems/{}/suggest", w.problem)
    );
    assert!(problem["approve"]["body"]["suggestion"].is_string());
    assert_eq!(
        problem["reject"]["path"],
        format!("/v1/admin/problems/{}/close", w.problem)
    );
    assert!(problem["reject"]["body"]["note"].is_string());
    assert!(
        problem["creative"]["record"]
            .as_str()
            .unwrap()
            .ends_with("/record")
    );
    let dispute = item(&w.dispute_job.to_string());
    assert_eq!(
        dispute["approve"]["path"],
        format!("/v1/admin/review-disputes/{}/decide", w.dispute_job)
    );

    // One kind at a time; an unknown kind is simply nothing.
    for (kind, n) in [
        ("release", 1),
        ("change", 3),
        ("media", 1),
        ("listing", 1),
        ("portfolio", 1),
        ("problem", 1),
        ("review_dispute", 1),
        ("nonsense", 0),
    ] {
        let only = feed(&format!("?kind={kind}")).await;
        let items = only["items"].as_array().unwrap();
        assert_eq!(items.len(), n, "{kind}");
        assert!(items.iter().all(|i| i["kind"] == kind), "{kind}");
    }

    // At most 100 of a kind.
    let (maker_id, _) = app.user("Prolific Maker", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx)
         select gen_random_uuid(), 'bulk-' || n, 'Bulk ' || n, 'One of many', 'game', 'coming_soon', 'draft', $1,
                'https://github.com/x/y', 'MIT'
         from generate_series(1, 101) n",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let listings = feed("?kind=listing").await;
    assert_eq!(listings["items"].as_array().unwrap().len(), 100);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn review_notes_go_on_every_kind_and_check_what_they_say(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let w = seed_waiting(&app).await;
    let read = admin_token(&app, &owner, "read").await;
    let note = |kind: &str, id: &str, body: Value| {
        with_token(
            req(Method::POST, format!("/v1/admin/review/{kind}/{id}/notes")).json(body),
            &read,
        )
    };

    // Each kind takes notes (a read token is enough); they come back with the item.
    let subjects = [
        ("release", w.release.clone()),
        ("change", w.repo_change.to_string()),
        ("media", w.media.to_string()),
        ("listing", w.listing.to_string()),
        ("portfolio", w.portfolio.to_string()),
        ("problem", w.problem.to_string()),
        ("review_dispute", w.dispute_job.to_string()),
    ];
    for (kind, id) in &subjects {
        let res = app
            .send(note(
                kind,
                id,
                json!({ "verdict": "concern", "note": format!("  A worry about this {kind}.  ") }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED, "{kind}");
        let saved = body_json(res).await;
        assert_eq!(saved["note"], format!("A worry about this {kind}."));
        assert_eq!(saved["verdict"], "concern");
    }
    let all = body_json(
        app.send(with_token(req(Method::GET, "/v1/admin/review"), &read))
            .await,
    )
    .await;
    for (kind, id) in &subjects {
        let item = all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id.as_str())
            .unwrap();
        assert_eq!(item["notes"][0]["verdict"], "concern", "{kind}");
        assert!(
            item["notes"][0]["author"]
                .as_str()
                .unwrap()
                .starts_with("AI reviewer \"Agent read\"")
        );
    }

    // A verdict is optional; the longest note fits.
    let res = app
        .send(note(
            "release",
            &w.release,
            json!({ "note": "n".repeat(4000) }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert!(body_json(res).await["verdict"].is_null());

    // Refused: unknown kinds, subjects that aren't there (or are another kind), bad verdicts and
    // notes, unknown fields, bad IDs, people who aren't staff.
    let media = w.media.to_string();
    for (kind, id, body, status) in [
        (
            "app",
            media.as_str(),
            json!({ "note": "x" }),
            StatusCode::NOT_FOUND,
        ),
        (
            "release",
            media.as_str(),
            json!({ "note": "x" }),
            StatusCode::NOT_FOUND,
        ),
        (
            "review_dispute",
            &w.problem.to_string(),
            json!({ "note": "x" }),
            StatusCode::NOT_FOUND,
        ),
        (
            "media",
            media.as_str(),
            json!({ "verdict": "maybe", "note": "x" }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "media",
            media.as_str(),
            json!({ "note": "   " }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "media",
            media.as_str(),
            json!({ "note": "n".repeat(4001) }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "media",
            media.as_str(),
            json!({ "note": "x", "score": 3 }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "media",
            "not-a-uuid",
            json!({ "note": "x" }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let res = app.send(note(kind, id, body.clone())).await;
        assert_eq!(res.status(), status, "{kind} {id} {body}");
    }
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/review/media/{media}/notes"),
            )
            .cookie(&player)
            .json(json!({ "note": "x" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(count(&app, "select count(*) from review_notes").await, 8);
}

// ---------------------------------------------------------------------------------------------
// Status and accounts

/// Files a report about an app; its ID.
async fn report(app: &TestApp, reporter: &str, slug: &str, reason: &str) -> Uuid {
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(reporter)
                .json(report_app(slug, reason)),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let reference = body_json(res).await["reference"]
        .as_str()
        .unwrap()
        .to_string();
    sqlx::query_scalar("select id from reports where reference = $1")
        .bind(reference)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A folder of its own under the system's temporary folder.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("slop-agents-{name}-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_status_counts_what_waits_and_reads_the_ops_folders(db: PgPool) {
    let backups = temp_dir("backups");
    let deploys = temp_dir("deploys");
    std::fs::write(backups.join("last-success"), "2026-10-21T03:00:00Z\n").unwrap();
    std::fs::write(backups.join("last-restore-check"), "2026-10-18T04:30:00Z").unwrap();
    let build = deploys.join("0123456789abcdef0123456789abcdef01234567");
    std::fs::create_dir_all(&build).unwrap();
    std::os::unix::fs::symlink(&build, deploys.join("current")).unwrap();
    let app = test_app_with(db, |c| {
        c.rate_limits.write_per_minute = 10_000;
        c.ops.backup_dir = Some(backups.clone());
        c.ops.deploy_dir = Some(deploys.clone());
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (troll_id, troll) = app.user("Troll Account", "US").await;
    let token = admin_token(&app, &owner, "read").await;
    let w = seed_waiting(&app).await;
    let status = || async {
        let res = app
            .send(with_token(req(Method::GET, "/v1/admin/status"), &token))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await
    };
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/status").cookie(&troll))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Reports by priority, one overdue; an appeal; an abuse signal; alerts, one resolved.
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "miner", "released", "").await;
    let mut reports = vec![];
    for (i, reason) in ["malware", "scam", "broken"].iter().enumerate() {
        let (_, reporter) = app.user(&format!("Reporter {i}"), "US").await;
        reports.push(report(&app, &reporter, "miner", reason).await);
    }
    for (id, priority) in reports.iter().zip([0, 1, 2]) {
        sqlx::query("update reports set priority = $2 where id = $1")
            .bind(id)
            .bind(priority as i16)
            .execute(&app.db)
            .await
            .unwrap();
    }
    sqlx::query("update reports set due_at = now() - interval '1 hour' where id = $1")
        .bind(reports[2])
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{troll_id}/decide"))
                .cookie(&owner)
                .json(json!({ "action": "warn", "rule": "harassment", "facts": "Insults in reviews." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let decision: Uuid = sqlx::query_scalar("select id from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(&troll)
                .json(json!({ "appeal": "Those were jokes between friends." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    sqlx::query(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, 'vote_burst', 'vote_burst:miner', 'app', $2, 'miner', '{}')",
    )
    .bind(Uuid::now_v7())
    .bind(app_id(&app, "miner").await)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into ops_alerts (key, title, detail, link, resolved_at) values
         ('disk-full', 'The disk is nearly full', '95% used', '/admin', null),
         ('backup-stale', 'The nightly backup hasn''t run', 'Old', '/admin', now())",
    )
    .execute(&app.db)
    .await
    .unwrap();

    let s = status().await;
    assert_eq!(
        s["waiting"],
        json!({
            "releases_to_review": 1, "releases_failed_this_week": 0, "changes": 3, "pictures": 1,
            "portfolios": 1, "reports_urgent": 1, "reports_serious": 1, "reports_other": 1,
            "reports_overdue": 1, "appeals": 1, "live_releases_flagged_by_rescan": 0,
            "abuse_signals": 1, "apps_ready_to_list": 0
        }),
        "{s}"
    );
    assert_eq!(s["alerts"].as_array().unwrap().len(), 1);
    assert_eq!(s["alerts"][0]["key"], "disk-full");
    assert_eq!(s["alerts"][0]["title"], "The disk is nearly full");
    assert_eq!(s["alerts"][0]["detail"], "95% used");
    assert!(s["alerts"][0]["since"].is_number());
    assert_eq!(s["last_backup"], "2026-10-21T03:00:00Z");
    assert_eq!(s["last_restore_check"], "2026-10-18T04:30:00Z");
    assert_eq!(s["deployed"], "0123456789ab");
    assert_eq!(
        (&s["scanning"], &s["signing_key"], &s["github_app"]),
        (&json!(false), &json!(true), &json!(false))
    );
    assert_eq!(s["apps_listed"], 1);
    assert_eq!(s["approved_last_day"], 0);
    let accounts = s["accounts"].as_i64().unwrap();

    // The release is approved; then a re-scan flags it, and the app is ready to list. A failed
    // release counts for the week; a closed account doesn't count.
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        w.release.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    sqlx::query("update release_files set rescan_flagged_at = now()")
        .execute(&app.db)
        .await
        .unwrap();
    // A copy of it that failed its checks.
    let mut tx = app.db.begin().await.unwrap();
    for sql in [
        "create temp table failed_copy on commit drop as select * from releases limit 1",
        "update failed_copy set id = gen_random_uuid(), version = '0.9.0', status = 'failed'",
        "insert into releases select * from failed_copy",
    ] {
        sqlx::query(sql).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    slop_api::account::close_account(&app.db, troll_id, "test")
        .await
        .unwrap();
    let s = status().await;
    assert_eq!(s["waiting"]["releases_to_review"], 0);
    assert_eq!(s["waiting"]["releases_failed_this_week"], 1);
    assert_eq!(s["waiting"]["live_releases_flagged_by_rescan"], 1);
    assert_eq!(s["waiting"]["apps_ready_to_list"], 1);
    assert_eq!(s["approved_last_day"], 1);
    assert_eq!(s["accounts"].as_i64().unwrap(), accounts - 1);

    // Without the ops folders, those are simply unknown.
    std::fs::remove_file(backups.join("last-success")).unwrap();
    std::fs::write(backups.join("last-restore-check"), "not a date").unwrap();
    std::fs::remove_file(deploys.join("current")).unwrap();
    let s = status().await;
    assert!(
        s["last_backup"].is_null() && s["last_restore_check"].is_null() && s["deployed"].is_null()
    );
    let _ = std::fs::remove_dir_all(backups);
    let _ = std::fs::remove_dir_all(deploys);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn finding_accounts_matches_names_handles_and_ids_literally(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let token = admin_token(&app, &owner, "read").await;
    let find = |q: &str| with_token(req(Method::GET, format!("/v1/admin/users?q={q}")), &token);
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|u| u["display_name"].as_str().unwrap().to_string())
            .collect()
    };

    // Bad searches.
    for q in ["", "%20%20", &"x".repeat(101)] {
        let res = app.send(find(q)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{q:?}");
    }
    assert_eq!(
        app.send(with_token(req(Method::GET, "/v1/admin/users"), &token))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.send(find(&"x".repeat(100))).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/users?q=Pat").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // By ID, by People handle; % and _ are only themselves.
    let (zed, _) = app.user("Zed", "US").await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, headline, ai_use, trader)
         values ($1, 'pixelqueen', 'Zed', '', 'none', false)",
    )
    .bind(zed)
    .execute(&app.db)
    .await
    .unwrap();
    app.user("100% Real", "US").await;
    app.user("Under_score", "US").await;
    app.user("UnderXscore", "US").await;
    let by_id = body_json(app.send(find(&zed.to_string())).await).await;
    assert_eq!(names(&by_id), ["Zed"]);
    assert_eq!(by_id[0]["handle"], "pixelqueen");
    assert_eq!(
        names(&body_json(app.send(find("XELque")).await).await),
        ["Zed"]
    );
    assert_eq!(
        names(&body_json(app.send(find("%25")).await).await),
        ["100% Real"]
    );
    assert_eq!(
        names(&body_json(app.send(find("r_s")).await).await),
        ["Under_score"]
    );

    // At most 20, newest first, with what an agent needs to know about each.
    for i in 1..=22 {
        app.user(&format!("Clone {i:02}"), "US").await;
    }
    let clones = body_json(app.send(find("clone")).await).await;
    let got = names(&clones);
    assert_eq!(got.len(), 20);
    assert_eq!(got[0], "Clone 22");
    assert_eq!(got[19], "Clone 03");
    let (sus, _) = app.user("Suspended Sid", "US").await;
    suspend(&app, &owner, sus).await;
    let (gone, _) = app.user("Closed Cal", "US").await;
    slop_api::account::close_account(&app.db, gone, "test")
        .await
        .unwrap();
    let flags = |v: Value| {
        (
            v[0]["suspended"].as_bool().unwrap(),
            v[0]["closed"].as_bool().unwrap(),
            v[0]["is_admin"].as_bool().unwrap(),
        )
    };
    assert_eq!(
        flags(body_json(app.send(find("Sid")).await).await),
        (true, false, false)
    );
    assert_eq!(
        flags(body_json(app.send(find("Cal")).await).await),
        (false, true, false)
    );
    assert_eq!(
        flags(body_json(app.send(find("Olive")).await).await),
        (false, false, true)
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reading_an_account_needs_moderation_rights_and_is_logged_each_time(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (mia, _) = app.user("Mia Maker", "DE").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', 'gh-subject-4242', $1)",
    )
    .bind(mia)
    .execute(&app.db)
    .await
    .unwrap();
    listed_with(&app, mia, "miner", "released", "").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    report(&app, &rita, "miner", "malware").await;
    // Two decisions about her, one later reversed.
    for facts in ["Insults in reviews.", "More insults."] {
        app.send(
            req(Method::POST, format!("/v1/admin/users/{mia}/decide"))
                .cookie(&owner)
                .json(json!({ "action": "warn", "rule": "harassment", "facts": facts })),
        )
        .await;
    }
    sqlx::query(
        "update moderation_decisions set reversed_at = now() where facts = 'Insults in reviews.'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let path = format!("/v1/admin/users/{mia}");

    // Who may: moderation rights, and a review or moderate token.
    let read = admin_token(&app, &owner, "read").await;
    assert_eq!(
        app.send(with_token(req(Method::GET, &path), &read))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for roles in [&["reviewer"][..], &["finance"], &["security"], &[]] {
        let (_, cookie) = staff(&app, &format!("Staff {roles:?}"), roles).await;
        assert_eq!(
            app.send(req(Method::GET, &path).cookie(&cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{roles:?}"
        );
    }
    let (_, sue) = staff(&app, "Sue Support", &["support"]).await;
    assert_eq!(
        app.send(req(Method::GET, &path).cookie(&sue))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(with_token(
            req(Method::GET, format!("/v1/admin/users/{}", Uuid::now_v7())),
            &admin_token(&app, &owner, "review").await
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from audit_log where action = 'user.view'"
        )
        .await,
        1
    );

    // What it shows: sign-in providers (not the accounts), apps, reports, decisions.
    let review = admin_token(&app, &owner, "review").await;
    let res = app.send(with_token(req(Method::GET, &path), &review)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let d = body_json(res).await;
    assert_eq!(d["display_name"], "Mia Maker");
    assert_eq!(d["country_code"], "DE");
    assert_eq!(d["sign_ins"], json!(["github"]));
    assert!(!d.to_string().contains("gh-subject-4242"));
    assert_eq!(d["apps"][0]["slug"], "miner");
    assert_eq!(d["apps"][0]["status"], "listed");
    assert_eq!(d["reports_about"][0]["reason"], "malware");
    assert_eq!(d["reports_about"][0]["about"], "app");
    let decisions = d["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0]["reversed"], false);
    assert_eq!(decisions[1]["reversed"], true);
    assert_eq!(decisions[1]["rule"], "harassment");
    assert!(d["suspended_at"].is_null() && d["closed_at"].is_null());

    // Each read is logged, under whoever read it.
    app.send(with_token(req(Method::GET, &path), &review)).await;
    let actors: Vec<(String, String)> = sqlx::query_as(
        "select actor, subject from audit_log where action = 'user.view' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(actors.len(), 3);
    assert!(actors[0].0.starts_with("Sue Support ("));
    assert!(
        actors[1]
            .0
            .starts_with("AI reviewer \"Agent review\" for Olive Owner")
    );
    assert!(
        actors
            .iter()
            .all(|(_, subject)| *subject == mia.to_string())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn warning_or_suspending_an_account_checks_who_what_and_why(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (olive_id, owner) = make_admin(&app, "Olive Owner").await;
    let (_, sue) = staff(&app, "Sue Support", &["support"]).await;
    let (_, rex) = staff(&app, "Rex Reviewer", &["reviewer"]).await;
    let (troll, _) = app.user("Troll Account", "US").await;
    let (gone, _) = app.user("Closed Cal", "US").await;
    slop_api::account::close_account(&app.db, gone, "test")
        .await
        .unwrap();
    let read = admin_token(&app, &owner, "read").await;
    let moderate = admin_token(&app, &owner, "moderate").await;
    let decide = |who: Uuid, body: Value| {
        req(Method::POST, format!("/v1/admin/users/{who}/decide")).json(body)
    };
    let warn =
        json!({ "action": "warn", "rule": "harassment", "facts": "Insults in three reviews." });
    let decisions = || count(&app, "select count(*) from moderation_decisions");

    // Bad input, field by field.
    for (body, field) in [
        (
            json!({ "action": "ban", "rule": "harassment", "facts": "x" }),
            "action",
        ),
        (
            json!({ "action": "warn", "rule": "rudeness", "facts": "x" }),
            "rule",
        ),
        (
            json!({ "action": "warn", "rule": "harassment", "facts": "  " }),
            "facts",
        ),
        (
            json!({ "action": "warn", "rule": "harassment", "facts": "f".repeat(4001) }),
            "facts",
        ),
    ] {
        let res = app
            .send(with_token(decide(troll, body.clone()), &moderate))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{body}");
    }
    let mut extra = warn.clone();
    extra["severity"] = json!("high");
    assert_eq!(
        app.send(with_token(decide(troll, extra), &moderate))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Not on yourself; not on accounts that aren't there or are closed.
    let res = app
        .send(decide(olive_id, warn.clone()).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    for who in [gone, Uuid::now_v7()] {
        assert_eq!(
            app.send(with_token(decide(who, warn.clone()), &moderate))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    // Who may: not read tokens or reviewers; support warns but doesn't suspend.
    assert_eq!(
        app.send(with_token(decide(troll, warn.clone()), &read))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(decide(troll, warn.clone()).cookie(&rex))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut suspend_body = warn.clone();
    suspend_body["action"] = json!("suspend_account");
    assert_eq!(
        app.send(decide(troll, suspend_body.clone()).cookie(&sue))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(decisions().await, 0);

    // A warning: recorded with its facts (a blank law dropped), explained to the person.
    let mut blank_law = warn.clone();
    blank_law["law"] = json!("   ");
    let res = app.send(decide(troll, blank_law).cookie(&sue)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["decision"].is_string());
    let (law, facts, by): (Option<String>, String, String) = sqlx::query_as(
        "select law, facts, decided_by from moderation_decisions where user_id = $1",
    )
    .bind(troll)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((law, facts.as_str()), (None, "Insults in three reviews."));
    assert!(by.starts_with("Sue Support ("));
    let told: (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and category = 'moderation'",
    )
    .bind(troll)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told.0, "Warning: Troll Account");
    assert!(told.1.contains("Content Policy rule"), "{}", told.1);

    // A suspension with a law, once; a second one conflicts.
    suspend_body["law"] = json!("§ 185 StGB");
    let res = app
        .send(with_token(decide(troll, suspend_body.clone()), &moderate))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(is_suspended(&app, troll).await);
    let law: Option<String> =
        sqlx::query_scalar("select law from moderation_decisions where action = 'suspend_account'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(law.as_deref(), Some("§ 185 StGB"));
    let res = app
        .send(with_token(decide(troll, suspend_body), &moderate))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let failed: String =
        sqlx::query_scalar("select status from staff_actions order by requested_at desc limit 1")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(failed, "failed");
    assert_eq!(decisions().await, 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn lifting_a_suspension_needs_a_reason_and_a_suspended_account(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (troll, _) = app.user("Troll Account", "US").await;
    let tokens = [
        admin_token(&app, &owner, "read").await,
        admin_token(&app, &owner, "review").await,
    ];
    let moderate = admin_token(&app, &owner, "moderate").await;
    let lift = |reason: String| {
        req(
            Method::POST,
            format!("/v1/admin/users/{troll}/lift-suspension"),
        )
        .json(json!({ "reason": reason }))
    };
    let good = "Apologised to everyone involved.".to_string();

    // Not suspended: nothing to lift.
    let res = app.send(with_token(lift(good.clone()), &moderate)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    suspend(&app, &owner, troll).await;

    for reason in ["Too short".to_string(), "r".repeat(1001)] {
        let res = app.send(with_token(lift(reason), &moderate)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    for token in &tokens {
        assert_eq!(
            app.send(with_token(lift(good.clone()), token))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert!(is_suspended(&app, troll).await);

    // Lifted: the decision is marked reversed, the person told, the audit log says why.
    let res = app
        .send(with_token(lift(format!("  {good}  ")), &moderate))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!is_suspended(&app, troll).await);
    let reversed: bool = sqlx::query_scalar(
        "select reversed_at is not null from moderation_decisions where action = 'suspend_account'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(reversed);
    assert!(
        notice_titles(&app, troll)
            .await
            .contains(&"Your account is no longer suspended".to_string())
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'moderation.lift_suspension'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail["reason"], good.as_str());
    assert!(audit_actors(&app, "moderation.lift_suspension").await[0].contains("Agent moderate"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_passkey_grace_week_lets_someone_publish_until_it_ends(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (mia, maker) = app.user("Mia Maker", "US").await;
    let (gone, _) = app.user("Closed Cal", "US").await;
    slop_api::account::close_account(&app.db, gone, "test")
        .await
        .unwrap();
    let moderate = admin_token(&app, &owner, "moderate").await;
    let reason = "Video call; matched her GitHub account.";
    let grace = |who: Uuid, reason: String| {
        req(Method::POST, format!("/v1/admin/users/{who}/passkey-grace"))
            .json(json!({ "reason": reason }))
    };
    // Past her passkey deadline, with none: she can't publish.
    sqlx::query("update users set passkey_due_at = now() - interval '1 hour' where id = $1")
        .bind(mia)
        .execute(&app.db)
        .await
        .unwrap();
    let create = |slug: &str| {
        req(Method::POST, "/v1/apps")
            .cookie(&maker)
            .json(new_app(slug))
    };
    assert_eq!(
        app.send(create("one")).await.status(),
        StatusCode::FORBIDDEN
    );

    for scope in ["read", "review"] {
        let token = admin_token(&app, &owner, scope).await;
        assert_eq!(
            app.send(with_token(grace(mia, reason.into()), &token))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{scope}"
        );
    }
    for who in [gone, Uuid::now_v7()] {
        assert_eq!(
            app.send(with_token(grace(who, reason.into()), &moderate))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        app.send(with_token(grace(mia, "r".repeat(1001)), &moderate))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A moderate token gives it: a week, and she's told until when.
    let res = app
        .send(with_token(grace(mia, reason.into()), &moderate))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["until"].is_string());
    let until: OffsetDateTime =
        sqlx::query_scalar("select passkey_grace_until from users where id = $1")
            .bind(mia)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        (until - OffsetDateTime::now_utc() - time::Duration::days(7)).abs()
            < time::Duration::minutes(1)
    );
    let (title, body): (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and title like 'Add a new passkey%'",
    )
    .bind(mia)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(title, "Add a new passkey this week");
    assert!(body.contains(&until.date().to_string()), "{body}");
    assert!(audit_actors(&app, "account.passkey_grace").await[0].contains("Agent moderate"));
    assert_eq!(app.send(create("one")).await.status(), StatusCode::CREATED);

    // Once it ends, she needs a passkey again.
    sqlx::query("update users set passkey_grace_until = now() - interval '1 minute' where id = $1")
        .bind(mia)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(create("two")).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn account_actions_wait_for_a_second_person_and_check_again_when_approved(db: PgPool) {
    let app = agents_app(db, 2).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (troll, _) = app.user("Troll Account", "US").await;
    let (mia, _) = app.user("Mia Maker", "US").await;
    let agent = admin_token(&app, &ada, "moderate").await;
    let approve = |id: &str, cookie: &str| {
        req(Method::POST, format!("/v1/admin/actions/{id}/approve")).cookie(cookie)
    };
    let pending = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let body = body_json(res).await;
        assert_eq!(body["approvals_still_needed"], 1);
        body["pending_action"].as_str().unwrap().to_string()
    };
    let action_status = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select status from staff_actions where id = $1::uuid")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // Ada's agent asks to suspend; nothing happens until Ben approves, then it's in both names.
    let suspend_troll = || {
        with_token(
            req(Method::POST, format!("/v1/admin/users/{troll}/decide")).json(json!({
                "action": "suspend_account", "rule": "harassment", "facts": "Kept at it after a warning."
            })),
            &agent,
        )
    };
    let first = pending(app.send(suspend_troll()).await).await;
    assert!(!is_suspended(&app, troll).await);
    assert_eq!(
        app.send(approve(&first, &ben)).await.status(),
        StatusCode::OK
    );
    assert!(is_suspended(&app, troll).await);
    let by: String = sqlx::query_scalar("select decided_by from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(
        by.contains("Agent moderate") && by.contains("approved by Ben Owner"),
        "{by}"
    );

    // Lifting it waits the same way, and then reverses the decision.
    let lift = || {
        with_token(
            req(
                Method::POST,
                format!("/v1/admin/users/{troll}/lift-suspension"),
            )
            .json(json!({ "reason": "The reports were a misunderstanding." })),
            &agent,
        )
    };
    let lifted = pending(app.send(lift()).await).await;
    assert!(is_suspended(&app, troll).await);
    assert_eq!(
        app.send(approve(&lifted, &ben)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(!is_suspended(&app, troll).await);
    assert_eq!(
        count(
            &app,
            "select count(*) from moderation_decisions where reversed_at is not null"
        )
        .await,
        1
    );
    assert!(
        audit_actors(&app, "moderation.lift_suspension").await[0].contains("approved by Ben Owner")
    );

    // A passkey grace too.
    let grace = pending(
        app.send(with_token(
            req(Method::POST, format!("/v1/admin/users/{mia}/passkey-grace"))
                .json(json!({ "reason": "Video call; matched her GitHub account." })),
            &agent,
        ))
        .await,
    )
    .await;
    assert_eq!(
        count(
            &app,
            &format!(
                "select count(*) from users where id = '{mia}' and passkey_grace_until is not null"
            )
        )
        .await,
        0
    );
    assert_eq!(
        app.send(approve(&grace, &ben)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        count(
            &app,
            &format!(
                "select count(*) from users where id = '{mia}' and passkey_grace_until > now()"
            )
        )
        .await,
        1
    );

    // Things change while a request waits: it's checked again when approved.
    let again = pending(app.send(suspend_troll()).await).await;
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(troll)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(approve(&again, &ben)).await.status(),
        StatusCode::CONFLICT,
        "already suspended"
    );
    assert_eq!(action_status(again).await, "failed");
    let lift_req = pending(app.send(lift()).await).await;
    sqlx::query("update users set suspended_at = null where id = $1")
        .bind(troll)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(approve(&lift_req, &ben)).await.status(),
        StatusCode::CONFLICT,
        "not suspended any more"
    );
    assert_eq!(action_status(lift_req).await, "failed");
    let late = pending(app.send(suspend_troll()).await).await;
    slop_api::account::close_account(&app.db, troll, "test")
        .await
        .unwrap();
    assert_eq!(
        app.send(approve(&late, &ben)).await.status(),
        StatusCode::CONFLICT,
        "closed"
    );

    // Requests nobody approved within three days lapse and never run.
    let (other, _) = app.user("Other Troll", "US").await;
    let stale = pending(
        app.send(with_token(
            req(Method::POST, format!("/v1/admin/users/{other}/decide")).json(json!({
                "action": "suspend_account", "rule": "harassment", "facts": "Spam in every review."
            })),
            &agent,
        ))
        .await,
    )
    .await;
    sqlx::query(
        "update staff_actions set requested_at = now() - interval '4 days' where id = $1::uuid",
    )
    .bind(&stale)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app.send(approve(&stale, &ben)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
    assert!(!is_suspended(&app, other).await);
}

// ---------------------------------------------------------------------------------------------
// The admin MCP endpoint

/// Every tool the admin MCP endpoint offers (agent.rs TOOLS).
const ADMIN_TOOLS: [&str; 54] = [
    "slop_status",
    "review_guide",
    "review_queue",
    "admin_queue",
    "add_note",
    "approve_release",
    "reject_release",
    "withdraw_release",
    "clear_scan_finding",
    "keep_rescanned_release",
    "decide_change",
    "decide_pictures",
    "decide_portfolio",
    "list_app",
    "delist_app",
    "act_on_report",
    "dismiss_report",
    "decide_appeal",
    "close_abuse_signal",
    "find_users",
    "user_details",
    "moderate_user",
    "lift_suspension",
    "passkey_grace",
    "pending_actions",
    "audit_log",
    "find_apps",
    "app_record",
    "edit_store_page",
    "set_app_stage",
    "freeze_app",
    "unfreeze_app",
    "donation_pauses",
    "pause_donations",
    "lift_donation_pause",
    "answer_donation_request",
    "user_profile",
    "edit_profile",
    "edit_team",
    "featured_apps",
    "set_featured_apps",
    "store_shelves",
    "create_shelf",
    "update_shelf",
    "delete_shelf",
    "order_shelves",
    "hide_front_shelf",
    "legal_documents",
    "publish_legal_document",
    "withdraw_legal_document",
    "staff",
    "change_staff_role",
    "remove_from_staff",
    "cancel_pending_action",
];

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_admin_mcp_endpoint_speaks_json_rpc_to_staff_only(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let token = admin_token(&app, &owner, "review").await;
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

    // Nobody, people who aren't staff, and makers' agents: there's nothing here.
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Claude", "scope": "publish" })),
        )
        .await;
    let maker_token = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    for r in [
        req(Method::POST, "/v1/admin/mcp").json(ping.clone()),
        req(Method::POST, "/v1/admin/mcp")
            .cookie(&maker)
            .json(ping.clone()),
        with_token(
            req(Method::POST, "/v1/admin/mcp").json(ping.clone()),
            &maker_token,
        ),
        with_token(
            req(Method::POST, "/v1/admin/mcp").json(ping.clone()),
            "Bearer slopadm_not-a-token",
        ),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    // There's no stream to open: staff get 405, everyone else still nothing.
    assert_eq!(
        app.send(with_token(req(Method::GET, "/v1/admin/mcp"), &token))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/mcp").cookie(&owner))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    for r in [
        req(Method::GET, "/v1/admin/mcp"),
        req(Method::GET, "/v1/admin/mcp").cookie(&maker),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }

    // The handshake, a ping, the tools; ids come back as they were sent.
    let init = mcp(
        &app,
        &token,
        json!({ "jsonrpc": "2.0", "id": "start", "method": "initialize", "params": {} }),
    )
    .await;
    assert_eq!(init["id"], "start");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("slop_status")
    );
    let pong = mcp(
        &app,
        &token,
        json!({ "jsonrpc": "2.0", "id": 42, "method": "ping" }),
    )
    .await;
    assert_eq!(pong, json!({ "jsonrpc": "2.0", "id": 42, "result": {} }));
    let list = mcp(
        &app,
        &token,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ADMIN_TOOLS);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["description"].as_str().unwrap().len() > 10);
    }
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/admin/mcp")
                .json(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
            &token,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    assert!(
        res.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );

    // Protocol errors.
    let unknown = mcp(
        &app,
        &token,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32601);
    assert_eq!(unknown["id"], 3);
    let no_tool = mcp(
        &app,
        &token,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "drop_tables" } }),
    )
    .await;
    assert_eq!(no_tool["error"]["code"], -32602);
    let res = app
        .send(
            req(Method::POST, "/v1/admin/mcp")
                .header("authorization", token.clone())
                .header("content-type", "application/json")
                .json(json!("not an object")),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["code"], -32700);
    // Bodies over the router's 64 KiB limit never reach it.
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/admin/mcp").json(json!({
                "jsonrpc": "2.0", "id": 5, "method": "ping", "params": { "pad": "p".repeat(70 * 1024) }
            })),
            &token,
        ))
        .await;
    // (413 when the length is announced; read to the limit and refused otherwise.)
    if res.status() == StatusCode::OK {
        let reply = body_json(res).await;
        assert_eq!(reply["error"]["code"], -32700, "{reply}");
        assert_eq!(reply["error"]["message"], "request too large");
    } else {
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
    let big = json!({
        "jsonrpc": "2.0", "id": 5, "method": "ping", "params": { "pad": "p".repeat(70 * 1024) }
    });
    let res = app
        .send(
            with_token(req(Method::POST, "/v1/admin/mcp"), &token)
                .header("content-length", big.to_string().len().to_string())
                .json(big),
        )
        .await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Tool arguments: required ones, path values that would leave their segment, decisions.
    let id = Uuid::now_v7().to_string();
    assert_eq!(
        tool_refused(&app, &token, "approve_release", json!({})).await,
        "missing argument: id"
    );
    for bad in ["", ".", ".."] {
        let text = tool_refused(&app, &token, "approve_release", json!({ "id": bad })).await;
        assert_eq!(text, "id isn't a valid value", "{bad:?}");
    }
    let text = tool_refused(
        &app,
        &token,
        "decide_change",
        json!({ "id": id, "decision": "../../tokens" }),
    )
    .await;
    assert_eq!(text, "decision must be approve or reject");
    // A "/" is escaped: the value reaches the route as one (invalid) ID, not another route.
    let text = tool_refused(&app, &token, "user_details", json!({ "id": "x/y" })).await;
    assert!(!text.contains("not_found"), "{text}");
    // The route's own refusal comes back as it was, marked as an error.
    let text = tool_refused(&app, &token, "find_users", json!({ "q": " " })).await;
    let err: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(err["error"]["field"], "q");
    // An empty success says so.
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (ok, text) = tool(
        &app,
        &token,
        "set_app_stage",
        json!({ "slug": "arena", "stage": "coming_soon" }),
    )
    .await;
    assert!(ok);
    assert_eq!(text, "Done (HTTP 204).");

    // A token revoked between two calls stops at once.
    let token_id: Uuid = sqlx::query_scalar("select id from admin_tokens")
        .fetch_one(&app.db)
        .await
        .unwrap();
    app.send(req(Method::POST, format!("/v1/admin/tokens/{token_id}/revoke")).cookie(&owner))
        .await;
    assert_eq!(
        app.send(with_token(
            req(Method::POST, "/v1/admin/mcp").json(ping),
            &token
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_signed_in_on_the_website_can_drive_the_admin_mcp_tools(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, rex) = staff(&app, "Rex Reviewer", &["reviewer"]).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let call = |cookie: &str, name: &str, args: Value| {
        req(Method::POST, "/v1/admin/mcp")
            .cookie(cookie)
            .json(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": args }
            }))
    };
    let result = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::OK);
        let reply = body_json(res).await;
        (
            reply["result"]["isError"] == false,
            reply["result"]["content"][0]["text"].clone(),
        )
    };

    // The session cookie is passed on: reading, and acting in the person's name.
    let (ok, text) = result(app.send(call(&owner, "slop_status", json!({}))).await).await;
    assert!(
        ok && text.as_str().unwrap().contains("\"waiting\""),
        "{text}"
    );
    let (ok, text) = result(
        app.send(
            call(
                &owner,
                "set_app_stage",
                json!({ "slug": "arena", "stage": "coming_soon" }),
            )
            .header("sec-fetch-site", "same-origin"),
        )
        .await,
    )
    .await;
    assert!(ok, "{text}");
    assert!(audit_actors(&app, "app.stage").await[0].starts_with("Olive Owner ("));
    // With the person's own roles: a reviewer can't read an account.
    let (ok, _) = result(
        app.send(call(
            &rex,
            "user_details",
            json!({ "id": maker_id.to_string() }),
        ))
        .await,
    )
    .await;
    assert!(!ok);

    // From another site, the cookie doesn't get anywhere.
    for (name, value) in [
        ("sec-fetch-site", "cross-site"),
        ("origin", "https://evil.example"),
    ] {
        let res = app
            .send(
                call(
                    &owner,
                    "set_app_stage",
                    json!({ "slug": "arena", "stage": "released" }),
                )
                .header(name, value),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{name}");
    }
    let stage: String = sqlx::query_scalar("select stage::text from apps where slug = 'arena'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "coming_soon");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "b60: the router's 64 KiB body limit refuses legal documents (and MCP calls) that legal.rs and serve_mcp accept"]
async fn a_long_legal_document_can_be_published_over_mcp(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let token = admin_token(&app, &owner, "moderate").await;
    // legal.rs takes up to 200,000 characters; real terms run long.
    let body = "Every clause, spelled out in full.\n\n".repeat(2_000);
    assert!(body.len() > 64 * 1024 && body.chars().count() < 200_000);
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/admin/mcp").json(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "publish_legal_document",
                            "arguments": { "kind": "terms", "summary": "Spells everything out.", "body_md": body } }
            })),
            &token,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let reply = body_json(res).await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
}

// ---------------------------------------------------------------------------------------------
// Admin MCP tools

/// A copy of a release, as another version in this state.
async fn copy_release(app: &TestApp, id: &str, version: &str, status: &str) -> Uuid {
    let new = Uuid::now_v7();
    let mut tx = app.db.begin().await.unwrap();
    sqlx::query("create temp table release_copy on commit drop as select * from releases where id = $1::uuid")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update release_copy set id = $1, version = $2, status = '{status}', signed_manifest = null"
    )))
    .bind(new)
    .bind(version)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("insert into releases select * from release_copy")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    new
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_review_releases_changes_pictures_and_portfolios_through_mcp_tools(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let w = seed_waiting(&app).await;
    let maker: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let release_status = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select status::text from releases where id = $1::uuid")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // Looking: the status, the guide, the feed (all, or one kind), the admin queue.
    let status = tool_ok(&app, &read, "slop_status", json!({})).await;
    assert_eq!(status["waiting"]["releases_to_review"], 1);
    let (ok, guide) = tool(&app, &read, "review_guide", json!({})).await;
    assert!(ok && guide.contains("http://localhost:8080") && !guide.contains("{origin}"));
    let all = tool_ok(&app, &read, "review_queue", json!({})).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 9);
    assert_eq!(all["can_decide"], false);
    let changes = tool_ok(&app, &read, "review_queue", json!({ "kind": "change" })).await;
    let items = changes["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i["kind"] == "change"));
    sqlx::query(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, 'vote_burst', 'vote_burst:game', 'app', $2, 'game', '{}')",
    )
    .bind(Uuid::now_v7())
    .bind(app_id(&app, "game").await)
    .execute(&app.db)
    .await
    .unwrap();
    let queue = tool_ok(&app, &read, "admin_queue", json!({})).await;
    assert!(queue["rules"].as_array().unwrap().len() > 3);
    assert_eq!(queue["abuse"][0]["kind"], "vote_burst");
    assert_eq!(queue["releases"][0]["id"], w.release.as_str());

    // Notes: kind and id go in the path, the rest in the body.
    let note = tool_ok(
        &app,
        &read,
        "add_note",
        json!({ "kind": "media", "id": w.media.to_string(), "verdict": "pass", "note": "Pictures match the game." }),
    )
    .await;
    assert_eq!(note["verdict"], "pass");
    let subject: Uuid = sqlx::query_scalar("select subject_id from review_notes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(subject, w.media);
    let text = tool_refused(
        &app,
        &read,
        "add_note",
        json!({ "kind": "media", "id": w.media.to_string(), "verdict": "great", "note": "x" }),
    )
    .await;
    assert!(text.contains("\"field\":\"verdict\""), "{text}");
    assert_eq!(
        tool_refused(
            &app,
            &read,
            "add_note",
            json!({ "kind": "media", "id": w.media.to_string() })
        )
        .await,
        "missing argument: note"
    );

    // Listing needs an approved release first; a read token can't decide anything.
    let text = tool_refused(&app, &review, "list_app", json!({ "slug": "game" })).await;
    assert!(text.contains("approve a release"), "{text}");
    assert!(
        tool_refused(&app, &read, "list_app", json!({ "slug": "soon" }))
            .await
            .contains("forbidden")
    );

    // A scan finding cleared as a false positive, before approval.
    let clear = |platform: &str, reason: &str| json!({ "id": w.release.clone(), "platform": platform, "reason": reason });
    let why = "The packer is UPX; the same binary is clean on VirusTotal.";
    assert!(
        tool_refused(&app, &read, "clear_scan_finding", clear("windows", why))
            .await
            .contains("forbidden")
    );
    tool_refused(
        &app,
        &review,
        "clear_scan_finding",
        clear("windows", "fine"),
    )
    .await;
    let text = tool_refused(&app, &review, "clear_scan_finding", clear("macos", why)).await;
    assert!(text.contains("no such file"), "{text}");
    assert_eq!(
        tool_ok(&app, &review, "clear_scan_finding", clear("windows", why)).await,
        Value::Null
    );
    let cleared: Option<String> =
        sqlx::query_scalar("select cleared_by from release_files where release_id = $1::uuid")
            .bind(&w.release)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(cleared.unwrap().starts_with("AI reviewer \"Agent review\""));
    assert_eq!(audit_actors(&app, "release.clear_scan").await.len(), 1);

    // Rejecting a release: it needs a reason, the maker is told, and it happens once.
    let other = copy_release(&app, &w.release, "1.0.1", "awaiting_review")
        .await
        .to_string();
    assert!(
        tool_refused(
            &app,
            &read,
            "reject_release",
            json!({ "id": other, "reason": "Crashes." })
        )
        .await
        .contains("forbidden")
    );
    assert_eq!(
        tool_refused(&app, &review, "reject_release", json!({ "id": other })).await,
        "missing argument: reason"
    );
    tool_refused(
        &app,
        &review,
        "reject_release",
        json!({ "id": other, "reason": "  " }),
    )
    .await;
    let (ok, text) = tool(
        &app,
        &review,
        "reject_release",
        json!({ "id": other, "reason": "It crashes on start: the exec path is wrong." }),
    )
    .await;
    assert!(ok, "{text}");
    assert_eq!(text, "Done (HTTP 204).");
    assert_eq!(release_status(other.clone()).await, "rejected");
    let told: Vec<String> = sqlx::query_scalar("select body from notifications where user_id = $1")
        .bind(maker)
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert!(
        told.iter().any(|b| b.contains("the exec path is wrong")),
        "{told:?}"
    );
    let text = tool_refused(
        &app,
        &review,
        "reject_release",
        json!({ "id": other, "reason": "Again." }),
    )
    .await;
    assert!(text.contains("conflict"), "{text}");

    // Approving: a read token can't; a review token approves and signs, once.
    assert!(
        tool_refused(&app, &read, "approve_release", json!({ "id": w.release }))
            .await
            .contains("forbidden")
    );
    assert_eq!(release_status(w.release.clone()).await, "awaiting_review");
    let (ok, text) = tool(&app, &review, "approve_release", json!({ "id": w.release })).await;
    assert!(ok, "{text}");
    assert_eq!(release_status(w.release.clone()).await, "approved");
    let signed: bool =
        sqlx::query_scalar("select signed_manifest is not null from releases where id = $1::uuid")
            .bind(&w.release)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(signed);
    assert!(
        audit_actors(&app, "release.approve").await[0].starts_with("AI reviewer \"Agent review\"")
    );
    let text = tool_refused(&app, &review, "approve_release", json!({ "id": w.release })).await;
    assert!(text.contains("conflict"), "{text}");
    assert_eq!(
        tool_ok(&app, &review, "list_app", json!({ "slug": "game" })).await,
        Value::Null
    );
    let listed: String = sqlx::query_scalar("select status::text from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(listed, "listed");

    // A re-scan flags the live release; it's kept as a false positive.
    sqlx::query("update release_files set rescan_flagged_at = now() where release_id = $1::uuid")
        .bind(&w.release)
        .execute(&app.db)
        .await
        .unwrap();
    let keep =
        |reason: &str| json!({ "id": w.release.clone(), "platform": "windows", "reason": reason });
    assert!(
        tool_refused(&app, &read, "keep_rescanned_release", keep(why))
            .await
            .contains("forbidden")
    );
    let text = tool_refused(&app, &review, "keep_rescanned_release", keep("short")).await;
    assert!(text.contains("\"field\":\"reason\""), "{text}");
    tool_ok(&app, &review, "keep_rescanned_release", keep(why)).await;
    let (flagged, kept): (Option<OffsetDateTime>, Option<String>) = sqlx::query_as(
        "select rescan_flagged_at, rescan_cleared_reason from release_files where release_id = $1::uuid",
    )
    .bind(&w.release)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(flagged.is_none());
    assert_eq!(kept.as_deref(), Some(why));
    assert!(audit_actors(&app, "release.rescan_cleared").await[0].contains("Agent review"));
    tool_refused(&app, &review, "keep_rescanned_release", keep(why)).await;

    // Changes: turned down with a reason, or approved.
    let change_status = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>)>(
                "select status, status_reason from app_changes where id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let repo = w.repo_change.to_string();
    assert!(
        tool_refused(
            &app,
            &read,
            "decide_change",
            json!({ "id": repo, "decision": "reject", "reason": "No." })
        )
        .await
        .contains("forbidden")
    );
    tool_refused(
        &app,
        &review,
        "decide_change",
        json!({ "id": repo, "decision": "reject" }),
    )
    .await;
    tool_ok(
        &app,
        &review,
        "decide_change",
        json!({ "id": repo, "decision": "reject", "reason": "That repository isn't yours." }),
    )
    .await;
    assert_eq!(
        change_status(w.repo_change).await,
        (
            "rejected".into(),
            Some("That repository isn't yours.".into())
        )
    );
    tool_ok(
        &app,
        &review,
        "decide_change",
        json!({ "id": w.follow_change.to_string(), "decision": "approve" }),
    )
    .await;
    assert_eq!(change_status(w.follow_change).await.0, "approved");

    // Pictures: approved, or turned down with a reason.
    let media = w.media.to_string();
    tool_refused(
        &app,
        &review,
        "decide_pictures",
        json!({ "id": media, "decision": "reject" }),
    )
    .await;
    tool_ok(
        &app,
        &review,
        "decide_pictures",
        json!({ "id": media, "decision": "approve" }),
    )
    .await;
    let shown: Option<Uuid> =
        sqlx::query_scalar("select media_set_id from apps where slug = 'pictured'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(shown, Some(w.media));
    let second = Uuid::now_v7();
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, fingerprint, source) values ($1, $2, $3, 'fp2', 'maker')",
    )
    .bind(second)
    .bind(app_id(&app, "pictured").await)
    .bind("f".repeat(40))
    .execute(&app.db)
    .await
    .unwrap();
    tool_ok(
        &app,
        &review,
        "decide_pictures",
        json!({ "id": second.to_string(), "decision": "reject", "reason": "The thumbnail is another game's." }),
    )
    .await;
    let (status, reason): (String, Option<String>) =
        sqlx::query_as("select status, status_reason from media_sets where id = $1")
            .bind(second)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("rejected", Some("The thumbnail is another game's."))
    );

    // A portfolio: not with a read token; turning it down needs a reason.
    let portfolio = w.portfolio.to_string();
    assert!(
        tool_refused(
            &app,
            &read,
            "decide_portfolio",
            json!({ "id": portfolio, "decision": "approve" })
        )
        .await
        .contains("forbidden")
    );
    tool_refused(
        &app,
        &review,
        "decide_portfolio",
        json!({ "id": portfolio, "decision": "reject" }),
    )
    .await;
    tool_ok(
        &app,
        &review,
        "decide_portfolio",
        json!({ "id": portfolio, "decision": "reject", "reason": "These works are someone else's." }),
    )
    .await;
    let status: String = sqlx::query_scalar("select status from portfolio_sets where id = $1")
        .bind(w.portfolio)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "rejected");

    // Stages: only the three there are, and not with a read token.
    let text = tool_refused(
        &app,
        &review,
        "set_app_stage",
        json!({ "slug": "other", "stage": "gold" }),
    )
    .await;
    assert!(!text.is_empty());
    assert!(
        tool_refused(
            &app,
            &read,
            "set_app_stage",
            json!({ "slug": "other", "stage": "coming_soon" })
        )
        .await
        .contains("forbidden")
    );
    tool_ok(
        &app,
        &review,
        "set_app_stage",
        json!({ "slug": "other", "stage": "coming_soon" }),
    )
    .await;
    let stage: String = sqlx::query_scalar("select stage::text from apps where slug = 'other'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "coming_soon");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_moderate_reports_appeals_and_accounts_through_mcp_tools(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (olive, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let moderate = admin_token(&app, &owner, "moderate").await;
    let (mia, mia_cookie) = app.user("Mia Maker", "US").await;
    listed_with(&app, mia, "miner", "released", "").await;
    listed_with(&app, mia, "decoy", "released", "").await;
    let (rita, rita_cookie) = app.user("Rita Reporter", "US").await;
    let (_, otto) = app.user("Otto Other", "US").await;
    let (troll, troll_cookie) = app.user("Troll Account", "US").await;

    // Finding and reading accounts.
    tool_refused(&app, &read, "find_users", json!({ "q": "" })).await;
    let found = tool_ok(&app, &read, "find_users", json!({ "q": "troll" })).await;
    assert_eq!(found[0]["id"], troll.to_string());
    assert!(
        tool_refused(
            &app,
            &read,
            "user_details",
            json!({ "id": troll.to_string() })
        )
        .await
        .contains("forbidden")
    );
    let text = tool_refused(
        &app,
        &review,
        "user_details",
        json!({ "id": Uuid::now_v7().to_string() }),
    )
    .await;
    assert!(text.contains("not_found"), "{text}");
    let details = tool_ok(
        &app,
        &review,
        "user_details",
        json!({ "id": mia.to_string() }),
    )
    .await;
    assert_eq!(details["apps"].as_array().unwrap().len(), 2);

    // Warning without a report; bad rules, empty facts and your own account are refused.
    let warn = |who: Uuid, rule: &str, facts: &str| json!({ "id": who.to_string(), "action": "warn", "rule": rule, "facts": facts });
    let text = tool_refused(
        &app,
        &review,
        "moderate_user",
        warn(troll, "rudeness", "Insults."),
    )
    .await;
    assert!(text.contains("\"field\":\"rule\""), "{text}");
    let text = tool_refused(
        &app,
        &review,
        "moderate_user",
        warn(troll, "harassment", " "),
    )
    .await;
    assert!(text.contains("\"field\":\"facts\""), "{text}");
    let text = tool_refused(
        &app,
        &review,
        "moderate_user",
        warn(olive, "harassment", "Me."),
    )
    .await;
    assert!(text.contains("own account"), "{text}");
    let warned = tool_ok(
        &app,
        &review,
        "moderate_user",
        warn(troll, "harassment", "Insults in reviews."),
    )
    .await;
    let decision = warned["decision"].as_str().unwrap().to_string();

    // Lifting a suspension that isn't there, or without enough reason, or with a review token.
    let lift = |reason: &str| json!({ "id": troll.to_string(), "reason": reason });
    assert!(
        tool_refused(&app, &review, "lift_suspension", lift("They apologised."))
            .await
            .contains("forbidden")
    );
    assert!(
        tool_refused(&app, &moderate, "lift_suspension", lift("They apologised."))
            .await
            .contains("isn't suspended")
    );
    assert!(
        tool_refused(&app, &moderate, "lift_suspension", lift("Sorry"))
            .await
            .contains("\"field\":\"reason\"")
    );

    // Passkey grace: a moderate token, not a review one.
    let grace =
        json!({ "id": mia.to_string(), "reason": "Video call; matched her GitHub account." });
    assert!(
        tool_refused(&app, &review, "passkey_grace", grace.clone())
            .await
            .contains("forbidden")
    );
    let given = tool_ok(&app, &moderate, "passkey_grace", grace).await;
    assert!(given["until"].is_string());

    // Appeals: kept, or reversed; the boolean goes through as one.
    let appeal = |cookie: &str, id: &str| {
        req(Method::POST, format!("/v1/me/decisions/{id}/appeal"))
            .cookie(cookie)
            .json(json!({ "appeal": "That wasn't me." }))
    };
    assert_eq!(
        app.send(appeal(&troll_cookie, &decision)).await.status(),
        StatusCode::OK
    );
    let keep = json!({ "id": decision, "reverse": false, "reason": "The messages are from your account." });
    assert!(
        tool_refused(&app, &read, "decide_appeal", keep.clone())
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &review, "decide_appeal", keep.clone()).await;
    let (status, reversed): (String, Option<OffsetDateTime>) = sqlx::query_as(
        "select appeal_status, reversed_at from moderation_decisions where id = $1::uuid",
    )
    .bind(&decision)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((status.as_str(), reversed), ("upheld", None));
    assert!(
        tool_refused(&app, &review, "decide_appeal", keep)
            .await
            .contains("isn't pending")
    );

    // Reports: acting needs the outcome; suspending needs a moderate token; the reporter hears.
    let first = report(&app, &rita_cookie, "miner", "malware")
        .await
        .to_string();
    let act = |action: &str| {
        json!({ "id": first, "action": action, "rule": "malware",
                "facts": "Version 1.2 starts a crypto miner.", "outcome": "We took it out of the store.",
                "external_reference": "NCA-2026-77" })
    };
    assert!(
        tool_refused(&app, &read, "act_on_report", act("delist_app"))
            .await
            .contains("forbidden")
    );
    let mut no_outcome = act("delist_app");
    no_outcome.as_object_mut().unwrap().remove("outcome");
    assert_eq!(
        tool_refused(&app, &review, "act_on_report", no_outcome).await,
        "missing argument: outcome"
    );
    assert!(
        tool_refused(&app, &review, "act_on_report", act("suspend_account"))
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &review, "act_on_report", act("delist_app")).await;
    let (status, outcome, external): (String, String, Option<String>) = sqlx::query_as(
        "select status, outcome, external_reference from reports where id = $1::uuid",
    )
    .bind(&first)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), outcome.as_str(), external.as_deref()),
        (
            "actioned",
            "We took it out of the store.",
            Some("NCA-2026-77")
        )
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/reports").cookie(&rita_cookie))
            .await,
    )
    .await;
    assert_eq!(mine[0]["outcome"], "We took it out of the store.");
    let delisted: String = sqlx::query_scalar("select status::text from apps where slug = 'miner'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(delisted, "delisted");
    // A report about a person, acted on with a suspension by a moderate token.
    let about_troll = report(&app, &otto, "decoy", "scam").await.to_string();
    tool_ok(
        &app,
        &moderate,
        "act_on_report",
        json!({ "id": about_troll, "action": "suspend_account", "rule": "scam",
                "facts": "The app takes payments and never delivers.", "outcome": "We suspended the account." }),
    )
    .await;
    assert!(is_suspended(&app, mia).await);

    // Dismissing: not with a read token; the reporter is told; only once.
    listed_with(&app, troll, "harmless", "released", "").await;
    let third = report(&app, &rita_cookie, "harmless", "broken")
        .await
        .to_string();
    let dismiss = json!({ "id": third, "outcome": "It works for us: try reinstalling." });
    assert!(
        tool_refused(&app, &read, "dismiss_report", dismiss.clone())
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &review, "dismiss_report", dismiss.clone()).await;
    let status: String = sqlx::query_scalar("select status from reports where id = $1::uuid")
        .bind(&third)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "dismissed");
    let told = notice_titles(&app, rita).await;
    assert!(told.len() >= 2, "{told:?}");
    assert!(
        tool_refused(&app, &review, "dismiss_report", dismiss)
            .await
            .contains("isn't open")
    );

    // The maker appeals the delisting, and it's reversed.
    let delist_decision: String = sqlx::query_scalar(
        "select id::text from moderation_decisions where action = 'delist_app' and user_id = $1",
    )
    .bind(mia)
    .fetch_one(&app.db)
    .await
    .unwrap();
    // (Suspended people can still appeal.)
    assert_eq!(
        app.send(appeal(&mia_cookie, &delist_decision))
            .await
            .status(),
        StatusCode::OK
    );
    tool_ok(
        &app,
        &review,
        "decide_appeal",
        json!({ "id": delist_decision, "reverse": true, "reason": "It was a worker thread, not a miner." }),
    )
    .await;
    let relisted: String = sqlx::query_scalar("select status::text from apps where slug = 'miner'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(relisted, "listed");

    // An abuse signal closed as acted on, with the note kept.
    let signal = Uuid::now_v7();
    sqlx::query(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, 'vote_burst', 'vote_burst:decoy', 'app', $2, 'decoy', '{}')",
    )
    .bind(signal)
    .bind(app_id(&app, "decoy").await)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(
        tool_refused(
            &app,
            &read,
            "close_abuse_signal",
            json!({ "id": signal.to_string(), "note": "x" })
        )
        .await
        .contains("forbidden")
    );
    tool_ok(
        &app,
        &review,
        "close_abuse_signal",
        json!({ "id": signal.to_string(), "note": "Votes from one address range; removed them.", "acted": true }),
    )
    .await;
    let (status, note, by): (String, String, String) =
        sqlx::query_as("select status, decision_note, decided_by from abuse_signals where id = $1")
            .bind(signal)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), note.as_str()),
        ("actioned", "Votes from one address range; removed them.")
    );
    assert!(by.contains("Agent review"));

    // Taking an app out of the store directly: a moderate token, a real rule; the maker hears why.
    let delist = |rule: &str| json!({ "slug": "harmless", "rule": rule, "reason": "Impersonates another studio." });
    assert!(
        tool_refused(&app, &review, "delist_app", delist("misleading"))
            .await
            .contains("forbidden")
    );
    assert!(
        tool_refused(&app, &moderate, "delist_app", delist("bad_vibes"))
            .await
            .contains("\"field\":\"rule\"")
    );
    tool_ok(&app, &moderate, "delist_app", delist("misleading")).await;
    let reasons: Vec<String> = sqlx::query_scalar(
        "select facts from moderation_decisions where user_id = $1 and action = 'delist_app'",
    )
    .bind(troll)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(reasons, ["Impersonates another studio."]);
    assert!(
        notice_titles(&app, troll)
            .await
            .contains(&"Taken out of the store: App: harmless".to_string())
    );
    let text = tool_refused(&app, &moderate, "delist_app", delist("misleading")).await;
    assert!(text.contains("isn't in the store"), "{text}");

    // The audit log, searched and paged; the sensitive actions, all done (one approval here).
    let log = tool_ok(&app, &read, "audit_log", json!({ "q": "moderation." })).await;
    let entries = log.as_array().unwrap();
    assert!(entries.len() >= 4);
    assert!(
        entries
            .iter()
            .all(|e| e["action"].as_str().unwrap().contains("moderation.")
                || e["actor"].as_str().unwrap().contains("moderation.")
                || e["subject"].as_str().unwrap().contains("moderation."))
    );
    let newest = entries[0]["id"].as_i64().unwrap();
    let older = tool_ok(
        &app,
        &read,
        "audit_log",
        json!({ "q": "moderation.", "before": newest }),
    )
    .await;
    assert_eq!(older.as_array().unwrap().len(), entries.len() - 1);
    assert!(
        older
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["id"].as_i64().unwrap() < newest)
    );
    let waiting = tool_ok(&app, &read, "pending_actions", json!({})).await;
    assert_eq!(waiting["actions"], json!([]));
    let history = tool_ok(&app, &read, "pending_actions", json!({ "status": "all" })).await;
    let kinds: Vec<&str> = history["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"app.delist") && kinds.contains(&"account.passkey_grace"),
        "{kinds:?}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_arrange_the_front_page_through_mcp_tools(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    for slug in ["cozy-farm", "tea-shop", "dark-dungeon"] {
        listed_with(&app, maker, slug, "released", "").await;
    }
    let shelf_slugs = |v: &Value| -> Vec<String> {
        v["shelves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["slug"].as_str().unwrap().to_string())
            .collect()
    };

    // The Featured carousel: up to 8, each line up to 160 characters, set by review tokens.
    assert_eq!(
        tool_ok(&app, &read, "featured_apps", json!({})).await,
        json!([])
    );
    let nine: Vec<Value> = (0..9).map(|_| json!({ "slug": "tea-shop" })).collect();
    let text = tool_refused(&app, &review, "set_featured_apps", json!({ "apps": nine })).await;
    assert!(text.contains("at most 8"), "{text}");
    let text = tool_refused(
        &app,
        &review,
        "set_featured_apps",
        json!({ "apps": [{ "slug": "tea-shop", "blurb": "b".repeat(161) }] }),
    )
    .await;
    assert!(text.contains("\"field\":\"blurb\""), "{text}");
    let slides = json!({ "apps": [
        { "slug": "tea-shop", "blurb": "Brew tea for tired travellers." },
        { "slug": "cozy-farm" }
    ] });
    assert!(
        tool_refused(&app, &read, "set_featured_apps", slides.clone())
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &review, "set_featured_apps", slides).await;
    let featured = tool_ok(&app, &read, "featured_apps", json!({})).await;
    assert_eq!(featured[0]["slug"], "tea-shop");
    assert_eq!(featured[0]["blurb"], "Brew tea for tired travellers.");
    assert_eq!(featured[1]["slug"], "cozy-farm");

    // Shelves: a bad address or too many apps is refused; arrays and booleans pass through.
    let listed = tool_ok(&app, &read, "store_shelves", json!({})).await;
    assert_eq!(listed["shelves"], json!([]));
    assert!(
        listed["built_in"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["hidden"] == false)
    );
    tool_refused(
        &app,
        &review,
        "create_shelf",
        json!({ "slug": "Cozy Games", "title": "Cozy", "apps": [] }),
    )
    .await;
    let many: Vec<String> = (0..25).map(|i| format!("app-{i}")).collect();
    let text = tool_refused(
        &app,
        &review,
        "create_shelf",
        json!({ "slug": "big", "title": "Big", "apps": many }),
    )
    .await;
    assert!(text.contains("at most 24"), "{text}");
    assert!(
        tool_refused(
            &app,
            &read,
            "create_shelf",
            json!({ "slug": "cozy", "title": "Cozy games", "apps": ["tea-shop"] })
        )
        .await
        .contains("forbidden")
    );
    let made = tool_ok(
        &app,
        &review,
        "create_shelf",
        json!({ "slug": "cozy", "title": "Cozy games", "blurb": "Slow and warm.", "apps": ["tea-shop", "cozy-farm"] }),
    )
    .await;
    assert_eq!(shelf_slugs(&made), ["cozy"]);
    tool_ok(
        &app,
        &review,
        "create_shelf",
        json!({ "slug": "spooky", "title": "Spooky", "on_front": false, "apps": ["dark-dungeon"] }),
    )
    .await;
    let updated = tool_ok(
        &app,
        &review,
        "update_shelf",
        json!({ "slug": "cozy", "title": "Cozy picks", "on_front": false, "apps": ["cozy-farm"] }),
    )
    .await;
    let cozy = &updated["shelves"][0];
    assert_eq!(
        (cozy["title"].as_str(), cozy["on_front"].as_bool()),
        (Some("Cozy picks"), Some(false))
    );
    assert_eq!(cozy["apps"][0]["slug"], "cozy-farm");
    assert_eq!(cozy["apps"].as_array().unwrap().len(), 1);

    // A new order of every shelf.
    let ordered = tool_ok(
        &app,
        &review,
        "order_shelves",
        json!({ "slugs": ["spooky", "cozy"] }),
    )
    .await;
    assert_eq!(shelf_slugs(&ordered), ["spooky", "cozy"]);
    assert_eq!(audit_actors(&app, "shelf.reordered").await.len(), 1);

    // Hiding an automatic shelf, then removing a shelf (no body; an empty reply).
    let hidden = tool_ok(
        &app,
        &review,
        "hide_front_shelf",
        json!({ "key": "fresh_slop", "hidden": true }),
    )
    .await;
    assert!(
        hidden["built_in"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["key"] == "fresh_slop" && b["hidden"] == true)
    );
    tool_refused(
        &app,
        &review,
        "hide_front_shelf",
        json!({ "key": "nonsense", "hidden": true }),
    )
    .await;
    let (ok, text) = tool(&app, &review, "delete_shelf", json!({ "slug": "spooky" })).await;
    assert!(ok);
    assert_eq!(text, "Done (HTTP 204).");
    tool_refused(&app, &review, "delete_shelf", json!({ "slug": "spooky" })).await;
    assert_eq!(
        shelf_slugs(&tool_ok(&app, &read, "store_shelves", json!({})).await),
        ["cozy"]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reordering_shelves_takes_every_shelf_exactly_once(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let review = admin_token(&app, &owner, "review").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "tea-shop", "released", "").await;
    for slug in ["cozy", "spooky"] {
        tool_ok(
            &app,
            &review,
            "create_shelf",
            json!({ "slug": slug, "title": slug, "apps": ["tea-shop"] }),
        )
        .await;
    }
    let order = || async {
        let shelves = tool_ok(&app, &review, "store_shelves", json!({})).await;
        shelves["shelves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["slug"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    // A shelf left out, or one that isn't there, is refused and nothing moves.
    for slugs in [
        json!(["spooky"]),
        json!(["spooky", "cozy", "nope"]),
        json!(["spooky", "spooky", "cozy"]),
    ] {
        tool_refused(
            &app,
            &review,
            "order_shelves",
            json!({ "slugs": slugs.clone() }),
        )
        .await;
        assert_eq!(order().await, ["cozy", "spooky"], "{slugs}");
    }
    assert_eq!(audit_actors(&app, "shelf.reordered").await.len(), 0);

    // A shelf can't take the address the reordering lives at (it could never be changed).
    let text = tool_refused(
        &app,
        &review,
        "create_shelf",
        json!({ "slug": "order", "title": "Order", "apps": [] }),
    )
    .await;
    assert!(text.contains("\"field\":\"slug\""), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_fix_apps_and_profiles_through_mcp_tools(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let moderate = admin_token(&app, &owner, "moderate").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    listed_with(&app, maker_id, "sketch", "coming_soon", "status = 'draft'").await;

    // Finding apps, by status and words; one app's whole record.
    let drafts = tool_ok(&app, &read, "find_apps", json!({ "status": "draft" })).await;
    let slugs: Vec<&str> = drafts
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs.len(), 2);
    assert!(slugs.contains(&"game") && slugs.contains(&"sketch"));
    let listed = tool_ok(
        &app,
        &read,
        "find_apps",
        json!({ "status": "listed", "q": "aren" }),
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["slug"], "arena");
    tool_refused(&app, &read, "find_apps", json!({ "status": "gone" })).await;
    let record = tool_ok(&app, &read, "app_record", json!({ "slug": "arena" })).await;
    assert_eq!(record["app"]["maker"], "Maker");
    let text = tool_refused(&app, &read, "app_record", json!({ "slug": "no-such-app" })).await;
    assert!(text.contains("not_found"), "{text}");

    // Fixing a store page: arrays and integers pass through; it needs a reason and a review token.
    let edit = json!({ "slug": "arena", "min_age": 16, "content_labels": ["violence", "horror"],
                       "reason": "The trailer shows gore." });
    assert!(
        tool_refused(&app, &read, "edit_store_page", edit.clone())
            .await
            .contains("forbidden")
    );
    assert_eq!(
        tool_refused(
            &app,
            &review,
            "edit_store_page",
            json!({ "slug": "arena", "min_age": 16 })
        )
        .await,
        "missing argument: reason"
    );
    let changed = tool_ok(&app, &review, "edit_store_page", edit).await;
    assert_eq!(changed["changed"], json!(["age rating", "content labels"]));
    let (age, labels): (i16, Vec<String>) =
        sqlx::query_as("select min_age, content_labels from apps where slug = 'arena'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (age, labels),
        (16, vec!["violence".to_string(), "horror".to_string()])
    );

    // Freezing by hand: the maker is told; lifting it gives null back.
    let freeze =
        json!({ "slug": "arena", "reason": "A player says the last build steals passwords." });
    assert!(
        tool_refused(&app, &read, "freeze_app", freeze.clone())
            .await
            .contains("forbidden")
    );
    let frozen = tool_ok(&app, &review, "freeze_app", freeze.clone()).await;
    assert_eq!(frozen["by_staff"], true);
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Slop froze arena".to_string())
    );
    assert!(
        tool_refused(&app, &review, "freeze_app", freeze)
            .await
            .contains("conflict")
    );
    assert!(
        tool_refused(
            &app,
            &read,
            "unfreeze_app",
            json!({ "slug": "arena", "note": "Fine." })
        )
        .await
        .contains("forbidden")
    );
    let lifted = tool_ok(
        &app,
        &review,
        "unfreeze_app",
        json!({ "slug": "arena", "note": "Checked: a false report." }),
    )
    .await;
    assert!(lifted.is_null());
    tool_refused(
        &app,
        &review,
        "unfreeze_app",
        json!({ "slug": "arena", "note": "Again." }),
    )
    .await;

    // Withdrawing a live release: a moderate token's call.
    let release = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        release.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let withdraw = json!({ "id": release, "reason": "Ships a crypto miner." });
    assert!(
        tool_refused(&app, &review, "withdraw_release", withdraw.clone())
            .await
            .contains("forbidden")
    );
    let (ok, text) = tool(&app, &moderate, "withdraw_release", withdraw.clone()).await;
    assert!(ok, "{text}");
    assert_eq!(text, "Done (HTTP 204).");
    let status: String =
        sqlx::query_scalar("select status::text from releases where id = $1::uuid")
            .bind(&release)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "revoked");
    assert!(
        tool_refused(&app, &moderate, "withdraw_release", withdraw)
            .await
            .contains("only an approved release")
    );

    // An automatic freeze (Slop lost the repository): "unfreezing" checks again, and it stays.
    sqlx::query("update apps set repo_url = $1, repo_checked_at = null, status = 'listed' where slug = 'game'")
        .bind(format!("{}octo/gone", app.fake_base))
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set repo_checked_at = now() where slug <> 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::frozen::run_once(&app.state).await.unwrap();
    let still = tool_ok(
        &app,
        &review,
        "unfreeze_app",
        json!({ "slug": "game", "note": "Asked to check again." }),
    )
    .await;
    assert!(
        still["reason"].as_str().unwrap().contains("can't be read"),
        "{still}"
    );
    assert_eq!(audit_actors(&app, "app.repo_rechecked").await.len(), 1);

    // Profiles and teams: seen and fixed with moderation rights (a review token has them).
    sqlx::query(
        "insert into creatives (user_id, handle, name, headline, ai_use, trader, links)
         values ($1, 'mia', 'Mia', 'Totally the real Studio Ghibli', 'none', false,
                 '[{\"label\": \"Free gems\", \"url\": \"https://scam.example\"}]')",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/teams").cookie(&maker).json(
                json!({ "slug": "mias-crew", "name": "Official Valve Team", "kind": "player" }),
            ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let who = json!({ "id": maker_id.to_string() });
    assert!(
        tool_refused(&app, &read, "user_profile", who.clone())
            .await
            .contains("forbidden")
    );
    let profile = tool_ok(&app, &review, "user_profile", who).await;
    assert_eq!(profile["creative"]["handle"], "mia");
    assert_eq!(profile["teams"][0]["slug"], "mias-crew");
    let fix = json!({
        "id": maker_id.to_string(),
        "creative": { "headline": "Pixel artist", "remove_links": true },
        "reason": "Claiming to be a studio you aren't is impersonation."
    });
    assert!(
        tool_refused(&app, &read, "edit_profile", fix.clone())
            .await
            .contains("forbidden")
    );
    let changed = tool_ok(&app, &review, "edit_profile", fix).await;
    assert_eq!(changed["changed"], json!(["headline", "links"]));
    let (headline, links): (String, Value) =
        sqlx::query_as("select headline, links from creatives where user_id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((headline.as_str(), links), ("Pixel artist", json!([])));
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Slop changed your profile".to_string())
    );
    let team = json!({ "slug": "mias-crew", "name": "Mia's Crew", "reason": "Not affiliated with Valve." });
    assert!(
        tool_refused(&app, &read, "edit_team", team.clone())
            .await
            .contains("forbidden")
    );
    let text = tool_refused(
        &app,
        &review,
        "edit_team",
        json!({ "slug": "no-such-team", "name": "x", "reason": "x" }),
    )
    .await;
    assert!(text.contains("not_found"), "{text}");
    assert_eq!(
        tool_ok(&app, &review, "edit_team", team).await["changed"],
        json!(["name"])
    );
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Slop changed your team's page".to_string())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_pause_and_resume_donations_through_mcp_tools(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 10_000;
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let moderate = admin_token(&app, &owner, "moderate").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", false).await;
    let open_hold = || async {
        sqlx::query_as::<_, (Uuid, String)>(
            "select id, kind from donation_holds where lifted_at is null",
        )
        .fetch_optional(&app.db)
        .await
        .unwrap()
    };

    // Paused for review: a review token may, a read token may not.
    let pause = |kind: &str| {
        json!({ "slug": "arena", "kind": kind, "reason": "A trademark complaint says the name is theirs.",
                "ground": "Complaint TM-1" })
    };
    assert!(
        tool_refused(&app, &read, "pause_donations", pause("review"))
            .await
            .contains("forbidden")
    );
    let placed = tool_ok(&app, &review, "pause_donations", pause("review")).await;
    assert!(placed["hold"].is_string(), "{placed}");
    let (hold, kind) = open_hold().await.unwrap();
    assert_eq!(kind, "review");
    let open = tool_ok(&app, &read, "donation_pauses", json!({})).await;
    assert_eq!(open[0]["slug"], "arena");
    assert_eq!(open[0]["ground"], "Complaint TM-1");

    // The maker asks; the agent keeps it paused, and the maker reads why.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/arena/donations/hold/request")
                .cookie(&maker)
                .json(json!({ "message": "We renamed it." })),
        )
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    let request: Uuid = sqlx::query_scalar("select id from donation_hold_requests")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let answer = json!({ "id": request.to_string(), "lift": false, "answer": "The old name is still in the store page." });
    assert!(
        tool_refused(&app, &read, "answer_donation_request", answer.clone())
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &review, "answer_donation_request", answer.clone()).await;
    assert_eq!(open_hold().await.unwrap().0, hold);
    let (outcome, said): (Option<String>, Option<String>) =
        sqlx::query_as("select outcome, answer from donation_hold_requests where id = $1")
            .bind(request)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (outcome.as_deref(), said.as_deref()),
        (
            Some("kept"),
            Some("The old name is still in the store page.")
        )
    );
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Donations to arena stay paused for now".to_string())
    );
    tool_refused(&app, &review, "answer_donation_request", answer).await;

    // Lifted with a note; then turned off (a moderate token's call), which a review token can't
    // lift.
    tool_ok(
        &app,
        &review,
        "lift_donation_pause",
        json!({ "id": hold.to_string(), "note": "Renamed everywhere now." }),
    )
    .await;
    assert!(open_hold().await.is_none());
    assert!(
        tool_refused(&app, &review, "pause_donations", pause("off"))
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &moderate, "pause_donations", pause("off")).await;
    let (off, kind) = open_hold().await.unwrap();
    assert_eq!(kind, "off");
    let lift = json!({ "id": off.to_string(), "note": "The complaint was withdrawn." });
    assert!(
        tool_refused(&app, &review, "lift_donation_pause", lift.clone())
            .await
            .contains("forbidden")
    );
    tool_ok(&app, &moderate, "lift_donation_pause", lift.clone()).await;
    assert!(open_hold().await.is_none());
    tool_refused(&app, &moderate, "lift_donation_pause", lift).await;

    // The history: none open, both with show=all.
    assert_eq!(
        tool_ok(&app, &read, "donation_pauses", json!({})).await,
        json!([])
    );
    let all = tool_ok(&app, &read, "donation_pauses", json!({ "show": "all" })).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert!(
        all.as_array()
            .unwrap()
            .iter()
            .all(|h| h["lifted_at"].is_string())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agents_sensitive_actions_wait_for_a_second_person(db: PgPool) {
    let app = agents_app(db, 2).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let moderate = admin_token(&app, &ada, "moderate").await;
    let read = admin_token(&app, &ada, "read").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let release = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        release.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();

    // Withdrawing, delisting and turning donations off are recorded and wait.
    for (name, args) in [
        (
            "withdraw_release",
            json!({ "id": release, "reason": "Ships a crypto miner." }),
        ),
        (
            "delist_app",
            json!({ "slug": "arena", "rule": "misleading", "reason": "Impersonates a studio." }),
        ),
        (
            "pause_donations",
            json!({ "slug": "arena", "kind": "off", "reason": "Raises money for a scam." }),
        ),
    ] {
        let waiting = tool_ok(&app, &moderate, name, args).await;
        assert_eq!(waiting["approvals_still_needed"], 1, "{name}: {waiting}");
        assert!(waiting["pending_action"].is_string());
    }
    let status: String =
        sqlx::query_scalar("select status::text from releases where id = $1::uuid")
            .bind(&release)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "approved");
    assert_eq!(
        count(
            &app,
            "select count(*) from apps where slug = 'arena' and status = 'listed'"
        )
        .await,
        1
    );
    assert_eq!(count(&app, "select count(*) from donation_holds").await, 0);

    // The agent sees what waits (by default only that), but only people approve.
    let pending = tool_ok(&app, &read, "pending_actions", json!({})).await;
    let actions = pending["actions"].as_array().unwrap();
    let kinds: Vec<&str> = actions
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["donations.off", "app.delist", "release.revoke"]);
    assert_eq!(pending["approvals_required"], 2);
    let first = actions[2]["id"].as_str().unwrap().to_string();
    assert_eq!(
        app.send(with_token(
            req(Method::POST, format!("/v1/admin/actions/{first}/approve")),
            &moderate
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/admin/actions/{first}/approve")).cookie(&ben))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    // It can withdraw its own request.
    let delist = actions[1]["id"].as_str().unwrap().to_string();
    tool_ok(
        &app,
        &moderate,
        "cancel_pending_action",
        json!({ "id": delist }),
    )
    .await;
    let left = tool_ok(&app, &read, "pending_actions", json!({})).await;
    assert_eq!(left["actions"].as_array().unwrap().len(), 1);
    let all = tool_ok(&app, &read, "pending_actions", json!({ "status": "all" })).await;
    let statuses: Vec<&str> = all["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["pending", "cancelled", "done"]);
    let status: String =
        sqlx::query_scalar("select status::text from releases where id = $1::uuid")
            .bind(&release)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "revoked");
}

// ---------------------------------------------------------------------------------------------
// Makers' agents (the reported problems in this area)

/// A maker agent token made with this cookie, as an Authorization header value.
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

/// Calls one maker MCP tool: (whether it worked, the text it gave back).
async fn maker_tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/maker/mcp").json(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": args }
            })),
            auth,
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let reply = body_json(res).await;
    (
        reply["result"]["isError"] == false,
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

async fn bounty(app: &TestApp, slug: &str, by: Uuid, title: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into bounties (id, app_id, created_by, title, body, currency)
         values ($1, (select id from apps where slug = $2), $3, $4, 'Please fix it.', 'usd')",
    )
    .bind(id)
    .bind(slug)
    .bind(by)
    .bind(title)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_lists_one_apps_bounties(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (mia, maker) = app.user("Mia Maker", "US").await;
    let (otto, _) = app.user("Otto Other", "US").await;
    let (pat, _) = app.user("Pat Player", "US").await;
    listed_with(&app, mia, "pong", "released", "").await;
    listed_with(&app, otto, "arena", "released", "").await;
    bounty(&app, "pong", pat, "Add a two-player mode").await;
    bounty(&app, "arena", pat, "Fix the crash on start").await;
    let token = maker_token(&app, &maker, json!({ "name": "Claude", "scope": "read" })).await;

    // The list across the store doesn't promise an app filter it hasn't got; one app's is its
    // own tool.
    let res = app
        .send(with_token(
            req(Method::POST, "/v1/maker/mcp")
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
            &token,
        ))
        .await;
    let tools = body_json(res).await["result"]["tools"].clone();
    let find = |name: &str| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))
            .clone()
    };
    let list = find("list_bounties");
    let props = list["inputSchema"]["properties"].as_object().unwrap();
    let promises_filter = list["description"]
        .as_str()
        .unwrap()
        .contains("With an app's address");
    assert!(props.contains_key("slug") || !promises_filter, "{list}");
    assert_eq!(
        find("app_bounties")["inputSchema"]["required"],
        json!(["slug"])
    );

    let (ok, text) = maker_tool(&app, &token, "list_bounties", json!({})).await;
    assert!(ok, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (ok, text) = maker_tool(&app, &token, "app_bounties", json!({ "slug": "pong" })).await;
    assert!(ok, "{text}");
    let pong: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(pong.as_array().unwrap().len(), 1);
    assert_eq!(pong[0]["title"], "Add a two-player mode");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_clients_agent_cant_withdraw_their_job_request(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (mia, maker) = app.user("Mia Maker", "US").await;
    let (cora, cora_cookie) = app.user("Cora Creative", "US").await;
    listed_with(&app, mia, "pong", "released", "").await;
    let pong = app_id(&app, "pong").await;
    let first = people_job(&app, cora, mia, pong, "Sprites for Pong").await;
    let second = people_job(&app, cora, mia, pong, "Music for Pong").await;
    for job in [first, second] {
        sqlx::query("update jobs set status = 'requested', offer = null where id = $1")
            .bind(job)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let people = json!({ "name": "Claude", "scope": "publish", "people": true });
    let mia_agent = maker_token(&app, &maker, people.clone()).await;
    let cora_agent = maker_token(&app, &cora_cookie, people).await;
    let decline = |job: Uuid| {
        req(Method::POST, format!("/v1/jobs/{job}/decline"))
            .json(json!({ "note": "Changed my mind." }))
    };
    let status = |job: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select status from jobs where id = $1")
                .bind(job)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // Withdrawing a request stays with the person: the client's agent can't.
    let res = app.send(with_token(decline(first), &mia_agent)).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(status(first).await, "requested");
    // The creative's agent may decline the work; the client herself may withdraw.
    let res = app.send(with_token(decline(first), &cora_agent)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(status(first).await, "declined");
    let res = app.send(decline(second).cookie(&maker)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(status(second).await, "declined");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_app_limited_makers_agent_answers_its_apps_bug_reports_and_claims_their_bounties(
    db: PgPool,
) {
    let app = agents_app(db, 1).await;
    let (mia, maker) = app.user("Mia Maker", "US").await;
    let (pat, _) = app.user("Pat Player", "US").await;
    listed_with(&app, mia, "pong", "released", "").await;
    listed_with(&app, mia, "other-game", "released", "").await;
    let report = |slug: &str| {
        let id = Uuid::now_v7();
        let db = app.db.clone();
        let slug = slug.to_string();
        async move {
            sqlx::query(
                "insert into bug_reports (id, app_id, reporter_id, title, body)
                 values ($1, (select id from apps where slug = $2), $3, 'Crashes', 'On start.')",
            )
            .bind(id)
            .bind(slug)
            .bind(pat)
            .execute(&db)
            .await
            .unwrap();
            id
        }
    };
    let pong_report = report("pong").await;
    let other_report = report("other-game").await;
    let token = maker_token(
        &app,
        &maker,
        json!({ "name": "Pong agent", "scope": "publish", "apps": ["pong"] }),
    )
    .await;
    let fix = |id: Uuid| {
        req(Method::PATCH, format!("/v1/bug-reports/{id}"))
            .json(json!({ "status": "fixed", "maker_note": "Fixed in 1.0.1." }))
    };

    // Its own app's reports, yes; another app's (even the same maker's), no.
    let res = app.send(with_token(fix(pong_report), &token)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["status"], "fixed");
    let res = app.send(with_token(fix(other_report), &token)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app.send(with_token(fix(Uuid::now_v7()), &token)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let status: String = sqlx::query_scalar("select status from bug_reports where id = $1")
        .bind(other_report)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "open");

    // Claiming a bounty on its app gets as far as the claim's own checks; for another app, no.
    let wanted = bounty(&app, "pong", pat, "Add a two-player mode").await;
    let claim = |slug: &str| {
        req(Method::POST, format!("/v1/bounties/{wanted}/claims"))
            .json(json!({ "app_slug": slug, "note": "1.1 adds it." }))
    };
    let res = app.send(with_token(claim("pong"), &token)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("release it first")
    );
    let res = app.send(with_token(claim("other-game"), &token)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_maker_can_never_hold_more_than_ten_agent_tokens(db: PgPool) {
    let app = agents_app(db, 1).await;
    let (mia, maker) = app.user("Mia Maker", "US").await;
    // Fifteen requests at the same moment.
    let mut tasks = vec![];
    for i in 0..15 {
        let router = app.router.clone();
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/me/maker-tokens")
            .header(header::HOST, "localhost:8080")
            .header(header::COOKIE, maker.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": format!("Agent {i}"), "scope": "read" }).to_string(),
            ))
            .unwrap();
        tasks.push(tokio::spawn(async move {
            router.oneshot(request).await.unwrap().status()
        }));
    }
    let mut made = 0;
    for t in tasks {
        let status = t.await.unwrap();
        if status == StatusCode::CREATED {
            made += 1;
        } else {
            assert_eq!(status, StatusCode::CONFLICT);
        }
    }
    let live = count(
        &app,
        &format!(
            "select count(*) from sessions where user_id = '{mia}' and kind = 'agent' and revoked_at is null"
        ),
    )
    .await;
    assert_eq!((made, live), (10, 10));
}
