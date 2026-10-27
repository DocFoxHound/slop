// SPDX-License-Identifier: AGPL-3.0-or-later
//! AI tools connecting by being approved in the browser (agent_oauth.rs): discovery, registration,
//! approval, tokens, refreshing, ending a connection, and the short MCP address.

use crate::*;

const SITE: &str = "http://localhost:8080";
const LOOPBACK: &str = "http://127.0.0.1:45678/callback";

/// An app whose rate limits don't get in the way (every request comes from one address).
async fn oauth_app(db: PgPool) -> TestApp {
    test_app_with(db, |c| {
        c.rate_limits.write_per_minute = 10_000;
        c.rate_limits.read_per_minute = 10_000;
        c.rate_limits.auth_per_minute = 10_000;
    })
    .await
}

/// A form POST, as AI tools send to the token and revocation endpoints.
async fn post_form(app: &TestApp, path: &str, pairs: &[(&str, &str)]) -> Response {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    app.router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(header::HOST, "localhost:8080")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Registers a tool with these return addresses; returns its client_id.
async fn register(app: &TestApp, redirect_uris: &[&str]) -> String {
    let res = app
        .send(req(Method::POST, "/v1/oauth/register").json(json!({
            "client_name": "Test Tool",
            "redirect_uris": redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })))
        .await;
    let status = res.status();
    let body = body_json(res).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["token_endpoint_auth_method"], "none");
    body["client_id"].as_str().unwrap().to_string()
}

fn verifier() -> String {
    format!("verifier-{}", slop_api::tokens::new_token())
}

/// The authorization request a tool makes, with `changes` replacing (or, with an empty value,
/// leaving out) parameters.
fn authorize_url(client_id: &str, verifier: &str, changes: &[(&str, &str)]) -> String {
    let challenge = pkce_challenge(verifier);
    let mut params: Vec<(&str, String)> = vec![
        ("response_type", "code".into()),
        ("client_id", client_id.into()),
        ("redirect_uri", LOOPBACK.into()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256".into()),
        ("state", "tool-state-123".into()),
        ("resource", format!("{SITE}/mcp")),
        ("scope", "read publish".into()),
    ];
    for (k, v) in changes {
        params.retain(|(p, _)| p != k);
        if !v.is_empty() {
            params.push((k, v.to_string()));
        }
    }
    let q = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())))
        .finish();
    format!("/v1/oauth/authorize?{q}")
}

/// Starts an approval; returns the request's ID from where the browser is sent.
async fn start(app: &TestApp, client_id: &str, verifier: &str) -> String {
    let res = app
        .send(req(Method::GET, authorize_url(client_id, verifier, &[])))
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let to = location(&res);
    to.strip_prefix(&format!("{SITE}/connect-ai/"))
        .unwrap_or_else(|| panic!("{to}"))
        .to_string()
}

async fn approve(app: &TestApp, cookie: &str, id: &str, body: Value) -> Response {
    app.send(
        req(Method::POST, format!("/v1/oauth/requests/{id}/approve"))
            .cookie(cookie)
            .json(body),
    )
    .await
}

fn choices(scope: &str) -> Value {
    json!({ "name": "Claude Code", "scope": scope, "apps": [], "people": false, "days": 30 })
}

/// The code from where approving sends the browser, checking `state` and `iss` came back.
fn code_of(redirect_url: &str) -> String {
    assert!(redirect_url.starts_with(LOOPBACK), "{redirect_url}");
    assert_eq!(query_param(redirect_url, "state"), "tool-state-123");
    assert_eq!(query_param(redirect_url, "iss"), SITE);
    query_param(redirect_url, "code")
}

async fn exchange(app: &TestApp, client_id: &str, code: &str, verifier: &str) -> Response {
    post_form(
        app,
        "/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", client_id),
            ("redirect_uri", LOOPBACK),
            ("resource", &format!("{SITE}/mcp")),
        ],
    )
    .await
}

async fn refresh(app: &TestApp, client_id: &str, refresh_token: &str) -> Response {
    post_form(
        app,
        "/v1/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("resource", &format!("{SITE}/mcp")),
        ],
    )
    .await
}

/// Registers, approves with `body` and swaps the code: (client_id, the token response).
async fn connect(app: &TestApp, cookie: &str, body: Value) -> (String, Value) {
    let client_id = register(app, &[LOOPBACK]).await;
    let v = verifier();
    let id = start(app, &client_id, &v).await;
    let res = approve(app, cookie, &id, body).await;
    let status = res.status();
    let answer = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let res = exchange(app, &client_id, &code, &v).await;
    let status = res.status();
    let tokens = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    (client_id, tokens)
}

fn bearer(tokens: &Value) -> String {
    format!("Bearer {}", tokens["access_token"].as_str().unwrap())
}

/// One MCP request at `path`.
async fn mcp(
    app: &TestApp,
    path: &str,
    auth: Option<&str>,
    method: &str,
    params: Value,
) -> Response {
    let mut r = req(Method::POST, path)
        .json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
    if let Some(a) = auth {
        r = r.header("authorization", a.to_string());
    }
    app.send(r).await
}

/// Calls a maker tool at `/mcp`: (failed, text).
async fn tool(app: &TestApp, auth: &str, name: &str, args: Value) -> (bool, String) {
    let res = mcp(
        app,
        "/mcp",
        Some(auth),
        "tools/call",
        json!({ "name": name, "arguments": args }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let reply = body_json(res).await;
    let result = &reply["result"];
    (
        result["isError"].as_bool().unwrap(),
        result["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

async fn make_app(app: &TestApp, cookie: &str, slug: &str) {
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(cookie)
                .json(new_app(slug)),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

fn www_authenticate(res: &Response) -> String {
    res.headers()
        .get(header::WWW_AUTHENTICATE)
        .expect("WWW-Authenticate")
        .to_str()
        .unwrap()
        .to_string()
}

// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_tool_connects_by_approval_and_the_connection_ends_on_the_ai_agents_page(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    make_app(&app, &maker, "alpha").await;

    // 1. The MCP address says where to find out how to connect.
    let res = mcp(&app, "/mcp", None, "initialize", json!({})).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let challenge = www_authenticate(&res);
    assert_eq!(
        challenge,
        format!(
            "Bearer resource_metadata=\"{SITE}/.well-known/oauth-protected-resource/mcp\", scope=\"read publish\""
        )
    );
    let prm = body_json(
        app.send(req(
            Method::GET,
            "/.well-known/oauth-protected-resource/mcp",
        ))
        .await,
    )
    .await;
    assert_eq!(prm["resource"], format!("{SITE}/mcp"));
    assert_eq!(prm["authorization_servers"], json!([SITE]));
    let res = app
        .send(req(Method::GET, "/.well-known/oauth-authorization-server"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["access-control-allow-origin"],
        "*",
        "browser-based tools can read it"
    );
    let meta = body_json(res).await;
    assert_eq!(meta["issuer"], SITE);
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        meta["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    assert_eq!(meta["client_id_metadata_document_supported"], true);
    assert_eq!(meta["authorization_response_iss_parameter_supported"], true);
    assert_eq!(meta["token_endpoint"], format!("{SITE}/v1/oauth/token"));

    // 2. Registration, and the browser is sent to the approval page.
    let client_id = register(&app, &["http://127.0.0.1/callback"]).await;
    let v = verifier();
    // Any port on this computer.
    let id = start(&app, &client_id, &v).await;

    // 3. The page needs someone signed in, and says who's asking and where the browser goes.
    let res = app
        .send(req(Method::GET, format!("/v1/oauth/requests/{id}")))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let info = body_json(
        app.send(req(Method::GET, format!("/v1/oauth/requests/{id}")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(info["client_name"], "Test Tool");
    assert_eq!(info["client_kind"], "registered");
    assert_eq!(info["redirect_kind"], "loopback");
    assert_eq!(info["unfamiliar"], false);
    assert_eq!(info["requested_scopes"], json!(["read", "publish"]));

    // 4. Approved to look only, for all apps.
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    // Approved once only.
    assert_eq!(
        approve(&app, &maker, &id, choices("read")).await.status(),
        StatusCode::NOT_FOUND
    );

    // 5. Tokens.
    let res = exchange(&app, &client_id, &code, &v).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["cache-control"], "no-store");
    let tokens = body_json(res).await;
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["scope"], "read");
    assert_eq!(tokens["expires_in"].as_i64().unwrap() / 60, 59);
    assert!(
        tokens["access_token"]
            .as_str()
            .unwrap()
            .starts_with("slopmk_")
    );
    assert!(
        tokens["refresh_token"]
            .as_str()
            .unwrap()
            .starts_with("slopmr_")
    );

    // 6. MCP works at both addresses, within what was chosen.
    let auth = bearer(&tokens);
    for path in ["/mcp", "/v1/maker/mcp"] {
        let res = mcp(&app, path, Some(&auth), "tools/list", json!({})).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert!(
            body_json(res).await["result"]["tools"]
                .as_array()
                .unwrap()
                .len()
                > 10
        );
    }
    let (failed, text) = tool(&app, &auth, "app_overview", json!({ "slug": "alpha" })).await;
    assert!(!failed, "{text}");
    let (failed, text) = tool(
        &app,
        &auth,
        "update_store_page",
        json!({ "slug": "alpha", "summary": "Looking only." }),
    )
    .await;
    assert!(failed && text.contains("forbidden"), "{text}");
    // Nothing outside maker work, as for any maker token.
    let res = app
        .send(req(Method::GET, "/v1/me/sessions").header("authorization", auth.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 7. Refreshing gives new tokens and spends the old refresh token.
    let old_refresh = tokens["refresh_token"].as_str().unwrap().to_string();
    let res = refresh(&app, &client_id, &old_refresh).await;
    assert_eq!(res.status(), StatusCode::OK);
    let renewed = body_json(res).await;
    assert_ne!(renewed["refresh_token"], tokens["refresh_token"]);
    assert_ne!(renewed["access_token"], tokens["access_token"]);
    let auth = bearer(&renewed);
    let (failed, _) = tool(&app, &auth, "my_apps", json!({})).await;
    assert!(!failed);

    // 8. Listed on Account → AI agents (and the devices page) as approved in the browser.
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    let listed = &list.as_array().unwrap()[0];
    assert_eq!(listed["name"], "Claude Code");
    assert_eq!(listed["scope"], "read");
    assert_eq!(listed["connected"]["client"], "Test Tool");
    assert_eq!(listed["connected"]["redirect_host"], Value::Null);
    let sessions = body_json(
        app.send(req(Method::GET, "/v1/me/sessions").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "agent" && s["user_agent"] == "AI agent: Claude Code"),
        "{sessions}"
    );

    // 9. Removing it there ends it at once: the access token and the refresh token stop.
    let grant = listed["id"].as_str().unwrap();
    let res = app
        .send(req(Method::DELETE, format!("/v1/me/maker-tokens/{grant}")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = mcp(&app, "/mcp", Some(&auth), "tools/list", json!({})).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(
        www_authenticate(&res).starts_with("Bearer error=\"invalid_token\", resource_metadata=")
    );
    let res = refresh(&app, &client_id, renewed["refresh_token"].as_str().unwrap()).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["error"], "invalid_grant");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_refresh_token_used_twice_ends_the_connection(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let (client_id, tokens) = connect(&app, &maker, choices("publish")).await;
    assert_eq!(tokens["scope"], "read publish");
    let first = tokens["refresh_token"].as_str().unwrap();
    let renewed = body_json(refresh(&app, &client_id, first).await).await;
    let auth = bearer(&renewed);

    // The same token again at once (two copies of the tool refreshing together): refused, but
    // the connection goes on.
    let res = refresh(&app, &client_id, first).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
    let res = mcp(&app, "/mcp", Some(&auth), "tools/list", json!({})).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Later than that, it was copied: the connection ends.
    sqlx::query(
        "update refresh_tokens set used_at = now() - interval '1 minute' where used_at is not null",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let res = refresh(&app, &client_id, first).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
    let res = mcp(&app, "/mcp", Some(&auth), "tools/list", json!({})).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = refresh(&app, &client_id, renewed["refresh_token"].as_str().unwrap()).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn refresh_tokens_work_only_for_their_own_tool_and_endpoint(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let (client_id, tokens) = connect(&app, &maker, choices("read")).await;
    let other = register(&app, &[LOOPBACK]).await;
    let token = tokens["refresh_token"].as_str().unwrap();
    let res = refresh(&app, &other, token).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
    // Nor at the desktop app's endpoint.
    let res = app
        .send(
            req(Method::POST, "/v1/auth/desktop/token")
                .json(json!({ "grant_type": "refresh_token", "refresh_token": token })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // Neither spent it.
    let res = refresh(&app, &client_id, token).await;
    assert_eq!(res.status(), StatusCode::OK);
    // A resource other than SLAPorSLOP's is refused.
    let renewed = body_json(res).await;
    let res = post_form(
        &app,
        "/v1/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", renewed["refresh_token"].as_str().unwrap()),
            ("client_id", &client_id),
            ("resource", "https://other.example/mcp"),
        ],
    )
    .await;
    assert_eq!(body_json(res).await["error"], "invalid_target");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pkce_is_required_and_checked(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let client_id = register(&app, &[LOOPBACK]).await;
    let v = verifier();
    // No challenge, or a plain one: back to the tool with an error, and nothing to approve.
    for changes in [
        vec![("code_challenge", ""), ("code_challenge_method", "")],
        vec![("code_challenge_method", "plain")],
        vec![("code_challenge", "short")],
    ] {
        let res = app
            .send(req(Method::GET, authorize_url(&client_id, &v, &changes)))
            .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let to = location(&res);
        assert!(to.starts_with(LOOPBACK), "{to}");
        assert_eq!(query_param(&to, "error"), "invalid_request");
        assert_eq!(query_param(&to, "state"), "tool-state-123");
        assert_eq!(query_param(&to, "iss"), SITE);
    }
    let pending: i64 = sqlx::query_scalar("select count(*) from agent_auth_requests")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(pending, 0);

    // A wrong verifier, or none: refused, and the code is spent.
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let res = post_form(
        &app,
        "/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
        ],
    )
    .await;
    assert_eq!(body_json(res).await["error"], "invalid_request");
    let res = exchange(&app, &client_id, &code, &verifier()).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["error"], "invalid_grant");
    let res = exchange(&app, &client_id, &code, &v).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_return_address_must_be_registered_exactly(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let client_id = register(&app, &[LOOPBACK, "https://tool.example/oauth/callback"]).await;
    let v = verifier();
    // Nothing goes back to an address that isn't the tool's: the website explains instead.
    for redirect in [
        "https://evil.example/oauth/callback",
        "https://tool.example/oauth/callback/",
        "https://tool.example/oauth/callback?x=1",
        "http://127.0.0.1:45678/other",
        "http://evil.example/callback",
    ] {
        let res = app
            .send(req(
                Method::GET,
                authorize_url(&client_id, &v, &[("redirect_uri", redirect)]),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            location(&res),
            format!("{SITE}/connect-ai?problem=redirect"),
            "{redirect}"
        );
    }
    // An unknown tool likewise.
    let res = app
        .send(req(Method::GET, authorize_url("slopcl_nobody", &v, &[])))
        .await;
    assert_eq!(location(&res), format!("{SITE}/connect-ai?problem=client"));
    // A loopback address on another port is the same program's.
    let res = app
        .send(req(
            Method::GET,
            authorize_url(
                &client_id,
                &v,
                &[("redirect_uri", "http://localhost:5999/callback")],
            ),
        ))
        .await;
    assert!(location(&res).starts_with(&format!("{SITE}/connect-ai/")));

    // A website the page doesn't know is flagged.
    let res = app
        .send(req(
            Method::GET,
            authorize_url(
                &client_id,
                &v,
                &[("redirect_uri", "https://tool.example/oauth/callback")],
            ),
        ))
        .await;
    let id = location(&res).rsplit('/').next().unwrap().to_string();
    let info = body_json(
        app.send(req(Method::GET, format!("/v1/oauth/requests/{id}")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(info["redirect_kind"], "web");
    assert_eq!(info["redirect_host"], "tool.example");
    assert_eq!(info["unfamiliar"], true);

    // At the token endpoint, the redirect_uri must be the one approved.
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let res = post_form(
        &app,
        "/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &v),
            ("client_id", &client_id),
            ("redirect_uri", "https://tool.example/oauth/callback"),
        ],
    )
    .await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");

    // Registration refuses addresses that aren't https or this computer.
    for bad in [
        "http://evil.example/cb",
        "javascript:alert(1)",
        "cursor://anysphere.cursor-mcp/oauth/callback",
        "https://tool.example/cb#x",
    ] {
        let res = app
            .send(req(Method::POST, "/v1/oauth/register").json(json!({ "redirect_uris": [bad] })))
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(body_json(res).await["error"], "invalid_redirect_uri");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_resource_must_be_slaporslops_mcp_address(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let client_id = register(&app, &[LOOPBACK]).await;
    let v = verifier();
    for bad in [
        "https://other.example/mcp",
        "http://localhost:8080/v1/admin/mcp",
        "http://localhost:8080/mcp#x",
        "not a url",
    ] {
        let res = app
            .send(req(
                Method::GET,
                authorize_url(&client_id, &v, &[("resource", bad)]),
            ))
            .await;
        let to = location(&res);
        assert!(to.starts_with(LOOPBACK), "{bad}: {to}");
        assert_eq!(query_param(&to, "error"), "invalid_target");
    }
    // Either MCP address (or the site, as the root metadata names) is fine.
    for good in [
        "http://localhost:8080/v1/maker/mcp",
        "http://LOCALHOST:8080/mcp/",
        "http://localhost:8080",
    ] {
        let res = app
            .send(req(
                Method::GET,
                authorize_url(&client_id, &v, &[("resource", good)]),
            ))
            .await;
        assert!(
            location(&res).starts_with(&format!("{SITE}/connect-ai/")),
            "{good}"
        );
    }
    // The token request must name the same one.
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let res = post_form(
        &app,
        "/v1/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &v),
            ("client_id", &client_id),
            ("resource", "http://localhost:8080/v1/maker/mcp"),
        ],
    )
    .await;
    assert_eq!(body_json(res).await["error"], "invalid_target");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn codes_expire_work_once_and_only_for_their_tool(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let client_id = register(&app, &[LOOPBACK]).await;

    // Expired.
    let v = verifier();
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    sqlx::query("update agent_auth_requests set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = exchange(&app, &client_id, &code, &v).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");

    // Another tool's.
    let v = verifier();
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let other = register(&app, &[LOOPBACK]).await;
    let res = exchange(&app, &other, &code, &v).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");

    // Used twice: the second use ends what the first made.
    let v = verifier();
    let id = start(&app, &client_id, &v).await;
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let tokens = body_json(exchange(&app, &client_id, &code, &v).await).await;
    let auth = bearer(&tokens);
    assert_eq!(
        mcp(&app, "/mcp", Some(&auth), "tools/list", json!({}))
            .await
            .status(),
        StatusCode::OK
    );
    let res = exchange(&app, &client_id, &code, &v).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
    assert_eq!(
        mcp(&app, "/mcp", Some(&auth), "tools/list", json!({}))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let res = refresh(&app, &client_id, tokens["refresh_token"].as_str().unwrap()).await;
    assert_eq!(body_json(res).await["error"], "invalid_grant");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn deny_sends_access_denied_back(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let client_id = register(&app, &[LOOPBACK]).await;
    let id = start(&app, &client_id, &verifier()).await;
    // Signed out, nothing happens.
    let res = app
        .send(req(Method::POST, format!("/v1/oauth/requests/{id}/deny")))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = app
        .send(req(Method::POST, format!("/v1/oauth/requests/{id}/deny")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let to = body_json(res).await["redirect_url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(to.starts_with(LOOPBACK), "{to}");
    assert_eq!(query_param(&to, "error"), "access_denied");
    assert_eq!(query_param(&to, "state"), "tool-state-123");
    assert_eq!(query_param(&to, "iss"), SITE);
    // It's gone: it can't be approved after.
    assert_eq!(
        approve(&app, &maker, &id, choices("read")).await.status(),
        StatusCode::NOT_FOUND
    );
    // A request from another site's page is refused (CSRF), as for every cookie request.
    let id = start(&app, &client_id, &verifier()).await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/oauth/requests/{id}/approve"))
                .cookie(&maker)
                .header("sec-fetch-site", "cross-site")
                .json(choices("read")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn publishing_needs_the_publisher_agreement(db: PgPool) {
    let app = oauth_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'publisher'")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let client_id = register(&app, &[LOOPBACK]).await;
    let v = verifier();
    let id = start(&app, &client_id, &v).await;
    let res = approve(&app, &maker, &id, choices("publish")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(err["error"]["code"], "terms_required");
    assert_eq!(err["error"]["field"], "publisher");
    // Looking needs nothing; accepting it, publishing works too (the request still waits).
    let version: String =
        sqlx::query_scalar("select version from legal_documents where kind = 'publisher' and effective_at <= now() order by effective_at desc limit 1")
            .fetch_one(&app.db)
            .await
            .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/me/legal/accept")
                .cookie(&maker)
                .json(json!({ "kind": "publisher", "version": version })),
        )
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    let answer = body_json(approve(&app, &maker, &id, choices("publish")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let tokens = body_json(exchange(&app, &client_id, &code, &v).await).await;
    assert_eq!(tokens["scope"], "read publish");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_connection_for_some_apps_only_reaches_those(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    make_app(&app, &maker, "alpha").await;
    make_app(&app, &maker, "beta").await;
    let (_, other) = app.user("Otto Other", "US").await;
    make_app(&app, &other, "ottos-game").await;
    // Another maker's app can't be chosen.
    let client_id = register(&app, &[LOOPBACK]).await;
    let id = start(&app, &client_id, &verifier()).await;
    let res = approve(
        &app,
        &maker,
        &id,
        json!({ "name": "Tool", "scope": "publish", "apps": ["ottos-game"], "days": 7 }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let (_, tokens) = connect(
        &app,
        &maker,
        json!({ "name": "Alpha only", "scope": "publish", "apps": ["alpha"], "people": false, "days": 7 }),
    )
    .await;
    let auth = bearer(&tokens);
    let (failed, text) = tool(&app, &auth, "app_overview", json!({ "slug": "alpha" })).await;
    assert!(!failed, "{text}");
    for slug in ["beta", "ottos-game"] {
        let (failed, text) = tool(&app, &auth, "app_overview", json!({ "slug": slug })).await;
        assert!(failed && text.contains("not_found"), "{slug}: {text}");
    }
    let (failed, text) = tool(
        &app,
        &auth,
        "update_store_page",
        json!({ "slug": "beta", "summary": "Not this one." }),
    )
    .await;
    assert!(failed, "{text}");
    // Without People access, no jobs.
    let (failed, text) = tool(&app, &auth, "my_jobs", json!({})).await;
    assert!(failed && text.contains("forbidden"), "{text}");
    // The connection lasts the days chosen: a week.
    let days: f64 = sqlx::query_scalar(
        "select extract(epoch from expires_at - created_at)::float8 / 86400 from sessions where kind = 'agent'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((days - 7.0).abs() < 0.01, "{days}");
    let refresh_end: bool = sqlx::query_scalar(
        "select bool_and(r.expires_at = s.expires_at) from refresh_tokens r join sessions s on s.id = r.session_id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(refresh_end, "refresh tokens end with the connection");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_mcp_endpoint_says_how_to_connect(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    for (path, metadata) in [
        ("/mcp", "/.well-known/oauth-protected-resource/mcp"),
        (
            "/v1/maker/mcp",
            "/.well-known/oauth-protected-resource/v1/maker/mcp",
        ),
    ] {
        let res = mcp(&app, path, None, "initialize", json!({})).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert!(
            www_authenticate(&res).contains(&format!("resource_metadata=\"{SITE}{metadata}\"")),
            "{path}"
        );
        let res = mcp(
            &app,
            path,
            Some("Bearer slopmk_wrong"),
            "initialize",
            json!({}),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(www_authenticate(&res).contains("error=\"invalid_token\""));
        // GET (a stream the server doesn't offer) says the same.
        let res = app.send(req(Method::GET, path)).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(www_authenticate(&res).contains("resource_metadata="));
        let doc = body_json(app.send(req(Method::GET, metadata)).await).await;
        assert_eq!(doc["resource"], format!("{SITE}{path}"));
    }
    let root = body_json(
        app.send(req(Method::GET, "/.well-known/oauth-protected-resource"))
            .await,
    )
    .await;
    assert_eq!(root["resource"], SITE);
    // Not the website's page where a tool looks for something that isn't here.
    for path in [
        "/.well-known/openid-configuration",
        "/.well-known/oauth-protected-resource/v1/admin/mcp",
        "/.well-known/oauth-authorization-server/mcp",
    ] {
        assert_eq!(
            app.send(req(Method::GET, path)).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    // The short address takes a token made on the website too, and not the website's session.
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Made here", "scope": "read" })),
        )
        .await;
    let secret = body_json(res).await["secret"].as_str().unwrap().to_string();
    let res = mcp(
        &app,
        "/mcp",
        Some(&format!("Bearer {secret}")),
        "tools/list",
        json!({}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::POST, "/mcp")
                .cookie(&maker)
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // An approval page's API refuses an agent.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/oauth/requests/{}/approve", Uuid::now_v7()),
            )
            .header("authorization", format!("Bearer {secret}"))
            .json(choices("read")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_tool_can_end_its_own_connection(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    let (client_id, tokens) = connect(&app, &maker, choices("read")).await;
    let auth = bearer(&tokens);
    let refresh_token = tokens["refresh_token"].as_str().unwrap();
    // Another tool can't.
    let other = register(&app, &[LOOPBACK]).await;
    let res = post_form(
        &app,
        "/v1/oauth/revoke",
        &[("token", refresh_token), ("client_id", &other)],
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        mcp(&app, "/mcp", Some(&auth), "tools/list", json!({}))
            .await
            .status(),
        StatusCode::OK
    );
    let res = post_form(
        &app,
        "/v1/oauth/revoke",
        &[("token", refresh_token), ("client_id", &client_id)],
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        mcp(&app, "/mcp", Some(&auth), "tools/list", json!({}))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn connections_count_toward_the_ten_agents(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    for n in 0..10 {
        let res = app
            .send(
                req(Method::POST, "/v1/me/maker-tokens")
                    .cookie(&maker)
                    .json(json!({ "name": format!("Token {n}"), "scope": "read" })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    let client_id = register(&app, &[LOOPBACK]).await;
    let id = start(&app, &client_id, &verifier()).await;
    let res = approve(&app, &maker, &id, choices("read")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("remove one first")
    );
    // Removing one there lets the same request go on.
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    let one = list[0]["id"].as_str().unwrap();
    app.send(req(Method::DELETE, format!("/v1/me/maker-tokens/{one}")).cookie(&maker))
        .await;
    assert_eq!(
        approve(&app, &maker, &id, choices("read")).await.status(),
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn registration_is_limited_and_unused_tools_are_forgotten(db: PgPool) {
    let app = oauth_app(db).await;
    let one = |ip: &'static str| {
        req(Method::POST, "/v1/oauth/register")
            .header("x-test-client-ip", ip)
            .json(json!({ "client_name": "Spam", "redirect_uris": [LOOPBACK] }))
    };
    for _ in 0..20 {
        assert_eq!(
            app.send(one("203.0.113.9")).await.status(),
            StatusCode::CREATED
        );
    }
    assert_eq!(
        app.send(one("203.0.113.9")).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.send(one("203.0.113.10")).await.status(),
        StatusCode::CREATED
    );
    // A name is tidied; none at all gets a plain one.
    let res = app
        .send(req(Method::POST, "/v1/oauth/register").json(json!({
            "client_name": "  Claude\u{202E}\n Code  ",
            "redirect_uris": [LOOPBACK]
        })))
        .await;
    assert_eq!(body_json(res).await["client_name"], "Claude Code");
    // Grants other than the code and refreshing aren't offered.
    let res = app
        .send(req(Method::POST, "/v1/oauth/register").json(json!({
            "redirect_uris": [LOOPBACK], "grant_types": ["client_credentials"]
        })))
        .await;
    assert_eq!(body_json(res).await["error"], "invalid_client_metadata");

    // A day later, the ones that never started an approval are gone; one that did stays.
    let used = register(&app, &[LOOPBACK]).await;
    start(&app, &used, &verifier()).await;
    sqlx::query("update agent_clients set created_at = now() - interval '2 days'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::cleanup::run_once(&app.db).await.unwrap();
    let left: Vec<String> = sqlx::query_scalar("select client_id from agent_clients")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(left, vec![used.clone()]);
    // 30 days after its last use with nothing working, it goes too.
    sqlx::query("update agent_clients set last_used_at = now() - interval '31 days'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update agent_auth_requests set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::cleanup::run_once(&app.db).await.unwrap();
    let left: i64 = sqlx::query_scalar("select count(*) from agent_clients")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

/// Serves a tool's description at `/client.json`, as `body` says (with its own address put in).
async fn description_server(body: impl Fn(&str) -> Value + Send + Sync + 'static) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let address = format!("{base}/client.json");
    let doc = body(&address);
    let app = Router::new().route(
        "/client.json",
        get(move || {
            let doc = doc.clone();
            async move { ([("cache-control", "max-age=300")], axum::Json(doc)) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_tool_named_by_its_description_address(db: PgPool) {
    let app = oauth_app(db).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    // As ChatGPT's: it prefers private_key_jwt but supports none, which is all SLAPorSLOP offers.
    let client_id = description_server(|address| {
        json!({
            "client_id": address,
            "client_name": "Described Tool",
            "redirect_uris": [LOOPBACK],
            "token_endpoint_auth_method": "private_key_jwt",
            "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
        })
    })
    .await;
    let v = verifier();
    let id = start(&app, &client_id, &v).await;
    let info = body_json(
        app.send(req(Method::GET, format!("/v1/oauth/requests/{id}")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(info["client_name"], "Described Tool");
    assert_eq!(info["client_kind"], "document");
    assert_eq!(info["client_host"], "127.0.0.1");
    let answer = body_json(approve(&app, &maker, &id, choices("read")).await).await;
    let code = code_of(answer["redirect_url"].as_str().unwrap());
    let res = exchange(&app, &client_id, &code, &v).await;
    assert_eq!(res.status(), StatusCode::OK);

    // A description naming another address, or using a shared secret, isn't accepted.
    for doc in [
        (|_: &str| json!({ "client_id": "https://claude.ai/oauth/x", "client_name": "Fake", "redirect_uris": [LOOPBACK] }))
            as fn(&str) -> Value,
        |a: &str| json!({ "client_id": a, "redirect_uris": [LOOPBACK], "token_endpoint_auth_method": "client_secret_basic" }),
        |a: &str| json!({ "client_id": a, "redirect_uris": ["http://evil.example/cb"] }),
    ] {
        let client_id = description_server(doc).await;
        let res = app
            .send(req(Method::GET, authorize_url(&client_id, &v, &[])))
            .await;
        assert_eq!(location(&res), format!("{SITE}/connect-ai?problem=client"));
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_connection_ending_says_to_approve_again_not_to_make_a_token(db: PgPool) {
    let app = oauth_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, tokens) = connect(&app, &maker, choices("read")).await;
    let auth = bearer(&tokens);
    sqlx::query(
        "update sessions set created_at = now() - interval '60 days', expires_at = now() + interval '3 days'
         where kind = 'agent'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    // The agent hears it from its guide.
    let res = app
        .send(req(Method::GET, "/v1/maker/guide").header("authorization", auth.clone()))
        .await;
    let guide =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(
        guide.contains(
            "This connection (\"Claude Code\", approved in the browser) expires in 3 days"
        ),
        "{guide}"
    );
    assert!(
        guide.contains("approves you again in their browser"),
        "{guide}"
    );
    // The owner is told the same way.
    assert_eq!(
        slop_api::maker_agents::warn_expiring(&app.db)
            .await
            .unwrap(),
        1
    );
    let (title, body): (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and category = 'agents'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(title, "Your AI tool \"Claude Code\" disconnects in 3 days");
    assert!(body.contains("approve it in your browser"), "{body}");
    assert!(!body.contains("replacement"), "{body}");
}
