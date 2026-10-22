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
