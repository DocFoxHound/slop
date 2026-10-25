// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 2 of the agents tests: the admin MCP tools for shelves, legal documents and staff; maker
//! agent tokens (making, listing, revoking, signing in, the allow-list) and the maker MCP tools.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// An admin API token with this scope, made over HTTP by a freshly signed-in admin.
async fn admin_token(app: &TestApp, cookie: &str, scope: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(cookie)
                .json(json!({ "name": format!("Claude {scope}"), "scope": scope, "days": 30 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
}

/// Hands an admin token over to another staff member (only security staff make tokens).
async fn give_token(app: &TestApp, token: &str, to: Uuid) {
    sqlx::query("update admin_tokens set user_id = $1 where token_hash = $2")
        .bind(to)
        .bind(slop_api::tokens::hash_token(
            token.trim_start_matches("Bearer "),
        ))
        .execute(&app.db)
        .await
        .unwrap();
}

/// A maker agent token made over HTTP. Returns the authorization header and the token's ID.
async fn maker_token(app: &TestApp, cookie: &str, body: Value) -> (String, String) {
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(cookie)
                .json(body),
        )
        .await;
    let status = res.status();
    let created = body_json(res).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    (
        format!("Bearer {}", created["secret"].as_str().unwrap()),
        created["id"].as_str().unwrap().to_string(),
    )
}

/// One JSON-RPC call to an MCP endpoint (`/v1/admin/mcp` or `/v1/maker/mcp`).
async fn rpc(app: &TestApp, endpoint: &str, auth: &str, method: &str, params: Value) -> Value {
    let res = app
        .send(
            req(Method::POST, endpoint)
                .header("authorization", auth.to_string())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// Calls a tool. Returns (whether it failed, its text).
async fn tool(
    app: &TestApp,
    endpoint: &str,
    auth: &str,
    name: &str,
    args: Value,
) -> (bool, String) {
    let reply = rpc(
        app,
        endpoint,
        auth,
        "tools/call",
        json!({ "name": name, "arguments": args }),
    )
    .await;
    let result = &reply["result"];
    assert!(result.is_object(), "{reply}");
    (
        result["isError"].as_bool().unwrap(),
        result["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

async fn admin_tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    tool(app, "/v1/admin/mcp", auth, name, args).await
}

async fn maker_tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    tool(app, "/v1/maker/mcp", auth, name, args).await
}

/// A tool's reply as JSON.
fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// A staff member with exactly these roles (not an owner unless asked).
async fn make_staff(app: &TestApp, name: &str, roles: &[&str]) -> (Uuid, String) {
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

/// A count, with `$1` bound to `id`.
async fn count_for(app: &TestApp, sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn is_staff(app: &TestApp, id: Uuid) -> bool {
    sqlx::query_scalar("select is_admin from users where id = $1")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn notification_titles(app: &TestApp, user: Uuid) -> Vec<String> {
    sqlx::query_scalar("select title from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Admin MCP: shelves, legal documents, staff

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agent_hides_and_shows_an_automatic_front_shelf(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let hide = json!({ "key": "fresh_slop", "hidden": true });

    let (failed, text) = admin_tool(&app, &read, "hide_front_shelf", hide.clone()).await;
    assert!(
        failed && text.contains("forbidden"),
        "a read token only looks: {text}"
    );
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!([]));

    let (failed, text) = admin_tool(&app, &review, "hide_front_shelf", hide).await;
    assert!(!failed, "{text}");
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!(["fresh_slop"]));
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'shelf.built_in_hidden'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        actor.starts_with("AI reviewer \"Claude review\""),
        "{actor}"
    );

    // An unknown key is an error and changes nothing; so is leaving out `hidden`.
    let (failed, text) = admin_tool(
        &app,
        &review,
        "hide_front_shelf",
        json!({ "key": "secret_shelf", "hidden": true }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &review,
        "hide_front_shelf",
        json!({ "key": "popular" }),
    )
    .await;
    assert!(failed && text.contains("hidden"), "{text}");
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!(["fresh_slop"]));

    let (failed, _) = admin_tool(
        &app,
        &review,
        "hide_front_shelf",
        json!({ "key": "fresh_slop", "hidden": false }),
    )
    .await;
    assert!(!failed);
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agent_reads_publishes_and_withdraws_legal_documents(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, player) = app.user("Pia Player", "US").await;
    let read = admin_token(&app, &owner, "read").await;
    let review = admin_token(&app, &owner, "review").await;
    let moderate = admin_token(&app, &owner, "moderate").await;

    // Any token reads them.
    let (failed, text) = admin_tool(&app, &read, "legal_documents", json!({})).await;
    assert!(!failed, "{text}");
    let docs = parsed(&text);
    let kinds: Vec<&str> = docs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["terms", "privacy", "content", "publisher", "people"]
    );
    let terms = docs[0].clone();
    let in_effect = terms["latest_version"].as_str().unwrap().to_string();
    let words = terms["latest_body_md"].as_str().unwrap().to_string();
    assert!(words.len() > 500);
    assert!(terms["file_text"].is_null(), "it came from the repository");
    let versions = || {
        count(
            &app,
            "select count(*) from legal_documents where kind = 'terms'",
        )
    };

    // Publishing is sensitive: neither a read nor a review token may.
    let new_terms = json!({ "kind": "terms", "summary": "Adds one line.",
                            "body_md": format!("{words}\n\nOne more line.") });
    for token in [&read, &review] {
        let (failed, text) =
            admin_tool(&app, token, "publish_legal_document", new_terms.clone()).await;
        assert!(failed && text.contains("forbidden"), "{text}");
    }
    assert_eq!(versions().await, 1);

    // Over 200,000 characters: refused, nothing stored.
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "terms", "summary": "Huge.", "body_md": "x".repeat(200_001) }),
    )
    .await;
    assert!(failed && text.contains("200000 characters"), "{text}");
    // More than a request may carry (about 1 MiB): refused, nothing stored.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/mcp")
                .header("authorization", moderate.clone())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
                    "name": "publish_legal_document",
                    "arguments": { "kind": "terms", "summary": "Huge.", "body_md": "x".repeat(1200 * 1024) } } })),
        )
        .await;
    // (413 when the client sends its length first; otherwise the reading stops at the limit.)
    if res.status() != StatusCode::PAYLOAD_TOO_LARGE {
        let reply = body_json(res).await;
        assert_eq!(reply["error"]["message"], "request too large", "{reply}");
    }
    assert_eq!(versions().await, 1);
    // An unknown document, a summary over one line, the text already in effect.
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "cookies", "summary": "x", "body_md": "y" }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "terms", "summary": "Two\nlines", "body_md": "y" }),
    )
    .await;
    assert!(failed && text.contains("one line"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "terms", "summary": "Nothing.", "body_md": words }),
    )
    .await;
    assert!(failed && text.contains("already published"), "{text}");
    assert_eq!(versions().await, 1);

    // A moderate token publishes a text just under the limit: in effect 15 days from now.
    let mut big = format!("{words}\n\n");
    while big.len() < 60 * 1024 {
        big.push_str("A long new clause about nothing in particular.\n");
    }
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "terms", "summary": "A much longer text.", "body_md": big }),
    )
    .await;
    assert!(!failed, "{text}");
    let published = parsed(&text);
    let version = published["version"].as_str().unwrap().to_string();
    assert_eq!(version, OffsetDateTime::now_utc().date().to_string());
    let days = (published["effective_at"].as_i64().unwrap()
        - OffsetDateTime::now_utc().unix_timestamp()) as f64
        / 86_400.0;
    assert!((14.99..=15.0).contains(&days), "{days}");
    let (body, by): (String, String) = sqlx::query_as(
        "select body_md, published_by from legal_documents where kind = 'terms' and version = $1",
    )
    .bind(&version)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(body.trim_end(), big.trim_end());
    assert!(by.starts_with("AI reviewer \"Claude moderate\""), "{by}");
    // Everyone is told: players see it in their legal notices, with its one-line summary.
    let notices = body_json(
        app.send(req(Method::GET, "/v1/me/legal").cookie(&player))
            .await,
    )
    .await;
    let notice = notices
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "terms" && n["version"] == version.as_str())
        .unwrap_or_else(|| panic!("{notices}"))
        .clone();
    assert_eq!(notice["summary"], "A much longer text.");
    assert_eq!(notice["in_effect"], false);
    assert_eq!(notice["action"], "accept");
    // One announced change at a time.
    let (failed, text) =
        admin_tool(&app, &moderate, "publish_legal_document", new_terms.clone()).await;
    assert!(failed && text.contains("already announced"), "{text}");

    // Withdrawing: never the version in effect, never by a review token, always with a reason.
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "withdraw_legal_document",
        json!({ "kind": "terms", "version": in_effect, "reason": "Trying." }),
    )
    .await;
    assert!(failed && text.contains("already in effect"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &review,
        "withdraw_legal_document",
        json!({ "kind": "terms", "version": version, "reason": "Trying." }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "withdraw_legal_document",
        json!({ "kind": "terms", "version": version, "reason": "  " }),
    )
    .await;
    assert!(failed && text.contains("reason"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "withdraw_legal_document",
        json!({ "kind": "terms", "version": "2001-01-01", "reason": "Not there." }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    assert_eq!(versions().await, 2);
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "withdraw_legal_document",
        json!({ "kind": "terms", "version": version, "reason": "Published by mistake." }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(versions().await, 1);
    let kept: Value =
        sqlx::query_scalar("select detail from audit_log where action = 'legal.withdrawn'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(kept["reason"], "Published by mistake.");
    assert_eq!(kept["body_md"].as_str().unwrap().trim_end(), big.trim_end());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agents_legal_change_waits_for_a_second_person(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (ben_id, ben) = make_admin(&app, "Ben Owner").await;
    let moderate = admin_token(&app, &ada, "moderate").await;
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "publish_legal_document",
        json!({ "kind": "privacy", "summary": "Shorter.", "body_md": "We keep almost nothing." }),
    )
    .await;
    assert!(!failed, "{text}");
    let pending = parsed(&text);
    assert_eq!(pending["approvals_still_needed"], 1);
    let action = pending["pending_action"].as_str().unwrap().to_string();
    let privacy = || {
        count(
            &app,
            "select count(*) from legal_documents where kind = 'privacy'",
        )
    };
    assert_eq!(privacy().await, 1, "nothing published yet");
    assert!(
        notification_titles(&app, ben_id)
            .await
            .contains(&"A second approval is needed".to_string())
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&ben))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(privacy().await, 2);
    let by: String = sqlx::query_scalar(
        "select published_by from legal_documents where kind = 'privacy' and published_by is not null",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(by.contains("approved by Ben Owner"), "{by}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_list_staff_and_only_ask_for_role_changes(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = make_admin(&app, "Olive Owner").await;
    let (sec_id, sec) = make_staff(&app, "Sam Security", &["security"]).await;
    let (moderator_id, _) = make_staff(&app, "Mo Moderator", &["moderator"]).await;
    let (helper_id, _) = app.user("Hal Helper", "US").await;

    // Any token reads the staff list.
    let read = admin_token(&app, &owner, "read").await;
    let (failed, text) = admin_tool(&app, &read, "staff", json!({})).await;
    assert!(!failed, "{text}");
    let staff = parsed(&text);
    let names: Vec<&str> = staff["staff"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Mo Moderator", "Olive Owner", "Sam Security"]);
    assert_eq!(staff["my_roles"], json!(["owner"]));
    assert_eq!(staff["approvals_required"], 1);

    // Read and review tokens can't ask; a moderate token can, and it waits for a person.
    let ask = json!({ "user_id": helper_id, "role": "reviewer", "grant": true });
    for scope in ["read", "review"] {
        let token = admin_token(&app, &owner, scope).await;
        let (failed, text) = admin_tool(&app, &token, "change_staff_role", ask.clone()).await;
        assert!(failed && text.contains("forbidden"), "{scope}: {text}");
    }
    let moderate = admin_token(&app, &owner, "moderate").await;
    let (failed, text) = admin_tool(&app, &moderate, "change_staff_role", ask.clone()).await;
    assert!(!failed, "{text}");
    let pending = parsed(&text);
    assert_eq!(pending["approvals_still_needed"], 1);
    let action = pending["pending_action"].as_str().unwrap().to_string();
    assert!(!is_staff(&app, helper_id).await);
    // The owner and security staff are told; the moderator, who can't approve it, isn't.
    let asked = "An AI agent asked for a staff change".to_string();
    assert!(notification_titles(&app, owner_id).await.contains(&asked));
    assert!(notification_titles(&app, sec_id).await.contains(&asked));
    assert!(
        !notification_titles(&app, moderator_id)
            .await
            .contains(&asked)
    );
    let audit: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'staff_action.request' and subject = $1",
    )
    .bind(&action)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(audit["needs_a_person"], true);
    // Its own admin approves it, signed in.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(is_staff(&app, helper_id).await);

    // A moderator's moderate token can't: staff changes need the security role.
    let mod_token = admin_token(&app, &owner, "moderate").await;
    give_token(&app, &mod_token, moderator_id).await;
    let (failed, text) = admin_tool(&app, &mod_token, "staff", json!({})).await;
    assert!(!failed);
    assert_eq!(parsed(&text)["my_roles"], json!(["moderator"]));
    let (failed, text) = admin_tool(
        &app,
        &mod_token,
        "change_staff_role",
        json!({ "user_id": helper_id, "role": "support", "grant": true }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");

    // Security staff's agent asks for ordinary roles, but only an owner gives the owner role.
    let sec_token = admin_token(&app, &sec, "moderate").await;
    let (failed, text) = admin_tool(
        &app,
        &sec_token,
        "change_staff_role",
        json!({ "user_id": helper_id, "role": "owner", "grant": true }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &sec_token,
        "change_staff_role",
        json!({ "user_id": helper_id, "role": "support", "grant": true }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(
        count(
            &app,
            "select count(*) from staff_actions where status = 'pending'"
        )
        .await,
        1
    );
    // Bad input: an unknown role or account.
    let (failed, _) = admin_tool(
        &app,
        &sec_token,
        "change_staff_role",
        json!({ "user_id": helper_id, "role": "wizard", "grant": true }),
    )
    .await;
    assert!(failed);
    let (failed, text) = admin_tool(
        &app,
        &sec_token,
        "change_staff_role",
        json!({ "user_id": Uuid::now_v7(), "role": "support", "grant": true }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agent_asks_to_remove_staff_and_a_person_decides(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, sec) = make_staff(&app, "Sam Security", &["security"]).await;
    let (other_owner_id, _) = make_admin(&app, "Otto Owner").await;
    let (rev_id, _) = make_staff(&app, "Rita Reviewer", &["reviewer"]).await;
    // Only security staff make tokens: Rita's is made by the owner and handed over.
    let rita_token = admin_token(&app, &owner, "review").await;
    give_token(&app, &rita_token, rev_id).await;
    let (failed, _) = admin_tool(&app, &rita_token, "staff", json!({})).await;
    assert!(!failed, "her token works");

    let moderate = admin_token(&app, &owner, "moderate").await;
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "remove_from_staff",
        json!({ "user_id": rev_id, "reason": "short" }),
    )
    .await;
    assert!(failed && text.contains("reason"), "{text}");
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "remove_from_staff",
        json!({ "user_id": rev_id, "reason": "No longer reviewing for Slop." }),
    )
    .await;
    assert!(!failed, "{text}");
    let action = parsed(&text)["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    // Pending: still staff, her token still works, and the agent can't approve it.
    assert!(is_staff(&app, rev_id).await);
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/actions/{action}/approve"))
                .header("authorization", moderate.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(is_staff(&app, rev_id).await);
    let (failed, _) = admin_tool(&app, &rita_token, "staff", json!({})).await;
    assert!(!failed);
    // A person approves: every role goes, and every token she made is revoked.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!is_staff(&app, rev_id).await);
    assert_eq!(
        count_for(
            &app,
            "select count(*) from staff_roles where user_id = $1",
            rev_id
        )
        .await,
        0
    );
    assert_eq!(
        count_for(
            &app,
            "select count(*) from admin_tokens where user_id = $1 and revoked_at is null",
            rev_id
        )
        .await,
        0
    );
    let res = app
        .send(req(Method::GET, "/v1/admin/review").header("authorization", rita_token))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Only an owner removes an owner, even with a moderate token of security staff.
    let sec_token = admin_token(&app, &sec, "moderate").await;
    let remove_owner = json!({ "user_id": other_owner_id, "reason": "Trying to remove an owner." });
    let (failed, text) =
        admin_tool(&app, &sec_token, "remove_from_staff", remove_owner.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    // A review token can't ask at all; an account that isn't staff is a conflict.
    let review = admin_token(&app, &owner, "review").await;
    let (failed, text) = admin_tool(&app, &review, "remove_from_staff", remove_owner).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    assert!(is_staff(&app, other_owner_id).await);
    let (failed, text) = admin_tool(
        &app,
        &moderate,
        "remove_from_staff",
        json!({ "user_id": rev_id, "reason": "She isn't staff any more." }),
    )
    .await;
    assert!(failed && text.contains("isn't staff"), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_agent_cancels_its_own_pending_requests_only(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (cleo_id, _) = app.user("Cleo", "US").await;
    let ada_moderate = admin_token(&app, &ada, "moderate").await;
    let ada_review = admin_token(&app, &ada, "review").await;
    let status_of = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select status from staff_actions where id = $1::uuid")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // Ada's agent asks to suspend Cleo, then thinks better of it.
    let (failed, text) = admin_tool(
        &app,
        &ada_moderate,
        "moderate_user",
        json!({ "id": cleo_id, "action": "suspend_account", "rule": "harassment", "facts": "Test." }),
    )
    .await;
    assert!(!failed, "{text}");
    let own = parsed(&text)["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let (failed, text) = admin_tool(
        &app,
        &ada_moderate,
        "cancel_pending_action",
        json!({ "id": own }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(status_of(own.clone()).await, "cancelled");
    let result: String = sqlx::query_scalar("select result from staff_actions where id = $1::uuid")
        .bind(&own)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(
        result.contains("AI reviewer \"Claude moderate\""),
        "{result}"
    );
    // Twice: it's no longer pending. A made-up ID isn't one.
    let (failed, text) = admin_tool(
        &app,
        &ada_moderate,
        "cancel_pending_action",
        json!({ "id": own }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, _) = admin_tool(
        &app,
        &ada_moderate,
        "cancel_pending_action",
        json!({ "id": "not-an-id" }),
    )
    .await;
    assert!(failed);

    // Ben asks for one himself: Ada's agents can't turn it down, only a person can.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{cleo_id}/decide"))
                .cookie(&ben)
                .json(
                    json!({ "action": "suspend_account", "rule": "harassment", "facts": "Again." }),
                ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let bens = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    for token in [&ada_review, &ada_moderate] {
        let (failed, text) =
            admin_tool(&app, token, "cancel_pending_action", json!({ "id": bens })).await;
        assert!(failed && text.contains("forbidden"), "{text}");
    }
    assert_eq!(status_of(bens.clone()).await, "pending");
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{bens}/cancel")).cookie(&ada))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(status_of(bens).await, "cancelled");
}

// ---------------------------------------------------------------------------------------------
// Maker agent tokens

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_tokens_are_checked_when_made(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let make = |body: Value| {
        req(Method::POST, "/v1/me/maker-tokens")
            .cookie(&maker)
            .json(body)
    };
    let refused = |body: Value| {
        let r = make(body);
        async {
            let res = app.send(r).await;
            let status = res.status();
            (status, body_json(res).await)
        }
    };

    // Signed out: no.
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens").json(json!({ "name": "A", "scope": "read" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // Bad input.
    for (body, field) in [
        (json!({ "name": "  ", "scope": "read" }), "name"),
        (json!({ "name": "x".repeat(61), "scope": "read" }), "name"),
        (json!({ "name": "A", "scope": "admin" }), "scope"),
        (json!({ "name": "A", "scope": "read", "days": 0 }), "days"),
        (json!({ "name": "A", "scope": "read", "days": 366 }), "days"),
        (json!({ "name": "A", "scope": "read", "days": -30 }), "days"),
    ] {
        let (status, err) = refused(body.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(err["error"]["field"], field, "{body}: {err}");
    }
    let (status, _) = refused(json!({ "name": "A", "scope": "read", "admin": true })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "unknown fields");
    assert_eq!(
        count(&app, "select count(*) from maker_tokens").await,
        0,
        "nothing was made"
    );

    // 60 characters is fine; days default to 30; the MCP address is the public site's.
    let name = "é".repeat(60);
    let res = app
        .send(make(json!({ "name": name, "scope": "read" })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    assert_eq!(created["name"], name.as_str());
    assert_eq!(created["mcp_url"], "http://localhost:8080/v1/maker/mcp");
    assert_eq!(created["apps"], json!([]));
    assert_eq!(created["people"], false);
    assert_eq!(created["passkey_proved"], false);
    assert_eq!(created["expired"], false);
    // The secret is in `secret` only: never inside a ready-made command (people pasted those into
    // AI chats; the website shows how to keep it in an environment variable instead).
    let secret = created["secret"].as_str().unwrap();
    let mut without = created.clone();
    without.as_object_mut().unwrap().remove("secret");
    assert!(!without.to_string().contains(secret), "{without}");
    assert!(created.get("claude_command").is_none(), "{created}");
    let lifetime = OffsetDateTime::parse(created["expires_at"].as_str().unwrap(), &Rfc3339)
        .unwrap()
        - OffsetDateTime::parse(created["created_at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(
        (lifetime - time::Duration::days(30)).abs() < time::Duration::minutes(1),
        "{lifetime}"
    );
    let (kind, agent, hash): (String, String, Vec<u8>) =
        sqlx::query_as("select kind, user_agent, token_hash from sessions where id = $1::uuid")
            .bind(created["id"].as_str().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(kind, "agent");
    assert_eq!(agent, format!("AI agent: {name}"));
    assert_eq!(
        hash,
        slop_api::tokens::hash_token(created["secret"].as_str().unwrap()),
        "only the hash is kept"
    );
    let res = app
        .send(make(
            json!({ "name": "Week", "scope": "publish", "days": 7, "apps": ["game"] }),
        ))
        .await;
    let week = body_json(res).await;
    assert_eq!(week["apps"], json!(["game"]));
    let lifetime = OffsetDateTime::parse(week["expires_at"].as_str().unwrap(), &Rfc3339).unwrap()
        - OffsetDateTime::now_utc();
    assert!((lifetime - time::Duration::days(7)).abs() < time::Duration::minutes(1));

    // Ten live tokens at most; revoked and expired ones don't count.
    for n in 2..10 {
        maker_token(
            &app,
            &maker,
            json!({ "name": format!("T{n}"), "scope": "read" }),
        )
        .await;
    }
    let (status, err) = refused(json!({ "name": "Eleventh", "scope": "read" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "select id from sessions where user_id = $1 and kind = 'agent' order by created_at limit 2",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    sqlx::query("update sessions set revoked_at = now() where id = $1")
        .bind(ids[0])
        .execute(&app.db)
        .await
        .unwrap();
    maker_token(
        &app,
        &maker,
        json!({ "name": "After a revoke", "scope": "read" }),
    )
    .await;
    let (status, _) = refused(json!({ "name": "Twelfth", "scope": "read" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    sqlx::query("update sessions set expires_at = now() - interval '1 second' where id = $1")
        .bind(ids[1])
        .execute(&app.db)
        .await
        .unwrap();
    maker_token(
        &app,
        &maker,
        json!({ "name": "After an expiry", "scope": "read" }),
    )
    .await;

    // An agent can't make tokens, even a publish one.
    let (status, _) = refused(json!({ "name": "x", "scope": "read" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    sqlx::query("update sessions set revoked_at = now() where user_id = $1 and kind = 'agent'")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (agent, _) =
        maker_token(&app, &maker, json!({ "name": "Agent", "scope": "publish" })).await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .header("authorization", agent)
                .json(json!({ "name": "Child", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // An old session makes one without signing in again or confirming (no step-up).
    sqlx::query("update sessions set created_at = now() - interval '1 hour', stepped_up_at = null where user_id = $1 and kind = 'web'")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, _) = refused(json!({ "name": "Late", "scope": "read" })).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_maker_token_needs_only_a_signed_in_session(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    // Signed in three days ago and never confirmed since.
    sqlx::query(
        "update sessions set created_at = now() - interval '3 days', stepped_up_at = null where user_id = $1",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Claude", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let made = body_json(res).await;
    assert!(made["secret"].as_str().unwrap().starts_with("slopmk_"));
    // Not passkey-proven: no confirmation was made.
    assert_eq!(made["passkey_proved"], json!(false));
    // 30 days unless chosen otherwise, and never more than a year.
    let lifetime = OffsetDateTime::parse(made["expires_at"].as_str().unwrap(), &Rfc3339).unwrap()
        - OffsetDateTime::now_utc();
    assert!(lifetime <= time::Duration::days(30));
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Too long", "scope": "read", "days": 366 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Signed out: no token.
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens").json(json!({ "name": "x", "scope": "read" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // The other step-ups stay: the same old session can't add a signing key.
    let res = app
        .send(
            req(Method::POST, "/v1/me/signing-keys")
                .cookie(&maker)
                .json(json!({ "name": "Laptop", "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl x@y" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "step_up_required");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_publishing_token_needs_the_publisher_agreement(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Nia Newcomer", "US").await;
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'publisher'")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let make = |scope: &str| {
        req(Method::POST, "/v1/me/maker-tokens")
            .cookie(&maker)
            .json(json!({ "name": format!("Claude {scope}"), "scope": scope }))
    };
    let mine = || async {
        body_json(
            app.send(req(Method::GET, "/v1/me/legal/publisher").cookie(&maker))
                .await,
        )
        .await
    };
    let status = mine().await;
    assert_eq!(status["accepts"], json!(true));
    assert_eq!(status["accepted"], json!(false));
    let version = status["version"].as_str().unwrap().to_string();

    // Publishing: refused until it's accepted, naming the agreement and where to accept it.
    let res = app.send(make("publish")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(err["error"]["code"], "terms_required");
    assert_eq!(err["error"]["field"], "publisher");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Account → AI agents"),
        "{err}"
    );
    assert_eq!(
        count(&app, "select count(*) from maker_tokens").await,
        0,
        "nothing was made"
    );
    // Looking doesn't need it.
    assert_eq!(app.send(make("read")).await.status(), StatusCode::CREATED);

    // Accepted: the same request makes the token.
    let res = app
        .send(
            req(Method::POST, "/v1/me/legal/accept")
                .cookie(&maker)
                .json(json!({ "kind": "publisher", "version": version })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(mine().await["accepted"], json!(true));
    let (agent, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;

    // A new version takes effect: the agent's tools wait for the owner, and say where.
    sqlx::query(
        "insert into legal_documents (kind, version, title, summary, body_md, sha256, effective_at)
         values ('publisher', '2099-01-01', 'Publisher Agreement', 'Clearer AI rules.', 'New words.', 'x', now())",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let status = mine().await;
    assert_eq!(status["version"], "2099-01-01");
    assert_eq!(status["accepted"], json!(false));
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .header("authorization", agent.clone())
                .json(new_app("agent-made")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(err["error"]["code"], "terms_required");
    assert_eq!(err["error"]["field"], "publisher");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Account → AI agents"),
        "{err}"
    );
    // The token can't accept it itself.
    let res = app
        .send(
            req(Method::POST, "/v1/me/legal/accept")
                .header("authorization", agent.clone())
                .json(json!({ "kind": "publisher", "version": "2099-01-01" })),
        )
        .await;
    assert!(res.status().is_client_error());
    assert_eq!(mine().await["accepted"], json!(false));

    // Policies are read, not accepted; unknown documents aren't found; signed out: no.
    let privacy = body_json(
        app.send(req(Method::GET, "/v1/me/legal/privacy").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(privacy["accepts"], json!(false));
    assert_eq!(privacy["accepted"], json!(false));
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/legal/nothing").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/legal/publisher"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // The document says whether it's accepted or read.
    let doc = body_json(app.send(req(Method::GET, "/v1/legal/people")).await).await;
    assert_eq!(doc["accepted"], json!(true));
    let doc = body_json(app.send(req(Method::GET, "/v1/legal/content")).await).await;
    assert_eq!(doc["accepted"], json!(false));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_recent_passkey_confirmation_carries_over_to_a_maker_token(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let set_passkey_at = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query(sql).bind(maker_id).execute(&db).await.unwrap();
        }
    };
    // A passkey, and the deadline to use one is past.
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, '\\x01', '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update users set passkey_due_at = now() - interval '1 day' where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();

    set_passkey_at(
        "update sessions set passkey_at = now() - interval '13 hours' where user_id = $1 and kind = 'web'",
    )
    .await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Old", "scope": "publish" })),
        )
        .await;
    let old = body_json(res).await;
    assert_eq!(old["passkey_proved"], false);
    set_passkey_at(
        "update sessions set passkey_at = now() - interval '11 hours' where user_id = $1 and kind = 'web'",
    )
    .await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Fresh", "scope": "publish" })),
        )
        .await;
    let fresh = body_json(res).await;
    assert_eq!(fresh["passkey_proved"], true);
    let at: Option<OffsetDateTime> =
        sqlx::query_scalar("select passkey_at from sessions where id = $1::uuid")
            .bind(fresh["id"].as_str().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(at.is_some());

    // Through the agent: the proven token publishes, even after the 12 hours; the other can't.
    let bearer = |t: &Value| format!("Bearer {}", t["secret"].as_str().unwrap());
    sqlx::query("update sessions set passkey_at = passkey_at - interval '2 days' where kind = 'agent' and passkey_at is not null")
        .execute(&app.db)
        .await
        .unwrap();
    let (failed, text) = maker_tool(
        &app,
        &bearer(&old),
        "create_store_page",
        new_app("unproved"),
    )
    .await;
    assert!(failed && text.contains("passkey_required"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &bearer(&fresh),
        "create_store_page",
        new_app("proved"),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["slug"], "proved");
    // Listing shows which is which.
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    let proved: Vec<(String, bool)> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["passkey_proved"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        proved,
        [("Fresh".to_string(), true), ("Old".to_string(), false)]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_list_and_revoke_only_their_own_live_tokens(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "beta-app", "beta", "").await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (people, people_id) = maker_token(
        &app,
        &maker,
        json!({ "name": "People", "scope": "publish", "people": true, "apps": ["game", "beta-app"] }),
    )
    .await;
    let (_, expired_id) =
        maker_token(&app, &maker, json!({ "name": "Expired", "scope": "read" })).await;
    let (_, revoked_id) =
        maker_token(&app, &maker, json!({ "name": "Revoked", "scope": "read" })).await;
    let (others, others_id) =
        maker_token(&app, &other, json!({ "name": "Otto's", "scope": "read" })).await;
    sqlx::query("update sessions set expires_at = now() - interval '1 second' where id = $1::uuid")
        .bind(&expired_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::DELETE, format!("/v1/me/maker-tokens/{revoked_id}")).cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        app.send(req(Method::GET, "/v1/me/maker-tokens"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    let list = list.as_array().unwrap();
    // The live one, and the expired one (listed for a week, marked expired); not the removed one.
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(list[0]["id"], expired_id.as_str(), "newest first");
    assert_eq!(list[0]["expired"], true);
    let list = &list[1..];
    assert_eq!(list[0]["expired"], false);
    assert_eq!(list[0]["id"], people_id.as_str());
    assert_eq!(list[0]["scope"], "publish");
    assert_eq!(list[0]["apps"], json!(["beta-app", "game"]));
    assert_eq!(list[0]["people"], true);
    assert_eq!(list[0]["passkey_proved"], false);
    assert!(list[0]["last_used_at"].is_string());
    assert!(list[0].get("secret").is_none(), "the secret is shown once");
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&other))
            .await,
    )
    .await;
    assert_eq!(theirs.as_array().unwrap().len(), 1);
    assert_eq!(theirs[0]["id"], others_id.as_str());

    // Revoking: another maker's token, a revoked one, a web session, or by an agent: no.
    let revoke = |id: &str, cookie: &str| {
        req(Method::DELETE, format!("/v1/me/maker-tokens/{id}")).cookie(cookie)
    };
    assert_eq!(
        app.send(revoke(&others_id, &maker)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").header("authorization", others.clone()))
            .await
            .status(),
        StatusCode::OK,
        "still live"
    );
    assert_eq!(
        app.send(revoke(&revoked_id, &maker)).await.status(),
        StatusCode::NOT_FOUND,
        "twice"
    );
    let web: Uuid =
        sqlx::query_scalar("select id from sessions where user_id = $1 and kind = 'web'")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        app.send(revoke(&web.to_string(), &maker)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&maker))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(
            req(Method::DELETE, format!("/v1/me/maker-tokens/{people_id}"))
                .header("authorization", people.clone())
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(req(
            Method::DELETE,
            format!("/v1/me/maker-tokens/{people_id}")
        ))
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(revoke(&people_id, &maker)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").header("authorization", people))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_tokens_sign_in_only_while_live(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (agent, id) = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let me = || req(Method::GET, "/v1/me/apps").header("authorization", agent.clone());
    let set = |sql: &'static str| {
        let db = app.db.clone();
        let id = id.clone();
        async move {
            sqlx::query(sql).bind(id).execute(&db).await.unwrap();
        }
    };
    let last_seen = || {
        let db = app.db.clone();
        let id = id.clone();
        async move {
            sqlx::query_scalar::<_, OffsetDateTime>(
                "select last_seen_at from sessions where id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(app.send(me()).await.status(), StatusCode::OK);

    // Seen: touched at most every 5 minutes.
    set("update sessions set last_seen_at = now() - interval '2 minutes' where id = $1::uuid")
        .await;
    let before = last_seen().await;
    app.send(me()).await;
    assert_eq!(last_seen().await, before, "not touched within 5 minutes");
    set("update sessions set last_seen_at = now() - interval '6 minutes' where id = $1::uuid")
        .await;
    app.send(me()).await;
    assert!(OffsetDateTime::now_utc() - last_seen().await < time::Duration::minutes(1));

    // Idle: no limit. A token lasts until the expiry its owner chose (up to a year), however
    // long it goes unused.
    set("update sessions set last_seen_at = now() - interval '20 days' where id = $1::uuid").await;
    assert_eq!(app.send(me()).await.status(), StatusCode::OK);
    set("update sessions set last_seen_at = now() - interval '200 days' where id = $1::uuid").await;
    assert_eq!(app.send(me()).await.status(), StatusCode::OK);

    // Expired.
    set("update sessions set expires_at = now() - interval '1 second' where id = $1::uuid").await;
    assert_eq!(app.send(me()).await.status(), StatusCode::UNAUTHORIZED);
    set("update sessions set expires_at = now() + interval '1 day' where id = $1::uuid").await;

    // Suspended: the agent may only look.
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(app.send(me()).await.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .header("authorization", agent.clone())
                .json(json!({ "summary": "Changed while suspended." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "suspended");
    // The MCP endpoint is a POST, so a suspended maker's agent can't use it at all.
    let res = app
        .send(
            req(Method::POST, "/v1/maker/mcp")
                .header("authorization", agent.clone())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
                    "name": "update_store_page", "arguments": { "slug": "game", "summary": "Changed while suspended." } } })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["code"], "suspended");
    let summary: String = sqlx::query_scalar("select summary from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(summary, "A game");

    // Closed: signed out.
    sqlx::query("update users set suspended_at = null, closed_at = now() where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(app.send(me()).await.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_maker_token_is_only_ever_a_bearer_token(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (agent, _) = maker_token(&app, &maker, json!({ "name": "Claude", "scope": "read" })).await;
    let secret = agent.trim_start_matches("Bearer ").to_string();
    let cookie_name = maker.split('=').next().unwrap();

    // As the website's cookie, it's nobody.
    let res = app
        .send(req(Method::GET, "/v1/me").cookie(format!("{cookie_name}={secret}")))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // An unknown slopmk_ token is refused even next to a good cookie: no falling back.
    let res = app
        .send(
            req(Method::GET, "/v1/me")
                .cookie(&maker)
                .header("authorization", "Bearer slopmk_not-a-real-token"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // A real one next to the cookie is the agent, with the agent's limits.
    let res = app
        .send(
            req(Method::GET, "/v1/me/export")
                .cookie(&maker)
                .header("authorization", agent.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // On public pages too, an unknown token is no one rather than an error.
    let res = app
        .send(req(Method::GET, "/v1/apps").header("authorization", "Bearer slopmk_nope"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    // A real one there is refused: the store's list isn't maker work.
    let res = app
        .send(req(Method::GET, "/v1/apps").header("authorization", agent))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------------------------
// The maker agent allow-list and what agents see elsewhere

/// A listed app with an approved 1.0.0 and a player's bug report on it. Returns the report's ID
/// and the player's.
async fn app_with_bug_report(app: &TestApp, maker: Uuid, slug: &str) -> (String, Uuid, String) {
    listed_with(app, maker, slug, "released", "").await;
    approved_release(app, slug, maker).await;
    let (player_id, player) = app.user("Pat Player", "US").await;
    app.send(req(Method::POST, format!("/v1/apps/{slug}/launch")).cookie(&player))
        .await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/apps/{slug}/bug-reports"))
                .cookie(&player)
                .json(json!({
                    "title": "Crash when the wave starts", "body": "Wave 3 always crashes.",
                    "version": "1.0.0", "platform": "windows", "log": "panic at wave.rs:12 for pat@example.com"
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    (
        body_json(res).await["id"].as_str().unwrap().to_string(),
        player_id,
        player,
    )
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn app_limited_tokens_stay_on_their_apps(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (report, _, _) = app_with_bug_report(&app, maker_id, "tower").await;
    let (limited, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Game only", "scope": "publish", "apps": ["game"] }),
    )
    .await;
    let (all, _) = maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let app = &app;
    let send = |auth: &str, m: Method, path: &str, body: Option<Value>| {
        let mut r = req(m, path).header("authorization", auth.to_string());
        if let Some(b) = body {
            r = r.json(b);
        }
        async move { app.send(r).await.status() }
    };

    // Its own app, yes; the maker's other app is as if it weren't there.
    assert_eq!(
        send(&limited, Method::GET, "/v1/apps/game/releases", None).await,
        StatusCode::OK
    );
    for (m, path, body) in [
        (Method::GET, "/v1/apps/tower/releases", None),
        (Method::GET, "/v1/apps/tower/manage", None),
        (Method::GET, "/v1/apps/tower/bug-reports", None),
        (
            Method::PATCH,
            "/v1/apps/tower",
            Some(json!({ "summary": "Changed by the wrong agent." })),
        ),
        (Method::GET, "/v1/apps/no-such-app/releases", None),
    ] {
        assert_eq!(
            send(&limited, m.clone(), path, body.clone()).await,
            StatusCode::NOT_FOUND,
            "{m} {path}"
        );
    }
    // A report on the other app is hidden too, though its route has no app in it.
    assert_eq!(
        send(
            &limited,
            Method::GET,
            &format!("/v1/bug-reports/{report}"),
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &all,
            Method::GET,
            &format!("/v1/bug-reports/{report}"),
            None
        )
        .await,
        StatusCode::OK
    );
    // Public pages of any app, yes.
    assert_eq!(
        send(&limited, Method::GET, "/v1/apps/tower", None).await,
        StatusCode::OK
    );

    // Nothing new: no new apps or community builds, which aren't tied to one of its apps.
    for (m, path, body) in [
        (
            Method::POST,
            "/v1/apps".to_string(),
            Some(new_app("new-one")),
        ),
        (
            Method::POST,
            "/v1/apps/tower/branches".to_string(),
            Some(json!({})),
        ),
        (
            Method::POST,
            "/v1/apps/game/branches".to_string(),
            Some(json!({})),
        ),
    ] {
        assert_eq!(
            send(&limited, m.clone(), &path, body).await,
            StatusCode::FORBIDDEN,
            "{m} {path}"
        );
    }
    // Reports and claims are found by their own IDs: another app's report isn't there for it.
    assert_eq!(
        send(
            &limited,
            Method::PATCH,
            &format!("/v1/bug-reports/{report}"),
            Some(json!({ "status": "fixed" })),
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let summary: String = sqlx::query_scalar("select summary from apps where slug = 'tower'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(summary, "An app about tower");
    assert_eq!(
        count(app, "select count(*) from apps").await,
        2,
        "no new app"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_agents_never_reach_money_chat_friends_or_sign_ins(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (agent, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Everything", "scope": "publish", "people": true }),
    )
    .await;
    let (read_people, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Reader", "scope": "read", "people": true }),
    )
    .await;
    let fake = Uuid::now_v7();
    let app = &app;
    let status = |auth: &str, m: Method, path: String| {
        let r = req(m, path)
            .header("authorization", auth.to_string())
            .json(json!({}));
        async move { app.send(r).await.status() }
    };
    for (m, path) in [
        (Method::POST, "/v1/apps/tower/donate".to_string()),
        (Method::GET, "/v1/apps/tower/donate".to_string()),
        (Method::POST, format!("/v1/bounties/{fake}/pledge")),
        (Method::POST, "/v1/apps/tower/bounties".to_string()),
        (Method::GET, "/v1/me/donation-account".to_string()),
        (Method::POST, "/v1/me/donation-account/change".to_string()),
        (Method::POST, "/v1/me/creative/stripe".to_string()),
        (Method::GET, "/v1/chat/conversations".to_string()),
        (Method::GET, "/v1/chat/devices".to_string()),
        (Method::GET, "/v1/me/friends".to_string()),
        (Method::POST, "/v1/me/friends".to_string()),
        (Method::GET, "/v1/me/sessions".to_string()),
        (Method::DELETE, format!("/v1/me/sessions/{fake}")),
        (Method::PUT, "/v1/me/creative".to_string()),
        (Method::POST, format!("/v1/jobs/{fake}/accept")),
        (Method::POST, format!("/v1/jobs/{fake}/pay")),
        (Method::POST, format!("/v1/jobs/{fake}/change/accept")),
        (Method::POST, format!("/v1/jobs/{fake}/accept-delivery")),
        (Method::POST, "/v1/people/vera/hire".to_string()),
        (Method::POST, "/v1/apps/tower/vote".to_string()),
        (Method::POST, "/v1/me/legal/accept".to_string()),
        // The store's own lists are for people: only the pages of single apps are maker work.
        (Method::GET, "/v1/apps".to_string()),
        (Method::GET, "/v1/me/library".to_string()),
    ] {
        assert_eq!(
            status(&agent, m.clone(), path.clone()).await,
            StatusCode::FORBIDDEN,
            "{m} {path}"
        );
    }
    // Only the methods listed: HEAD and OPTIONS aren't GET.
    let head = app
        .send(req(Method::HEAD, "/v1/me/apps").header("authorization", agent.clone()))
        .await;
    assert_eq!(head.status(), StatusCode::FORBIDDEN);
    let options = app
        .send(req(Method::OPTIONS, "/v1/me/apps").header("authorization", agent.clone()))
        .await;
    assert!(!options.status().is_success(), "{}", options.status());

    // People work with People access: reading with a read token, answering only with publish.
    assert_eq!(
        status(&read_people, Method::GET, "/v1/me/jobs".into()).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&read_people, Method::GET, "/v1/people/skills".into()).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&read_people, Method::GET, format!("/v1/jobs/{fake}")).await,
        StatusCode::NOT_FOUND,
        "allowed, but no such job"
    );
    for path in ["offer", "decline", "messages", "conversation/open"] {
        assert_eq!(
            status(
                &read_people,
                Method::POST,
                format!("/v1/jobs/{fake}/{path}")
            )
            .await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        // Past the allow-list: the job (or the body) is what's wrong.
        let past = status(&agent, Method::POST, format!("/v1/jobs/{fake}/{path}")).await;
        assert!(
            past == StatusCode::NOT_FOUND || past == StatusCode::UNPROCESSABLE_ENTITY,
            "{path}: {past}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_see_only_maker_notifications(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    for (category, title) in [
        ("apps", "Release 1.0.0 approved"),
        ("jobs", "A new job request"),
        ("chat", "New message from Sam"),
        ("friends", "Sam wants to be friends"),
        ("security", "A new sign-in from Firefox"),
    ] {
        sqlx::query(
            "insert into notifications (id, user_id, category, title, link) values ($1, $2, $3, $4, '/')",
        )
        .bind(Uuid::now_v7())
        .bind(maker_id)
        .bind(category)
        .bind(title)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let (people, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "People", "scope": "read", "people": true }),
    )
    .await;
    let (plain, _) = maker_token(&app, &maker, json!({ "name": "Plain", "scope": "read" })).await;
    let titles = |inbox: &Value| -> Vec<String> {
        let mut t: Vec<String> = inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["title"].as_str().unwrap().to_string())
            .collect();
        t.sort();
        t
    };

    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(titles(&mine).len(), 5);
    assert_eq!(mine["unread"], 5);

    // REST and the whats_new tool agree.
    for (auth, want) in [
        (&people, vec!["A new job request", "Release 1.0.0 approved"]),
        (&plain, vec!["Release 1.0.0 approved"]),
    ] {
        let inbox = body_json(
            app.send(
                req(Method::GET, "/v1/me/notifications").header("authorization", auth.clone()),
            )
            .await,
        )
        .await;
        assert_eq!(titles(&inbox), want);
        let (failed, text) = maker_tool(&app, auth, "whats_new", json!({})).await;
        assert!(!failed, "{text}");
        let inbox = parsed(&text);
        assert_eq!(titles(&inbox), want);
        // Not even how many there are of the hidden kinds, here or on its account.
        assert_eq!(inbox["unread"], want.len(), "{inbox}");
        let me = body_json(
            app.send(req(Method::GET, "/v1/me").header("authorization", auth.clone()))
                .await,
        )
        .await;
        assert_eq!(me["unread_notifications"], want.len(), "{me}");
    }
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&maker)).await).await;
    assert_eq!(me["unread_notifications"], 5);
}

// ---------------------------------------------------------------------------------------------
// Token lifetimes and expiry warnings

/// Moves a token's expiry (and, so it counts as a long-lived token, when it was made).
async fn expire_in(app: &TestApp, id: &str, made_days_ago: i64, expires_in: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update sessions set created_at = now() - interval '{made_days_ago} days',
                expires_at = now() + interval '{expires_in}' where id = $1::uuid"
    )))
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn warned(app: &TestApp, id: &str) -> i16 {
    sqlx::query_scalar("select expiry_warning from maker_tokens where session_id = $1::uuid")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn warn(app: &TestApp) -> usize {
    slop_api::maker_agents::warn_expiring(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn maker_tokens_last_from_a_day_to_a_year(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    for days in [1, 7, 30, 90, 180, 365] {
        let res = app
            .send(
                req(Method::POST, "/v1/me/maker-tokens")
                    .cookie(&maker)
                    .json(json!({ "name": format!("{days} days"), "scope": "read", "days": days })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED, "{days}");
        let t = body_json(res).await;
        let lifetime = OffsetDateTime::parse(t["expires_at"].as_str().unwrap(), &Rfc3339).unwrap()
            - OffsetDateTime::now_utc();
        assert!(
            (lifetime - time::Duration::days(days)).abs() < time::Duration::minutes(1),
            "{days}: {lifetime}"
        );
    }
    for days in [0, 366, 730] {
        let res = app
            .send(
                req(Method::POST, "/v1/me/maker-tokens")
                    .cookie(&maker)
                    .json(json!({ "name": "Too long", "scope": "read", "days": days })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{days}");
        let err = body_json(res).await;
        assert_eq!(err["error"]["field"], "days", "{err}");
        assert!(err.to_string().contains("1 to 365"), "{err}");
    }
    assert_eq!(count(&app, "select count(*) from maker_tokens").await, 6);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn owners_are_warned_a_week_and_a_day_before_a_token_expires_and_when_it_has(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (agent, laptop) = maker_token(
        &app,
        &maker,
        json!({ "name": "Laptop", "scope": "read", "days": 365 }),
    )
    .await;
    let (_, short) = maker_token(
        &app,
        &maker,
        json!({ "name": "Short", "scope": "read", "days": 7 }),
    )
    .await;
    let (_, removed) = maker_token(
        &app,
        &maker,
        json!({ "name": "Removed", "scope": "read", "days": 90 }),
    )
    .await;
    let (_, late) = maker_token(
        &app,
        &maker,
        json!({ "name": "Late", "scope": "read", "days": 90 }),
    )
    .await;
    let res = app
        .send(req(Method::DELETE, format!("/v1/me/maker-tokens/{removed}")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Nothing is near its expiry yet.
    assert_eq!(warn(&app).await, 0);
    assert!(notification_titles(&app, maker_id).await.is_empty());

    // A week before: one warning, once.
    expire_in(&app, &laptop, 300, "6 days 20 hours").await;
    // Not for a removed token, nor one made for a week or less.
    expire_in(&app, &removed, 80, "6 days").await;
    sqlx::query("update sessions set expires_at = now() + interval '20 hours' where id = $1::uuid")
        .bind(&short)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(warn(&app).await, 1);
    assert_eq!(warn(&app).await, 0, "sent once");
    assert_eq!(
        notification_titles(&app, maker_id).await,
        ["Your AI agent token \"Laptop\" expires in 7 days"]
    );
    assert_eq!(warned(&app, &laptop).await, 1);
    assert_eq!(warned(&app, &short).await, 0);
    assert_eq!(warned(&app, &removed).await, 0);
    let (category, link, body): (String, String, String) =
        sqlx::query_as("select category, link, body from notifications where user_id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(category, "agents");
    assert_eq!(link, "/account/agents");
    assert!(body.contains("Make a replacement"), "{body}");
    assert!(body.contains("never paste it into an AI chat"), "{body}");

    // The agent sees its own token's expiry, and is told to pass it on within a week.
    let (failed, text) = maker_tool(&app, &agent, "whats_new", json!({})).await;
    assert!(!failed, "{text}");
    let inbox = parsed(&text);
    assert_eq!(inbox["your_token"]["name"], "Laptop");
    assert_eq!(inbox["your_token"]["days_left"], 7);
    assert!(
        inbox["your_token"]["note"]
            .as_str()
            .unwrap()
            .contains("make a replacement on Slop under Account → AI agents"),
        "{inbox}"
    );
    // The token-expiry kind is for the agent too (it's about maker work).
    assert_eq!(inbox["items"][0]["category"], "agents", "{inbox}");
    // A person's own inbox has no `your_token`.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&maker))
            .await,
    )
    .await;
    assert!(mine.get("your_token").is_none(), "{mine}");

    // A day before: one more, once.
    expire_in(&app, &laptop, 300, "20 hours").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(warn(&app).await, 0);
    assert_eq!(warned(&app, &laptop).await, 2);
    let titles = notification_titles(&app, maker_id).await;
    assert!(
        titles.contains(&"Your AI agent token \"Laptop\" expires within a day".to_string()),
        "{titles:?}"
    );

    // A server that missed the week's warning sends only the latest one due.
    expire_in(&app, &late, 80, "10 hours").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(warned(&app, &late).await, 2);
    assert_eq!(
        count_for(
            &app,
            "select count(*) from notifications where user_id = $1 and title like '%\"Late\"%'",
            maker_id
        )
        .await,
        1
    );

    // Expired: it stops working, the owner is told once, and the page still lists it for a week.
    expire_in(&app, &laptop, 300, "-1 hour").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(warn(&app).await, 0);
    assert_eq!(warned(&app, &laptop).await, 3);
    assert!(
        notification_titles(&app, maker_id)
            .await
            .contains(&"Your AI agent token \"Laptop\" has expired".to_string())
    );
    let res = app
        .send(req(Method::GET, "/v1/me/apps").header("authorization", agent.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == laptop.as_str())
        .unwrap();
    assert_eq!(row["expired"], true);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != removed.as_str()),
        "removed tokens aren't listed"
    );
    // A week after: gone from the list, and never warned about again.
    expire_in(&app, &laptop, 300, "-8 days").await;
    assert_eq!(warn(&app).await, 0);
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != laptop.as_str()),
        "{list}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn token_expiry_emails_follow_their_own_switch(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update users set email = 'mia@example.com' where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    // The switch is listed with the others, and can be turned off.
    let prefs = body_json(
        app.send(req(Method::GET, "/v1/me/email-prefs").cookie(&maker))
            .await,
    )
    .await;
    let agents = prefs["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["category"] == "agents")
        .unwrap_or_else(|| panic!("{prefs}"));
    assert_eq!(agents["email"], true);
    assert!(
        agents["label"]
            .as_str()
            .unwrap()
            .contains("AI agent tokens")
    );
    let res = app
        .send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&maker)
                .json(json!({ "agents": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let (_, off) = maker_token(
        &app,
        &maker,
        json!({ "name": "Off", "scope": "read", "days": 90 }),
    )
    .await;
    expire_in(&app, &off, 85, "5 days").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        0,
        "switched off: in the bell only"
    );
    assert_eq!(count(&app, "select count(*) from email_outbox").await, 0);

    // On again: emailed.
    let res = app
        .send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&maker)
                .json(json!({ "agents": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let (_, on) = maker_token(
        &app,
        &maker,
        json!({ "name": "On", "scope": "read", "days": 90 }),
    )
    .await;
    expire_in(&app, &on, 85, "5 days").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        1
    );
    let (to, subject): (String, String) =
        sqlx::query_as("select to_address, subject from email_outbox")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(to, "mia@example.com");
    assert_eq!(subject, "Your AI agent token \"On\" expires in 5 days");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_replacement_leaves_the_old_token_working_and_quiet(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let settings = json!({ "name": "Laptop", "scope": "publish", "apps": ["game"], "days": 90 });
    let (old, old_id) = maker_token(&app, &maker, settings.clone()).await;
    expire_in(&app, &old_id, 85, "5 days").await;
    // The replacement: the same settings, made later, lasting longer.
    let (new, new_id) = maker_token(&app, &maker, settings).await;
    let works = |auth: String| {
        let r = req(Method::GET, "/v1/apps/game/manage").header("authorization", auth);
        async { app.send(r).await.status() }
    };
    assert_eq!(
        works(old.clone()).await,
        StatusCode::OK,
        "the old one still works"
    );
    assert_eq!(works(new.clone()).await, StatusCode::OK);
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    let old_row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == old_id.as_str())
        .unwrap();
    let new_row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == new_id.as_str())
        .unwrap();
    for field in ["name", "scope", "apps", "people"] {
        assert_eq!(old_row[field], new_row[field], "{field}");
    }
    // Replaced, so the owner isn't warned about the old one (but it's recorded as handled).
    assert_eq!(warn(&app).await, 0);
    assert_eq!(warned(&app, &old_id).await, 1);
    assert!(notification_titles(&app, maker_id).await.is_empty());
    // It keeps working until its own expiry, and removing the new one doesn't touch it.
    let res = app
        .send(req(Method::DELETE, format!("/v1/me/maker-tokens/{new_id}")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(works(old.clone()).await, StatusCode::OK);
    assert_eq!(works(new).await, StatusCode::UNAUTHORIZED);
    // Without its replacement, the day-before warning comes as usual.
    expire_in(&app, &old_id, 85, "20 hours").await;
    assert_eq!(warn(&app).await, 1);
    assert_eq!(
        notification_titles(&app, maker_id).await,
        ["Your AI agent token \"Laptop\" expires within a day"]
    );
}

// ---------------------------------------------------------------------------------------------
// Guides and the maker MCP endpoint

async fn text_of(res: Response) -> (StatusCode, String, String) {
    let status = res.status();
    let kind = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, kind, String::from_utf8(bytes.to_vec()).unwrap())
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_maker_guides_are_for_signed_in_makers_and_agents(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let (agent, _) = maker_token(&app, &maker, json!({ "name": "Claude", "scope": "read" })).await;

    for path in [
        "/v1/maker/guide",
        "/v1/maker/publishing-guide",
        "/v1/maker/publishing-guide/Saves%20and%20sandbox%20mode",
    ] {
        assert_eq!(
            app.send(req(Method::GET, path)).await.status(),
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }

    // The short guide, for a person or their agent.
    let (status, kind, guide) = text_of(
        app.send(req(Method::GET, "/v1/maker/guide").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "text/markdown; charset=utf-8");
    assert!(guide.starts_with("# Slop for makers' AI agents"), "{guide}");
    let (failed, text) = maker_tool(&app, &agent, "maker_guide", json!({})).await;
    assert!(!failed);
    // The agent's copy ends with its own token's expiry; a person's doesn't.
    let (start, end) = text.split_at(guide.len());
    assert_eq!(start, guide);
    assert!(
        end.starts_with("\n## Your token\n\nThis token (\"Claude\") expires in 30 days ("),
        "{end}"
    );
    assert!(!end.contains("make a replacement"), "{end}");
    assert!(!guide.contains("This token ("), "{guide}");

    // The publishing guide's sections, then one, by any capitalisation and with spaces around.
    let list = body_json(
        app.send(
            req(Method::GET, "/v1/maker/publishing-guide").header("authorization", agent.clone()),
        )
        .await,
    )
    .await;
    let sections: Vec<&str> = list["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    for want in [
        "Saves and sandbox mode",
        "Publishing a mod",
        "Checklist for agents",
    ] {
        assert!(sections.contains(&want), "{want}: {sections:?}");
    }
    assert!(
        list["read_one"]
            .as_str()
            .unwrap()
            .contains("/v1/maker/publishing-guide/{section}")
    );
    let (failed, text) = maker_tool(&app, &agent, "list_guide_sections", json!({})).await;
    assert!(!failed);
    assert_eq!(parsed(&text), list);

    let (status, kind, exact) = text_of(
        app.send(
            req(
                Method::GET,
                "/v1/maker/publishing-guide/Saves%20and%20sandbox%20mode",
            )
            .cookie(&maker),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "text/markdown; charset=utf-8");
    assert!(exact.starts_with("## Saves and sandbox mode") && exact.contains("SLOP_SAVE_DIR"));
    let (status, _, loose) = text_of(
        app.send(
            req(
                Method::GET,
                "/v1/maker/publishing-guide/%20%20sAVES%20AND%20sandbox%20MODE%20",
            )
            .header("authorization", agent.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(loose, exact);
    assert_eq!(
        app.send(
            req(
                Method::GET,
                "/v1/maker/publishing-guide/How%20to%20get%20rich"
            )
            .cookie(&maker)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let (failed, text) = maker_tool(
        &app,
        &agent,
        "publishing_guide",
        json!({ "section": "saves and sandbox mode" }),
    )
    .await;
    assert!(!failed);
    assert_eq!(text, exact);
    let (failed, text) = maker_tool(
        &app,
        &agent,
        "publishing_guide",
        json!({ "section": "How to get rich" }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_maker_mcp_endpoint_speaks_json_rpc_within_the_tokens_limits(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (other_id, _) = app.user("Otto Other", "US").await;
    listed_with(&app, other_id, "ottos-game", "released", "").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (limited, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Game only", "scope": "publish", "apps": ["game"] }),
    )
    .await;
    let (all, _) = maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let endpoint = "/v1/maker/mcp";

    // Anonymous: no. GET is for a server stream this server never sends.
    let res = app
        .send(
            req(Method::POST, endpoint)
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.send(req(Method::GET, endpoint)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::GET, endpoint).cookie(&maker))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        app.send(req(Method::GET, endpoint).header("authorization", read.clone()))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );

    let hello = rpc(&app, endpoint, &read, "initialize", json!({})).await;
    assert_eq!(hello["id"], 1);
    assert_eq!(hello["result"]["serverInfo"]["name"], "slop-maker");
    assert!(hello["result"]["protocolVersion"].is_string());
    assert!(
        hello["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("maker_guide")
    );
    assert_eq!(
        rpc(&app, endpoint, &read, "ping", json!({})).await["result"],
        json!({})
    );
    let unknown = rpc(&app, endpoint, &read, "resources/list", json!({})).await;
    assert_eq!(unknown["error"]["code"], -32601);
    let unknown = rpc(
        &app,
        endpoint,
        &read,
        "tools/call",
        json!({ "name": "delete_everything" }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32602);
    // A notification gets no answer; something that isn't JSON-RPC is a parse error.
    let res = app
        .send(
            req(Method::POST, endpoint)
                .header("authorization", read.clone())
                .json(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let res = app
        .send(
            req(Method::POST, endpoint)
                .header("authorization", read.clone())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "params": {} })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["code"], -32700);
    // The tool list is the same for every token: what each may do is checked when it's called.
    let tools = rpc(&app, endpoint, &read, "tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 45);
    let schema = &tools
        .iter()
        .find(|t| t["name"] == "update_store_page")
        .unwrap()["inputSchema"];
    assert_eq!(schema["required"], json!(["slug"]));
    assert!(schema["properties"]["summary"].is_object());

    // Missing arguments.
    let (failed, text) = maker_tool(&app, &read, "list_releases", json!({})).await;
    assert!(failed && text.contains("missing argument: slug"), "{text}");
    // A read token can't change anything.
    let (failed, text) = maker_tool(
        &app,
        &read,
        "update_store_page",
        json!({ "slug": "game", "summary": "Read tokens can't." }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    // Without People access, no jobs.
    let (failed, text) = maker_tool(&app, &all, "my_jobs", json!({})).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    // A token for some apps: the others aren't there, and neither are other makers' apps.
    for (token, slug) in [
        (&limited, "tower"),
        (&all, "ottos-game"),
        (&limited, "ottos-game"),
    ] {
        let (failed, text) = maker_tool(&app, token, "app_overview", json!({ "slug": slug })).await;
        assert!(failed && text.contains("not_found"), "{slug}: {text}");
    }
    let (failed, text) = maker_tool(
        &app,
        &all,
        "update_store_page",
        json!({ "slug": "ottos-game", "summary": "Not mine." }),
    )
    .await;
    assert!(failed, "{text}");
    let (failed, text) =
        maker_tool(&app, &limited, "app_overview", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    assert_eq!(
        parsed(&text)["auto_approval_note"],
        "an app's first release is always reviewed by a person",
        "{text}"
    );

    // my_apps: every app for an all-apps token, only its own for one limited to some.
    let slugs = |text: &str| -> Vec<String> {
        let mut s: Vec<String> = parsed(text)
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap().to_string())
            .collect();
        s.sort();
        s
    };
    let (failed, text) = maker_tool(&app, &all, "my_apps", json!({})).await;
    assert!(!failed);
    assert_eq!(slugs(&text), ["game", "tower"]);
    let (_, text) = maker_tool(&app, &limited, "my_apps", json!({})).await;
    assert_eq!(slugs(&text), ["game"]);
    let rest = body_json(
        app.send(req(Method::GET, "/v1/me/apps").header("authorization", limited.clone()))
            .await,
    )
    .await;
    assert_eq!(rest.as_array().unwrap().len(), 1);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/apps").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine.as_array().unwrap().len(), 2);

    // Categories and runtimes, for any token.
    let (failed, text) = maker_tool(&app, &read, "list_categories", json!({})).await;
    assert!(!failed);
    let categories = parsed(&text);
    assert!(
        categories
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["tags"].is_array()),
        "{categories}"
    );
    let (failed, text) = maker_tool(&app, &read, "list_runtimes", json!({})).await;
    assert!(!failed);
    assert!(
        text.contains("vcredist") || text.contains("Visual C++"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------------------------
// Maker MCP tools: players' reviews, bug reports and feedback

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_read_and_answer_reviews_and_bug_reports(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (report, pat_id, pat) = app_with_bug_report(&app, maker_id, "tower").await;
    app.send(
        req(Method::POST, "/v1/apps/tower/vote")
            .cookie(&pat)
            .json(json!({ "vote": "slop" })),
    )
    .await;
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/tower/review")
                .cookie(&pat)
                .json(json!({ "body": "Too grindy after level 5." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let (limited, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Tower only", "scope": "publish", "apps": ["tower"] }),
    )
    .await;

    // Reviews: read with any token, answered with a publish token.
    let (failed, text) = maker_tool(&app, &read, "list_reviews", json!({ "slug": "tower" })).await;
    assert!(!failed, "{text}");
    let reviews = parsed(&text);
    assert_eq!(reviews["reviews"][0]["body"], "Too grindy after level 5.");
    let review = reviews["reviews"][0]["id"].as_str().unwrap().to_string();
    let reply = |body: String| json!({ "slug": "tower", "id": review, "body": body });
    let (failed, text) = maker_tool(&app, &read, "reply_to_review", reply("Thanks!".into())).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) =
        maker_tool(&app, &publish, "reply_to_review", reply("x".repeat(2001))).await;
    assert!(failed && text.contains("invalid"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "reply_to_review",
        reply("Thanks! 1.2 halves the grind.".into()),
    )
    .await;
    assert!(!failed, "{text}");
    // A second reply replaces the first.
    let (failed, _) = maker_tool(
        &app,
        &limited,
        "reply_to_review",
        reply("Thanks! 1.2 halves the grind, and 1.3 adds a skip.".into()),
    )
    .await;
    assert!(!failed);
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(
        page["reviews"][0]["reply"],
        "Thanks! 1.2 halves the grind, and 1.3 adds a skip."
    );
    assert!(
        notification_titles(&app, pat_id)
            .await
            .iter()
            .any(|t| t.ends_with("replied to your review"))
    );

    // Bug reports: open ones by default, filtered by status.
    let (failed, text) =
        maker_tool(&app, &read, "list_bug_reports", json!({ "slug": "tower" })).await;
    assert!(!failed, "{text}");
    let open = parsed(&text);
    let r = &open["reports"][0];
    assert_eq!(r["id"], report.as_str());
    assert_eq!(r["log"], "panic at wave.rs:12 for pat@example.com");
    // Like the maker, the agent sees the player's display name and nothing else about them
    // (the tool tells it to leave even that out of public issues).
    assert_eq!(r["reporter"], "Pat Player");
    let fields: Vec<&String> = r.as_object().unwrap().keys().collect();
    for private in ["reporter_id", "user_id", "email", "country_code"] {
        assert!(!fields.iter().any(|f| f.as_str() == private), "{fields:?}");
    }
    assert!(!text.contains(&pat_id.to_string()));
    let (_, text) = maker_tool(
        &app,
        &read,
        "list_bug_reports",
        json!({ "slug": "tower", "status": "fixed" }),
    )
    .await;
    assert_eq!(parsed(&text)["reports"], json!([]));
    let (failed, text) = maker_tool(
        &app,
        &read,
        "list_bug_reports",
        json!({ "slug": "tower", "status": "everything" }),
    )
    .await;
    assert!(failed && text.contains("status"), "{text}");

    // Answering a report: a publish token (one for some apps, on those apps' reports).
    let fix = json!({ "id": report, "status": "fixed", "maker_note": "Fixed in 1.0.1." });
    let (failed, text) = maker_tool(&app, &read, "update_bug_report", fix.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "update_bug_report",
        json!({ "id": report, "status": "solved" }),
    )
    .await;
    assert!(failed && text.contains("status"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "update_bug_report",
        json!({ "id": report, "maker_note": "x".repeat(2001) }),
    )
    .await;
    assert!(failed && text.contains("maker_note"), "{text}");
    assert!(
        !notification_titles(&app, pat_id)
            .await
            .contains(&"Your bug report was fixed".to_string())
    );
    let (failed, text) = maker_tool(&app, &publish, "update_bug_report", fix).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["status"], "fixed");
    assert!(
        notification_titles(&app, pat_id)
            .await
            .contains(&"Your bug report was fixed".to_string())
    );
    let (_, text) = maker_tool(
        &app,
        &read,
        "list_bug_reports",
        json!({ "slug": "tower", "status": "fixed" }),
    )
    .await;
    assert_eq!(parsed(&text)["reports"][0]["maker_note"], "Fixed in 1.0.1.");

    // Feedback per release.
    let (failed, text) =
        maker_tool(&app, &read, "release_feedback", json!({ "slug": "tower" })).await;
    assert!(!failed, "{text}");
    let feedback = parsed(&text);
    assert_eq!(feedback[0]["version"], "1.0.0");
    assert_eq!(feedback[0]["bug_reports"], 1);
}

// ---------------------------------------------------------------------------------------------
// Maker MCP tools: bounties

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_find_bounties_and_claim_them_for_people_to_check(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
    listed_with(&app, maker_id, "farm", "released", "").await;
    let (forker_id, forker) = app.user("Forker", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx,
                           parent_app_id, branch_kind, branch_note)
         select $1, 'tower-fix', 'Tower (fixed)', 'Fixed', 'game', 'released', 'listed', $2, 'https://github.com/f/t',
                'MIT', id, 'fix', 'Fixes saves' from apps where slug = 'tower'",
    )
    .bind(Uuid::now_v7())
    .bind(forker_id)
    .execute(&app.db)
    .await
    .unwrap();
    approved_release(&app, "tower-fix", forker_id).await;
    listed_with(&app, forker_id, "forkers-own", "released", "").await;
    approved_release(&app, "forkers-own", forker_id).await;
    let (_, ann) = app.user("Ann Backer", "US").await;
    let mut ids = vec![];
    for (slug, title, amount) in [
        ("tower", "Fix lost saves", 3000),
        ("farm", "Add a dog", 1000),
    ] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/apps/{slug}/bounties"))
                    .cookie(&ann)
                    .json(json!({ "title": title, "body": "Please.", "currency": "usd" })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        let id = body_json(res).await["id"].as_str().unwrap().to_string();
        let res = app
            .send(
                req(Method::POST, format!("/v1/bounties/{id}/pledge"))
                    .cookie(&ann)
                    .json(json!({ "amount_minor": amount, "confirm_adult": true })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        ids.push(id);
    }
    let bounty = ids[0].clone();
    let (read, _) = maker_token(&app, &forker, json!({ "name": "Reader", "scope": "read" })).await;
    let (limited, _) = maker_token(
        &app,
        &forker,
        json!({ "name": "Fix only", "scope": "publish", "apps": ["tower-fix"] }),
    )
    .await;
    let (all, _) = maker_token(&app, &forker, json!({ "name": "All", "scope": "publish" })).await;

    // Every open bounty, biggest first, or one app's.
    let titles = |text: &str| -> Vec<String> {
        parsed(text)
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["title"].as_str().unwrap().to_string())
            .collect()
    };
    let (failed, text) = maker_tool(&app, &read, "list_bounties", json!({})).await;
    assert!(!failed, "{text}");
    assert_eq!(titles(&text), ["Fix lost saves", "Add a dog"]);
    let (_, text) = maker_tool(&app, &read, "list_bounties", json!({ "slug": "farm" })).await;
    assert_eq!(titles(&text), ["Add a dog"]);
    let (failed, text) = maker_tool(&app, &read, "get_bounty", json!({ "id": bounty })).await;
    assert!(!failed, "{text}");
    let detail = parsed(&text);
    assert_eq!(detail["title"], "Fix lost saves");
    assert_eq!(detail["pledged_minor"], 3000);
    let (failed, text) =
        maker_tool(&app, &read, "get_bounty", json!({ "id": Uuid::now_v7() })).await;
    assert!(failed && text.contains("not_found"), "{text}");

    // Claiming: a publish token, for the bounty's app or a build of it the maker owns (a token
    // for some apps, only for those).
    let claim = |slug: &str| json!({ "id": bounty, "app_slug": slug, "note": "1.0.0 writes saves safely." });
    let (failed, text) = maker_tool(&app, &read, "claim_bounty", claim("tower-fix")).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(&app, &limited, "claim_bounty", claim("forkers-own")).await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(&app, &all, "claim_bounty", claim("tower")).await;
    assert!(
        failed && text.contains("not_found"),
        "not the forker's: {text}"
    );
    let (failed, text) = maker_tool(&app, &all, "claim_bounty", claim("forkers-own")).await;
    assert!(
        failed && text.contains("app_slug"),
        "not a build of it: {text}"
    );
    let (failed, text) = maker_tool(
        &app,
        &all,
        "claim_bounty",
        json!({ "id": bounty, "app_slug": "tower-fix", "note": " " }),
    )
    .await;
    assert!(failed && text.contains("note"), "{text}");
    assert_eq!(count(&app, "select count(*) from bounty_claims").await, 0);
    let (failed, text) = maker_tool(&app, &all, "claim_bounty", claim("tower-fix")).await;
    assert!(!failed, "{text}");

    // Nothing is charged until the app's maker checks it, which an agent can't do for them.
    let (_, text) = maker_tool(&app, &read, "get_bounty", json!({ "id": bounty })).await;
    let detail = parsed(&text);
    assert_eq!(detail["status"], "claimed");
    let claim_id = detail["claims"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(count(&app, "select count(*) from payments").await, 0);
    let (makers_agent, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Maker's", "scope": "publish" }),
    )
    .await;
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/bounties/{bounty}/claims/{claim_id}/answer"),
            )
            .header("authorization", makers_agent)
            .json(json!({ "decision": "accepted" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/bounties/{bounty}/claims/{claim_id}/answer"),
            )
            .cookie(&maker)
            .json(json!({ "decision": "accepted" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);
}

// ---------------------------------------------------------------------------------------------
// Maker MCP tools: community builds and fixes offered back

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_register_community_builds_and_offer_fixes_back(db: PgPool) {
    let app = test_app(db).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let (bo_id, bo) = app.user("Bo Brancher", "US").await;
    listed_with(
        &app,
        mia_id,
        "castle",
        "released",
        "platforms = '{windows}', repo_url = 'https://github.com/mia/castle', min_age = 12",
    )
    .await;
    listed_with(&app, bo_id, "bos-own", "released", "").await;
    let (bo_read, _) = maker_token(&app, &bo, json!({ "name": "Reader", "scope": "read" })).await;
    let (bo_limited, _) = maker_token(
        &app,
        &bo,
        json!({ "name": "Own only", "scope": "publish", "apps": ["bos-own"] }),
    )
    .await;
    let (bo_all, _) = maker_token(&app, &bo, json!({ "name": "All", "scope": "publish" })).await;
    let (mia_read, _) = maker_token(&app, &mia, json!({ "name": "Reader", "scope": "read" })).await;
    let (mia_all, _) = maker_token(&app, &mia, json!({ "name": "All", "scope": "publish" })).await;

    let port = json!({
        "original": "castle", "slug": "castle-linux", "title": "Castle (Linux port)",
        "summary": "Runs natively on Linux.", "repo_url": "https://github.com/bo/castle",
        "license_spdx": "MIT", "kind": "port", "note": "Adds a Linux build and fixes paths.",
        "platforms": ["linux", "linux-deb"]
    });
    for token in [&bo_read, &bo_limited] {
        let (failed, text) =
            maker_tool(&app, token, "register_community_build", port.clone()).await;
        assert!(failed && text.contains("forbidden"), "{text}");
    }
    let mut no_note = port.clone();
    no_note.as_object_mut().unwrap().remove("note");
    let (failed, text) = maker_tool(&app, &bo_all, "register_community_build", no_note).await;
    assert!(failed && text.contains("missing argument: note"), "{text}");
    let mut unknown = port.clone();
    unknown["original"] = json!("no-such-castle");
    let (failed, text) = maker_tool(&app, &bo_all, "register_community_build", unknown).await;
    assert!(failed && text.contains("not_found"), "{text}");
    assert_eq!(
        count(
            &app,
            "select count(*) from apps where parent_app_id is not null"
        )
        .await,
        0
    );
    // `original` is the address in the path; the rest is the new app.
    let (failed, text) = maker_tool(&app, &bo_all, "register_community_build", port).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["slug"], "castle-linux");
    let (maker, parent, kind, min_age): (Uuid, String, String, i16) = sqlx::query_as(
        "select b.maker_id, p.slug, b.branch_kind, b.min_age from apps b join apps p on p.id = b.parent_app_id
         where b.slug = 'castle-linux'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (maker, parent.as_str(), kind.as_str(), min_age),
        (bo_id, "castle", "port", 12)
    );

    // Offering the fix back: publish tokens only, pull requests on the original only.
    let offer = |pr: &str| json!({ "slug": "castle-linux", "pull_request_url": pr, "message": "Fixes paths on Linux." });
    let (failed, text) = maker_tool(
        &app,
        &bo_read,
        "offer_fix_to_original",
        offer("https://github.com/mia/castle/pull/7"),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &bo_all,
        "offer_fix_to_original",
        offer("https://github.com/evil/castle/pull/7"),
    )
    .await;
    assert!(failed && text.contains("pull_request_url"), "{text}");
    let mut ids = vec![];
    for pr in [7, 8] {
        let (failed, text) = maker_tool(
            &app,
            &bo_all,
            "offer_fix_to_original",
            offer(&format!("https://github.com/mia/castle/pull/{pr}")),
        )
        .await;
        assert!(!failed, "{text}");
        ids.push(parsed(&text)["id"].as_str().unwrap().to_string());
    }
    assert_eq!(
        notification_titles(&app, mia_id)
            .await
            .iter()
            .filter(|t| t.ends_with("offered you a fix"))
            .count(),
        1,
        "told once, while unread"
    );

    // Each side lists them.
    let (failed, text) =
        maker_tool(&app, &mia_read, "list_offers", json!({ "slug": "castle" })).await;
    assert!(!failed, "{text}");
    let received = parsed(&text);
    assert_eq!(received.as_array().unwrap().len(), 2);
    assert_eq!(received[0]["direction"], "received");
    let (_, text) = maker_tool(
        &app,
        &bo_read,
        "list_offers",
        json!({ "slug": "castle-linux" }),
    )
    .await;
    assert_eq!(parsed(&text)[0]["direction"], "sent");
    let (failed, text) =
        maker_tool(&app, &bo_read, "list_offers", json!({ "slug": "castle" })).await;
    assert!(failed && text.contains("not_found"), "not Bo's app: {text}");

    // Answering: the original's maker, with a publish token.
    let answer = json!({ "slug": "castle", "id": ids[0], "decision": "accepted", "message": "Merged, thanks!" });
    let (failed, text) = maker_tool(&app, &mia_read, "answer_offer", answer.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let mut on_branch = answer.clone();
    on_branch["slug"] = json!("castle-linux");
    let (failed, _) = maker_tool(&app, &bo_all, "answer_offer", on_branch).await;
    assert!(failed, "Bo can't accept his own offer");
    let (failed, text) = maker_tool(
        &app,
        &mia_all,
        "answer_offer",
        json!({ "slug": "castle", "id": ids[0], "decision": "maybe" }),
    )
    .await;
    assert!(failed && text.contains("decision"), "{text}");
    let (failed, text) = maker_tool(&app, &mia_all, "answer_offer", answer).await;
    assert!(!failed, "{text}");
    assert!(
        notification_titles(&app, bo_id)
            .await
            .iter()
            .any(|t| t.contains("accepted"))
    );

    // Withdrawing: the build's maker, with a publish token, an offer still open.
    let withdraw = |slug: &str, id: &str| json!({ "slug": slug, "id": id });
    let (failed, text) = maker_tool(
        &app,
        &bo_read,
        "withdraw_offer",
        withdraw("castle-linux", &ids[1]),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    for (token, slug) in [
        (&mia_all, "castle-linux"),
        (&mia_all, "castle"),
        (&bo_all, "bos-own"),
    ] {
        let (failed, text) =
            maker_tool(&app, token, "withdraw_offer", withdraw(slug, &ids[1])).await;
        assert!(failed && text.contains("not_found"), "{slug}: {text}");
    }
    let (failed, text) = maker_tool(
        &app,
        &bo_all,
        "withdraw_offer",
        withdraw("castle-linux", &ids[0]),
    )
    .await;
    assert!(
        failed && text.contains("not_found"),
        "already answered: {text}"
    );
    let (failed, text) = maker_tool(
        &app,
        &bo_all,
        "withdraw_offer",
        withdraw("castle-linux", &ids[1]),
    )
    .await;
    assert!(!failed, "{text}");
    let (_, text) = maker_tool(
        &app,
        &bo_read,
        "list_offers",
        json!({ "slug": "castle-linux" }),
    )
    .await;
    let sent = parsed(&text);
    let mut statuses: Vec<&str> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["status"].as_str().unwrap())
        .collect();
    statuses.sort();
    assert_eq!(statuses, ["accepted", "withdrawn"]);
    let (failed, _) = maker_tool(
        &app,
        &bo_all,
        "withdraw_offer",
        withdraw("castle-linux", &ids[1]),
    )
    .await;
    assert!(failed, "twice");
}

// ---------------------------------------------------------------------------------------------
// Maker MCP tools: store pages, releases and checks

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_make_and_edit_store_pages(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (limited, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Game only", "scope": "publish", "apps": ["game"] }),
    )
    .await;
    let (all, _) = maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;

    // New pages: an all-apps publish token only.
    for token in [&read, &limited] {
        let (failed, text) = maker_tool(&app, token, "create_store_page", new_app("fresh")).await;
        assert!(failed && text.contains("forbidden"), "{text}");
    }
    let mut no_licence = new_app("fresh");
    no_licence.as_object_mut().unwrap().remove("license_spdx");
    let (failed, text) = maker_tool(&app, &all, "create_store_page", no_licence).await;
    assert!(
        failed && text.contains("missing argument: license_spdx"),
        "{text}"
    );
    assert_eq!(
        count(&app, "select count(*) from apps where slug = 'fresh'").await,
        0
    );
    let (failed, text) = maker_tool(&app, &all, "create_store_page", new_app("fresh")).await;
    assert!(!failed, "{text}");
    let page = parsed(&text);
    assert_eq!(page["slug"], "fresh");
    assert_eq!(page["status"], "draft");
    let owner: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'fresh'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, maker_id);

    // Editing: its own apps, with a publish token.
    let (failed, text) = maker_tool(
        &app,
        &limited,
        "update_store_page",
        json!({ "slug": "game", "summary": "A tiny roguelike about going down." }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(
        parsed(&text)["summary"],
        "A tiny roguelike about going down."
    );
    let summary: String = sqlx::query_scalar("select summary from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(summary, "A tiny roguelike about going down.");
    let (failed, text) = maker_tool(
        &app,
        &limited,
        "update_store_page",
        json!({ "slug": "tower", "summary": "Not this one." }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &all,
        "update_store_page",
        json!({ "slug": "game", "min_age": 13 }),
    )
    .await;
    assert!(failed && text.contains("min_age"), "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_submit_releases_with_notes_and_read_the_fixes(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let release = |version: &str| {
        let mut b = release_body(&app, version, "game-windows.zip");
        b["slug"] = json!("game");
        b["notes"] = json!("First release.");
        b
    };

    // No release yet: nothing to fix, and what to do next.
    let (failed, text) = maker_tool(&app, &read, "get_fixes", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    let none = parsed(&text);
    assert!(none["release"].is_null());
    assert!(none["next"].as_str().unwrap().contains("submit_release"));

    let (failed, text) = maker_tool(&app, &read, "submit_release", release("1.0.0")).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let mut no_files = release("1.0.0");
    no_files.as_object_mut().unwrap().remove("files");
    let (failed, text) = maker_tool(&app, &publish, "submit_release", no_files).await;
    assert!(failed && text.contains("missing argument: files"), "{text}");
    let (failed, text) = maker_tool(&app, &publish, "submit_release", release("1.0.0")).await;
    assert!(!failed, "{text}");
    let submitted = parsed(&text);
    assert_eq!(submitted["status"], "fetching");
    assert_eq!(submitted["notes"], "First release.");
    let id = submitted["id"].as_str().unwrap().to_string();
    assert!(releases::fetch_next(&app.state).await.unwrap());

    // Notes: a publish token sets them, up to 5,000 characters.
    let notes = |n: String| json!({ "slug": "game", "id": id, "notes": n });
    let (failed, text) =
        maker_tool(&app, &read, "set_release_notes", notes("Better.".into())).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) =
        maker_tool(&app, &publish, "set_release_notes", notes("x".repeat(5001))).await;
    assert!(failed && text.contains("notes"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "set_release_notes",
        notes("Adds a **jump**.".into()),
    )
    .await;
    assert!(!failed, "{text}");
    let saved: Option<String> =
        sqlx::query_scalar("select notes from releases where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(saved.as_deref(), Some("Adds a **jump**."));
    let (failed, text) = maker_tool(&app, &read, "list_releases", json!({ "slug": "game" })).await;
    assert!(!failed && text.contains("Adds a **jump**."), "{text}");

    // A rejected release with a secret Slop Build found: the reason first, then the secret.
    sqlx::query(
        "update releases set status = 'rejected', status_reason = 'Ships a debug console.',
                source_checks = '{\"secrets\": [{\"file\": \"src/net.rs\", \"line\": 3, \"rule\": \"aws-access-key\"}]}'
         where id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let (failed, text) = maker_tool(&app, &read, "get_fixes", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    let fixes = parsed(&text);
    assert_eq!(fixes["release"]["version"], "1.0.0");
    let kinds: Vec<&str> = fixes["fixes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["release:rejected", "secret"]);
    assert!(
        fixes["fixes"][0]["problem"]
            .as_str()
            .unwrap()
            .contains("Ships a debug console.")
    );
    assert!(
        fixes["fixes"][1]["problem"]
            .as_str()
            .unwrap()
            .contains("src/net.rs:3")
    );
    assert_eq!(fixes["fixes"][1]["severity"], "must");

    // Past the passkey deadline, only a token proven with a passkey submits.
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, '\\x01', '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update users set passkey_due_at = now() - interval '1 day' where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (failed, text) = maker_tool(&app, &publish, "submit_release", release("1.0.1")).await;
    assert!(failed && text.contains("passkey_required"), "{text}");
    sqlx::query("update sessions set passkey_at = now() where user_id = $1 and kind = 'web'")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (proven, _) = maker_token(
        &app,
        &maker,
        json!({ "name": "Proven", "scope": "publish" }),
    )
    .await;
    let (failed, text) = maker_tool(&app, &proven, "submit_release", release("1.0.1")).await;
    assert!(!failed, "{text}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_use_github_releases_and_slop_build(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = github_app(&app).await;
    sqlx::query("update apps set platforms = '{windows}'")
        .execute(&app.db)
        .await
        .unwrap();
    let (other_id, _) = app.user("Otto Other", "US").await;
    listed_with(&app, other_id, "ottos", "released", "").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;

    let (failed, text) =
        maker_tool(&app, &read, "slop_build_setup", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    let setup = parsed(&text);
    assert_eq!(setup["supported"], true);
    assert_eq!(setup["project_type"], "rust");
    assert!(setup["workflow"].as_str().unwrap().contains("app: game"));
    let (failed, text) =
        maker_tool(&app, &read, "slop_build_setup", json!({ "slug": "ottos" })).await;
    assert!(failed && text.contains("not_found"), "{text}");

    let (failed, text) =
        maker_tool(&app, &read, "github_releases", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    let picks = parsed(&text);
    let tags: Vec<&str> = picks
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["v1.1.0", "v1.0.0"]);
    let (failed, text) =
        maker_tool(&app, &read, "github_releases", json!({ "slug": "ottos" })).await;
    assert!(failed && text.contains("not_found"), "{text}");

    let pick = json!({
        "slug": "game", "tag": "v1.0.0",
        "files": [{
            "platform": "windows", "kind": "archive", "exec": "game.exe",
            "url": "https://github.com/octo/game/releases/download/v1.0.0/game-1.0.0-windows.zip"
        }]
    });
    let (failed, text) = maker_tool(&app, &read, "submit_github_release", pick.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "submit_github_release",
        json!({ "slug": "game", "tag": "" }),
    )
    .await;
    assert!(failed, "{text}");
    let (failed, text) = maker_tool(&app, &publish, "submit_github_release", pick).await;
    assert!(!failed, "{text}");
    let release = parsed(&text);
    assert_eq!(release["version"], "1.0.0");
    assert_eq!(release["commit_sha"], "1".repeat(40));
    let (_, text) = maker_tool(&app, &read, "github_releases", json!({ "slug": "game" })).await;
    assert_eq!(parsed(&text)[1]["submitted"], true);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn check_repository_checks_that_slop_can_still_read_the_repository(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (other_id, _) = app.user("Otto Other", "US").await;
    listed_with(&app, other_id, "ottos", "released", "").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let (failed, text) =
        maker_tool(&app, &read, "check_repository", json!({ "slug": "game" })).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "check_repository",
        json!({ "slug": "ottos" }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    // Readable: not frozen (null).
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "check_repository",
        json!({ "slug": "game" }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text), Value::Null);
    let frozen: bool =
        sqlx::query_scalar("select frozen_at is not null from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!frozen);
}

/// check_repository checks Slop can read the repository, freezing or unfreezing the app, and
/// says so; it doesn't check the code (bug b111: it used to say it ran the source check).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn check_repository_freezes_and_unfreezes_by_repository_access(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let tools = rpc(&app, "/v1/maker/mcp", &publish, "tools/list", json!({})).await;
    let described = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "check_repository")
        .unwrap()["description"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(described.contains("doesn't check the code"), "{described}");
    let frozen = || async {
        sqlx::query_scalar::<_, bool>("select frozen_at is not null from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    // The repository can't be read: frozen, with the reason.
    sqlx::query("update apps set repo_url = $1 where slug = 'game'")
        .bind(format!("{}octo/gone", app.fake_base))
        .execute(&app.db)
        .await
        .unwrap();
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "check_repository",
        json!({ "slug": "game" }),
    )
    .await;
    assert!(!failed, "{text}");
    let freeze = parsed(&text);
    assert!(
        freeze["reason"].as_str().unwrap().contains("can't be read"),
        "{text}"
    );
    assert_eq!(freeze["by_staff"], false);
    assert!(frozen().await);

    // Readable again: the freeze is lifted (null).
    sqlx::query("update apps set repo_url = $1 where slug = 'game'")
        .bind(format!("{}octo/game", app.fake_base))
        .execute(&app.db)
        .await
        .unwrap();
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "check_repository",
        json!({ "slug": "game" }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text), Value::Null);
    assert!(!frozen().await);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_host_and_watch_a_browser_demo(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-demos-agents-{}", rand::random::<u64>()));
    let demo_dir = dir.clone();
    let app = test_app_with(db, move |c| {
        c.demos.play_url = Some("http://play.localhost:8080".parse().unwrap());
        c.demos.dir = demo_dir;
    })
    .await;
    let (_, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed', stage_entered_at = now() where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let (other_id, _) = app.user("Otto Other", "US").await;
    listed_with(&app, other_id, "ottos", "released", "").await;
    let (read, _) = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;
    let (publish, _) =
        maker_token(&app, &maker, json!({ "name": "All", "scope": "publish" })).await;
    let url = format!("{}octo/game/releases/download/v1/web.zip", app.fake_base);

    let (failed, text) = maker_tool(&app, &read, "browser_demo", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    assert!(parsed(&text)["demo"].is_null(), "{text}");
    let (failed, text) = maker_tool(&app, &read, "browser_demo", json!({ "slug": "ottos" })).await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &read,
        "host_browser_demo",
        json!({ "slug": "game", "url": url }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "host_browser_demo",
        json!({ "slug": "ottos", "url": url }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &publish,
        "host_browser_demo",
        json!({ "slug": "game", "url": url }),
    )
    .await;
    assert!(!failed, "{text}");
    let mut status = Value::Null;
    for _ in 0..100 {
        let (_, text) = maker_tool(&app, &read, "browser_demo", json!({ "slug": "game" })).await;
        status = parsed(&text);
        if status["demo"]["status"] != "fetching" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    assert_eq!(status["demo"]["status"], "live", "{status}");
    assert_eq!(status["play_url"], "http://play.localhost:8080/play/game/");
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------------------------
// Maker MCP tools: People jobs

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_with_people_access_handle_jobs_but_never_agree_or_pay(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 1000;
    })
    .await;
    let (_, vera) = live_creative(&app, "vera", "US", true).await;
    let (client_id, client) = app.user("Tavern Dev", "US").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    listed_with(&app, client_id, "tiny-tavern", "released", "").await;
    let hire = || {
        req(Method::POST, "/v1/people/vera/hire")
            .cookie(&client)
            .json(json!({ "app_slug": "tiny-tavern", "skill": "pixel-art",
                          "request_md": "A knight, 4 directions, 32×32.", "confirm_adult": true,
                          "exact_wording_ack": true, "inquiry": true }))
    };
    let res = app.send(hire()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let job = body_json(res).await["id"].as_str().unwrap().to_string();

    let people =
        |scope: &str| json!({ "name": format!("People {scope}"), "scope": scope, "people": true });
    let (vera_agent, _) = maker_token(&app, &vera, people("publish")).await;
    let (vera_reader, _) = maker_token(&app, &vera, people("read")).await;
    let (vera_plain, _) = maker_token(
        &app,
        &vera,
        json!({ "name": "No People", "scope": "publish" }),
    )
    .await;
    let (client_agent, _) = maker_token(&app, &client, people("publish")).await;
    let (stranger_agent, _) = maker_token(&app, &stranger, people("publish")).await;
    let id = json!({ "id": job });

    // Reading: any token with People access, on a job they're a side of.
    let (failed, text) = maker_tool(&app, &vera_reader, "my_jobs", json!({})).await;
    assert!(!failed, "{text}");
    let mine = parsed(&text);
    assert_eq!(mine["as_creative"][0]["id"], job.as_str());
    assert_eq!(mine["as_creative"][0]["conversation_requested"], true);
    let (_, text) = maker_tool(&app, &client_agent, "my_jobs", json!({})).await;
    assert_eq!(parsed(&text)["as_client"][0]["id"], job.as_str());
    let (failed, text) = maker_tool(&app, &vera_reader, "get_job", id.clone()).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["status"], "requested");
    let (failed, text) = maker_tool(&app, &stranger_agent, "get_job", id.clone()).await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(&app, &vera_plain, "get_job", id.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");

    // The conversation: the creative opens and closes it, both sides acknowledge the recording.
    let (failed, text) = maker_tool(&app, &client_agent, "open_job_conversation", id.clone()).await;
    assert!(
        failed && text.contains("conflict"),
        "the creative opens it: {text}"
    );
    let (failed, text) = maker_tool(&app, &vera_reader, "open_job_conversation", id.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "open_job_conversation", id.clone()).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["conversation"]["open"], true);
    let message = |ack: bool| json!({ "id": job, "body": "Which engine? (Vera's AI assistant)", "recording_ack": ack });
    let (failed, text) = maker_tool(&app, &vera_reader, "send_job_message", message(true)).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "send_job_message", message(false)).await;
    assert!(failed && text.contains("recording_ack"), "{text}");
    let (failed, text) = maker_tool(&app, &stranger_agent, "send_job_message", message(true)).await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "send_job_message", message(true)).await;
    assert!(!failed, "{text}");
    let (failed, text) = maker_tool(&app, &vera_reader, "read_job_conversation", id.clone()).await;
    assert!(!failed, "{text}");
    assert_eq!(
        parsed(&text)["messages"][0]["body"],
        "Which engine? (Vera's AI assistant)"
    );
    let (failed, text) =
        maker_tool(&app, &stranger_agent, "read_job_conversation", id.clone()).await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) =
        maker_tool(&app, &client_agent, "close_job_conversation", id.clone()).await;
    assert!(failed, "the creative closes it: {text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "close_job_conversation", id.clone()).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["conversation"]["open"], false);
    // The client's agent asks for another; only the client can ask.
    let (failed, text) = maker_tool(
        &app,
        &vera_agent,
        "ask_for_job_conversation",
        json!({ "id": job, "note": "Talk?" }),
    )
    .await;
    assert!(failed && text.contains("conflict"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &client_agent,
        "ask_for_job_conversation",
        json!({ "id": job, "note": "One more question about the colours." }),
    )
    .await;
    assert!(!failed, "{text}");
    assert!(parsed(&text)["conversation"]["requested_at"].is_string());

    // The quote: only the creative's agent with publish and People access; prices are whole cents.
    let due = (OffsetDateTime::now_utc().date() + time::Duration::days(14)).to_string();
    let quote = |price: Value| {
        json!({ "id": job, "title": "Knight sprite sheet", "scope_md": "4 directions × 8 frames.",
                "price_minor": price, "currency": "usd", "payment": "upfront", "due_date": due,
                "rights": "transfer", "open_source": false, "credit": true, "ai_use": "none" })
    };
    for token in [&vera_reader, &vera_plain] {
        let (failed, text) = maker_tool(&app, token, "send_quote", quote(json!(12000))).await;
        assert!(failed && text.contains("forbidden"), "{text}");
    }
    let (failed, text) = maker_tool(&app, &client_agent, "send_quote", quote(json!(12000))).await;
    assert!(failed, "the client doesn't quote: {text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "send_quote", quote(json!("120.00"))).await;
    assert!(failed, "{text}");
    let mut short = quote(json!(12000));
    short.as_object_mut().unwrap().remove("due_date");
    let (failed, text) = maker_tool(&app, &vera_agent, "send_quote", short).await;
    assert!(
        failed && text.contains("missing argument: due_date"),
        "{text}"
    );
    let (failed, text) = maker_tool(&app, &vera_agent, "send_quote", quote(json!(12000))).await;
    assert!(!failed, "{text}");
    let offered = parsed(&text);
    assert_eq!(offered["status"], "offered");
    assert_eq!(offered["offer"]["price_minor"], 12000, "{offered}");

    // Agreeing, paying and delivering stay with the people.
    for (token, path, body) in [
        (
            &client_agent,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true }),
        ),
        (&client_agent, "pay", json!({})),
        (&client_agent, "withdraw", json!({})),
        (&client_agent, "cancel", json!({})),
        (&vera_agent, "deliver", json!({ "note": "Done." })),
        (
            &vera_agent,
            "sign-off/send",
            json!({ "links": ["https://drive.example/x.png"] }),
        ),
        (&client_agent, "sign-off", json!({})),
        (&client_agent, "accept-delivery", json!({})),
    ] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/jobs/{job}/{path}"))
                    .header("authorization", token.clone())
                    .json(body),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{path}");
    }
    let status: String = sqlx::query_scalar("select status from jobs where id = $1::uuid")
        .bind(&job)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "offered");

    // The client accepts and pays, signed in themselves.
    let res = app
        .send(
            req(Method::POST, format!("/v1/jobs/{job}/accept"))
                .cookie(&client)
                .json(json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let res = app
        .send(
            req(Method::POST, format!("/v1/jobs/{job}/pay"))
                .cookie(&client)
                .json(json!({})),
        )
        .await;
    assert_eq!(body_json(res).await["job"]["status"], "in_progress");

    // Changes: the client's agent asks, the creative's agent offers terms, and only the client
    // agrees to them.
    let (failed, text) = maker_tool(
        &app,
        &vera_agent,
        "request_change",
        json!({ "id": job, "note": "Could it run?" }),
    )
    .await;
    assert!(
        failed && text.contains("conflict"),
        "only the client asks: {text}"
    );
    let (failed, text) = maker_tool(
        &app,
        &client_agent,
        "request_change",
        json!({ "id": job, "note": "Could the knight also run?" }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["changes"][0]["status"], "requested");
    let new_due = (OffsetDateTime::now_utc().date() + time::Duration::days(30)).to_string();
    let change = json!({ "id": job, "title": "Add a running animation", "scope_md": "8 frames, 4 directions.",
                         "extra_price_minor": 4000, "new_due_date": new_due });
    let (failed, text) = maker_tool(&app, &client_agent, "propose_change", change.clone()).await;
    assert!(failed, "only the creative offers terms: {text}");
    let (failed, text) = maker_tool(&app, &vera_reader, "propose_change", change.clone()).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(&app, &vera_agent, "propose_change", change).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["changes"][0]["status"], "offered");
    let res = app
        .send(
            req(Method::POST, format!("/v1/jobs/{job}/change/accept"))
                .header("authorization", client_agent.clone())
                .json(json!({ "number": 1, "exact_terms_ack": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // Declined: the work stays as agreed.
    let (failed, text) = maker_tool(
        &app,
        &stranger_agent,
        "decline_change",
        json!({ "id": job, "note": "No." }),
    )
    .await;
    assert!(failed && text.contains("not_found"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &client_agent,
        "decline_change",
        json!({ "id": job, "note": "Not this month." }),
    )
    .await;
    assert!(!failed, "{text}");
    let after = parsed(&text);
    assert_eq!(after["changes"][0]["status"], "declined");
    assert_eq!(after["status"], "in_progress");
    assert_eq!(after["due_date"], due.as_str());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_decline_job_requests_with_or_without_a_note(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (_, vera) = live_creative(&app, "vera", "US", true).await;
    let (client_id, client) = app.user("Tavern Dev", "US").await;
    listed_with(&app, client_id, "tiny-tavern", "released", "").await;
    let mut jobs = vec![];
    for _ in 0..2 {
        let res = app
            .send(
                req(Method::POST, "/v1/people/vera/hire")
                    .cookie(&client)
                    .json(json!({ "app_slug": "tiny-tavern", "skill": "pixel-art",
                                  "request_md": "A knight.", "confirm_adult": true,
                                  "exact_wording_ack": true, "inquiry": false })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        jobs.push(body_json(res).await["id"].as_str().unwrap().to_string());
    }
    let (reader, _) = maker_token(
        &app,
        &vera,
        json!({ "name": "Reader", "scope": "read", "people": true }),
    )
    .await;
    let (agent, _) = maker_token(
        &app,
        &vera,
        json!({ "name": "Agent", "scope": "publish", "people": true }),
    )
    .await;
    let (failed, text) = maker_tool(
        &app,
        &reader,
        "decline_job",
        json!({ "id": jobs[0], "note": "Booked up." }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    let (failed, text) = maker_tool(
        &app,
        &agent,
        "decline_job",
        json!({ "id": jobs[0], "note": "Booked up until spring." }),
    )
    .await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["status"], "declined");
    let (failed, text) = maker_tool(&app, &agent, "decline_job", json!({ "id": jobs[0] })).await;
    assert!(
        failed && text.contains("conflict"),
        "already declined: {text}"
    );
    // The note is optional in the tools' schemas, so leaving it out must work too.
    let (client_agent, _) = maker_token(
        &app,
        &client,
        json!({ "name": "Client's", "scope": "publish", "people": true }),
    )
    .await;
    let (failed, text) = maker_tool(
        &app,
        &client_agent,
        "ask_for_job_conversation",
        json!({ "id": jobs[1] }),
    )
    .await;
    assert!(!failed, "{text}");
    assert!(parsed(&text)["conversation"]["requested_at"].is_string());
    let (failed, text) = maker_tool(&app, &agent, "decline_job", json!({ "id": jobs[1] })).await;
    assert!(!failed, "{text}");
    assert_eq!(parsed(&text)["status"], "declined");
}
