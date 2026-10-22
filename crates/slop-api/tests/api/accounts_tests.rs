// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign-up, sign-in, sessions, passkeys, linking and merging, closing an account, maker keys, repository tokens.

use crate::*;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------------------------
// Helpers

/// What the fake providers were sent: (provider, token request form).
type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// Google, GitLab, Bitbucket, Microsoft and Discord in one fake. The code is the access token and
/// names the account; "refused" gets a token error and "garbage" an account Slop can't read.
/// Google "g-unverified" has an unverified email and "g-long" a 50-character name; GitLab
/// "gl-unconfirmed" an unconfirmed email; Bitbucket "bb-noemails" can't list its emails.
async fn fake_providers() -> (String, Seen) {
    use axum::extract::{Path, State};
    async fn token(
        State(seen): State<Seen>,
        Path(provider): Path<String>,
        body: String,
    ) -> axum::Json<Value> {
        seen.lock().unwrap().push((provider, body.clone()));
        let code = form_code(&body);
        if code == "refused" {
            return axum::Json(json!({ "error": "bad_verification_code" }));
        }
        axum::Json(json!({ "access_token": code }))
    }
    let garbage = || axum::Json(json!({ "unexpected": true }));
    let app = Router::new()
        .route("/{provider}/token", post(token))
        .route(
            "/google/userinfo",
            get(move |headers: axum::http::HeaderMap| async move {
                let t = bearer(&headers);
                match t.as_str() {
                    "garbage" => garbage(),
                    "g-long" => axum::Json(json!({ "sub": t, "name": "é".repeat(50), "email": "long@example.com", "email_verified": true })),
                    "g-unverified" => axum::Json(json!({ "sub": t, "name": "Una", "email": "una@example.com", "email_verified": false })),
                    _ => axum::Json(json!({ "sub": t, "name": format!("Google {t}"), "email": format!("{t}@example.com"), "email_verified": true })),
                }
            }),
        )
        .route(
            "/gitlab/api/user",
            get(move |headers: axum::http::HeaderMap| async move {
                match bearer(&headers).as_str() {
                    "garbage" => garbage(),
                    "gl-unconfirmed" => axum::Json(json!({ "id": 701, "username": "una", "name": "Una Lab", "email": "una@example.com", "confirmed_at": null })),
                    t => axum::Json(json!({ "id": 700, "username": t, "name": "Gita Lab", "email": format!("{t}@example.com"), "confirmed_at": "2026-01-01T00:00:00Z" })),
                }
            }),
        )
        .route(
            "/bitbucket/user",
            get(move |headers: axum::http::HeaderMap| async move {
                match bearer(&headers).as_str() {
                    "garbage" => garbage(),
                    t => axum::Json(json!({ "uuid": format!("{{{t}}}"), "username": t, "display_name": "Bit Bucket" })),
                }
            }),
        )
        .route(
            "/bitbucket/user/emails",
            get(|headers: axum::http::HeaderMap| async move {
                let t = bearer(&headers);
                if t == "bb-noemails" {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                axum::Json(json!({ "values": [
                    { "email": "unconfirmed@example.com", "is_primary": true, "is_confirmed": false },
                    { "email": "old@example.com", "is_primary": false, "is_confirmed": true },
                    { "email": format!("{t}@example.com"), "is_primary": true, "is_confirmed": true }
                ] }))
                .into_response()
            }),
        )
        .route(
            "/microsoft/oidc/userinfo",
            get(move |headers: axum::http::HeaderMap| async move {
                match bearer(&headers).as_str() {
                    "garbage" => garbage(),
                    t => axum::Json(json!({ "sub": format!("ms-{t}"), "name": "Mia Soft" })),
                }
            }),
        )
        .route(
            "/discord/users/@me",
            get(move |headers: axum::http::HeaderMap| async move {
                match bearer(&headers).as_str() {
                    "garbage" => garbage(),
                    t => axum::Json(json!({ "id": format!("d-{t}"), "username": t, "global_name": "Dee", "email": "dee@example.com", "verified": true })),
                }
            }),
        );
    let seen = Seen::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = app.with_state(seen.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), seen)
}

/// Turns on every sign-in provider, against `fake_providers` at `base`.
fn all_providers(c: &mut Config, base: &str) {
    let base: url::Url = base.parse().unwrap();
    let oauth = |name: &str| slop_api::config::OAuthConfig {
        client_id: format!("{name}-client"),
        client_secret: "secret".into(),
        authorize_url: base.join(&format!("{name}/auth")).unwrap(),
        token_url: base.join(&format!("{name}/token")).unwrap(),
        api_base: base
            .join(&if name == "gitlab" {
                "gitlab/api/".to_string()
            } else {
                format!("{name}/")
            })
            .unwrap(),
    };
    c.google = Some(oauth("google"));
    c.gitlab = Some(oauth("gitlab"));
    c.bitbucket = Some(oauth("bitbucket"));
    c.microsoft = Some(oauth("microsoft"));
    c.discord = Some(oauth("discord"));
}

/// A test app with every sign-in provider on, and what the providers were sent.
async fn app_with_providers(db: PgPool) -> (TestApp, Seen) {
    let (base, seen) = fake_providers().await;
    let app = test_app_with(db, move |c| all_providers(c, &base)).await;
    (app, seen)
}

/// Links a provider account to a user directly.
async fn identity(app: &TestApp, user: Uuid, provider: &str, subject: &str) {
    sqlx::query("insert into identities (provider, provider_subject, user_id) values ($1, $2, $3)")
        .bind(provider)
        .bind(subject)
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Makes the user's sessions old: signed in an hour ago, never confirmed since.
async fn age_sessions(app: &TestApp, user: Uuid) {
    sqlx::query(
        "update sessions set created_at = now() - interval '1 hour', stepped_up_at = null, passkey_at = null
         where user_id = $1",
    )
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Sets a column of the user's sessions to now (`stepped_up_at`, `passkey_at`, `created_at`).
async fn sessions_now(app: &TestApp, user: Uuid, column: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update sessions set {column} = now() where user_id = $1"
    )))
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A passkey row for the user (not usable for signing: enough for "has a passkey").
async fn stored_passkey(app: &TestApp, user: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, '{}', 'Laptop')",
    )
    .bind(id)
    .bind(user)
    .bind(id.as_bytes().to_vec())
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// Whether the response removes the cookie (an empty value that has expired).
fn removes_cookie(res: &Response, name: &str) -> bool {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.starts_with(&format!("{name}=;")) && v.contains("Max-Age=0"))
}

/// The raw Set-Cookie header for a cookie.
fn raw_cookie(res: &Response, name: &str) -> String {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap_or_default()
        .to_string()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn status(app: &TestApp, r: Req) -> StatusCode {
    app.send(r).await.status()
}

async fn error_code(app: &TestApp, r: Req) -> Value {
    body_json(app.send(r).await).await["error"]["code"].clone()
}

fn signup_body(name: &str, country: &str) -> Value {
    json!({ "display_name": name, "country_code": country, "age_confirmed": true, "accept_terms": true })
}

// ---------------------------------------------------------------------------------------------
// OAuth sign-in

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn starting_a_sign_in_checks_the_provider_and_keeps_return_to_on_site(db: PgPool) {
    let app = test_app(db).await;
    assert_eq!(
        status(&app, req(Method::GET, "/v1/auth/myspace/start")).await,
        StatusCode::NOT_FOUND
    );
    // Google isn't set up on this server.
    let res = app.send(req(Method::GET, "/v1/auth/google/start")).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(res).await["error"]["code"], "unavailable");
    assert_eq!(count(&app, "select count(*) from oauth_flows").await, 0);

    // Somewhere off the site is never kept as the place to come back to.
    for bad in [
        "%2F%2Fevil.example",
        "https%3A%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "apps",
    ] {
        sqlx::query("delete from oauth_flows")
            .execute(&app.db)
            .await
            .unwrap();
        let res = app
            .send(req(
                Method::GET,
                format!("/v1/auth/github/start?return_to={bad}"),
            ))
            .await;
        assert!(res.status().is_redirection());
        let kept: String = sqlx::query_scalar("select return_to from oauth_flows")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(kept, "/", "{bad}");
    }

    // End to end: an existing account signs in and lands on the home page, not the other site.
    let (user, _) = app.user("Octo", "US").await;
    identity(&app, user, "github", "42").await;
    let res = oauth_flow(
        &app,
        "/v1/auth/github/start?return_to=%2F%2Fevil.example",
        &[],
        "ok",
    )
    .await;
    assert_eq!(location(&res), "/");
    assert!(set_cookie(&res, "slop_session").is_some());
    let res = oauth_flow(
        &app,
        "/v1/auth/github/start?return_to=%2Fapps%2Fx%3Fy%3D1",
        &[],
        "ok",
    )
    .await;
    assert_eq!(location(&res), "/apps/x?y=1");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn each_provider_is_asked_the_way_it_expects(db: PgPool) {
    let (app, seen) = app_with_providers(db).await;

    // Google: its account chooser, PKCE and the callback address.
    let res = app.send(req(Method::GET, "/v1/auth/google/start")).await;
    let google = location(&res);
    assert_eq!(query_param(&google, "prompt"), "select_account");
    assert_eq!(query_param(&google, "code_challenge_method"), "S256");
    assert_eq!(query_param(&google, "client_id"), "google-client");
    assert_eq!(
        query_param(&google, "redirect_uri"),
        "http://localhost:8080/v1/auth/google/callback"
    );

    // Bitbucket: the callback registered with the app, and no PKCE (it doesn't take it).
    let res = app.send(req(Method::GET, "/v1/auth/bitbucket/start")).await;
    let bitbucket = location(&res);
    assert!(bitbucket.contains("/bitbucket/auth?"), "{bitbucket}");
    assert!(!bitbucket.contains("redirect_uri"), "{bitbucket}");
    assert!(!bitbucket.contains("code_challenge"), "{bitbucket}");
    assert_eq!(query_param(&bitbucket, "response_type"), "code");
    assert!(!query_param(&bitbucket, "state").is_empty());

    // The token requests follow suit.
    oauth_flow(&app, "/v1/auth/bitbucket/start", &[], "bb-ann").await;
    oauth_flow(&app, "/v1/auth/google/start", &[], "g-ann").await;
    let seen = seen.lock().unwrap().clone();
    let form = |provider: &str| {
        seen.iter()
            .find(|(p, _)| p == provider)
            .map(|(_, f)| f.clone())
            .unwrap()
    };
    let bb = form("bitbucket");
    assert!(
        bb.contains("grant_type=authorization_code") && bb.contains("code=bb-ann"),
        "{bb}"
    );
    assert!(
        !bb.contains("redirect_uri") && !bb.contains("code_verifier"),
        "{bb}"
    );
    assert!(
        !bb.contains("client_secret"),
        "Bitbucket takes the secret as Basic auth"
    );
    let g = form("google");
    assert!(
        g.contains("redirect_uri=") && g.contains("code_verifier="),
        "{g}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn on_https_the_sign_in_cookies_are_host_only_and_secure(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.public_url = "https://slop.example".parse().unwrap()
    })
    .await;
    let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
    let to_github = location(&res);
    assert_eq!(
        query_param(&to_github, "redirect_uri"),
        "https://slop.example/v1/auth/github/callback"
    );
    let raw = raw_cookie(&res, "__Host-slop_oauth_state");
    for part in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(raw.contains(part), "{part}: {raw}");
    }
    assert!(set_cookie(&res, "slop_oauth_state").is_none());
    let state = query_param(&to_github, "state");

    // The plain cookie name isn't read on https.
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(format!("slop_oauth_state={state}")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(format!("__Host-slop_oauth_state={state}")),
        )
        .await;
    assert_eq!(location(&res), "/signup");
    let signup = raw_cookie(&res, "__Host-slop_signup");
    assert!(
        signup.contains("Secure") && signup.contains("HttpOnly"),
        "{signup}"
    );
    assert!(removes_cookie(&res, "__Host-slop_oauth_state"));

    // Signing up sets the __Host- session cookie.
    let signup = set_cookie(&res, "__Host-slop_signup").unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&signup)
                .json(signup_body("Octo", "US")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let session = raw_cookie(&res, "__Host-slop_session");
    assert!(session.contains("Secure"), "{session}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn callbacks_refuse_cancelled_incomplete_expired_and_crossed_flows(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let start = || async {
        let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
        (
            query_param(&location(&res), "state"),
            set_cookie(&res, "slop_oauth_state").unwrap(),
        )
    };

    // The person said no at the provider.
    let (state, cookie) = start().await;
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?error=access_denied&state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(location(&res), "/signin?error=cancelled");
    assert!(removes_cookie(&res, "slop_oauth_state"));

    // No code, or no state.
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "code");
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/auth/github/callback?code=ok").cookie(&cookie)
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A state from GitHub's flow doesn't finish Google's, and GitHub's still works after.
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/google/callback?code=g-ann&state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(location(&res), "/signup");

    // Expired flows are refused.
    let (state, cookie) = start().await;
    sqlx::query("update oauth_flows set expires_at = now() - interval '1 minute'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "state");
    assert_eq!(count(&app, "select count(*) from pending_signups").await, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signing_in_again_updates_the_sign_in_and_records_the_device(db: PgPool) {
    let app = test_app(db).await;
    let (user, _) = app.user("Octo", "US").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id, login, last_sign_in_at)
         values ('github', '42', $1, 'old-login', now() - interval '30 days')",
    )
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
    let agent = "Mozilla/5.0 ".repeat(30);
    let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
    let state = query_param(&location(&res), "state");
    let cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&cookie)
            .header("user-agent", agent.clone()),
        )
        .await;
    assert_eq!(location(&res), "/");
    let session = set_cookie(&res, "slop_session").unwrap();
    let (login, recent): (String, bool) = sqlx::query_as(
        "select login, last_sign_in_at > now() - interval '1 minute' from identities where user_id = $1",
    )
    .bind(user)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(login, "octo", "the login follows a rename at the provider");
    assert!(recent);
    let kept: String = sqlx::query_scalar(
        "select user_agent from sessions where user_id = $1 order by created_at desc limit 1",
    )
    .bind(user)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(kept.chars().count(), 200);
    assert!(agent.starts_with(&kept));
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(me["id"], json!(user));

    // A suspended account still signs in (to read, appeal or leave), but can't change anything.
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let session = set_cookie(&res, "slop_session").unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(me["suspended"], true);
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&session)
                .json(new_app("nope")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "suspended");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn new_sign_ins_bring_only_what_the_provider_has_checked(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let pending = |res: Response| {
        let cookie = set_cookie(&res, "slop_signup").unwrap();
        let app = &app;
        async move {
            let res = app
                .send(req(Method::GET, "/v1/auth/signup").cookie(&cookie))
                .await;
            assert_eq!(res.status(), StatusCode::OK);
            (body_json(res).await, cookie)
        }
    };

    // A long name is cut to 40 characters (not bytes).
    let (info, _) = pending(oauth_flow(&app, "/v1/auth/google/start", &[], "g-long").await).await;
    assert_eq!(info["suggested_name"], "é".repeat(40));
    assert_eq!(info["email"], "long@example.com");

    // Emails the provider hasn't checked are dropped.
    let (info, _) =
        pending(oauth_flow(&app, "/v1/auth/google/start", &[], "g-unverified").await).await;
    assert_eq!(info["suggested_name"], "Una");
    assert!(info["email"].is_null(), "{info}");
    let (info, _) =
        pending(oauth_flow(&app, "/v1/auth/gitlab/start", &[], "gl-unconfirmed").await).await;
    assert_eq!(info["suggested_name"], "Una Lab");
    assert!(info["email"].is_null(), "{info}");

    // Bitbucket: the account's UUID, its username, and the primary confirmed email.
    let (info, cookie) =
        pending(oauth_flow(&app, "/v1/auth/bitbucket/start", &[], "bb-ann").await).await;
    assert_eq!(info["suggested_name"], "Bit Bucket");
    assert_eq!(info["email"], "bb-ann@example.com");
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&cookie)
                .json(signup_body("Ann", "US")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let (subject, login): (String, String) = sqlx::query_as(
        "select provider_subject, login from identities where provider = 'bitbucket'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((subject.as_str(), login.as_str()), ("{bb-ann}", "bb-ann"));
    // Its email list failing doesn't stop the sign-in; there's just no email.
    let (info, _) =
        pending(oauth_flow(&app, "/v1/auth/bitbucket/start", &[], "bb-noemails").await).await;
    assert!(info["email"].is_null(), "{info}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_provider_answer_slop_cant_read_sends_the_visitor_back_to_sign_in(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    for provider in ["google", "gitlab", "bitbucket", "microsoft", "discord"] {
        for code in ["garbage", "refused"] {
            let res = oauth_flow(&app, &format!("/v1/auth/{provider}/start"), &[], code).await;
            assert_eq!(
                location(&res),
                "/signin?error=provider",
                "{provider} {code}"
            );
            assert!(set_cookie(&res, "slop_signup").is_none());
        }
    }
    assert_eq!(count(&app, "select count(*) from pending_signups").await, 0);
    assert_eq!(count(&app, "select count(*) from users").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn linking_again_after_a_failure_or_signed_out_says_what_happened(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    identity(&app, ann_id, "google", "g-ann").await;

    // The same sign-in again: nothing to do.
    let res = oauth_flow(&app, "/v1/auth/google/link", &[&ann], "g-ann").await;
    assert_eq!(location(&res), "/account?linked=already#sign-ins");
    assert_eq!(count(&app, "select count(*) from identities").await, 1);

    // The provider failing during a link goes back to where the link started, not to sign-in.
    let res = oauth_flow(&app, "/v1/auth/gitlab/link", &[&ann], "garbage").await;
    assert_eq!(location(&res), "/account?error=provider#sign-ins");
    let res = oauth_flow(
        &app,
        "/v1/auth/gitlab/link?return_to=%2Fsettings%3Ftab%3D1",
        &[&ann],
        "refused",
    )
    .await;
    assert_eq!(location(&res), "/settings?tab=1&error=provider");

    // Finished after signing out (no session at all): start again.
    let res = app
        .send(req(Method::GET, "/v1/auth/gitlab/link").cookie(&ann))
        .await;
    let state = query_param(&location(&res), "state");
    let cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/gitlab/callback?code=gl-ann&state={state}"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(location(&res), "/signin?error=link_expired");
    assert_eq!(count(&app, "select count(*) from identities").await, 1);
    assert_eq!(count(&app, "select count(*) from pending_signups").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn linking_needs_a_fresh_sign_in_and_a_passkey_when_the_account_has_one(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    stored_passkey(&app, ann_id).await;
    let link = |path: &str| req(Method::GET, path.to_string()).cookie(&ann);

    // A fresh sign-in isn't enough: the account page asks for the passkey first.
    let res = app.send(link("/v1/auth/gitlab/link")).await;
    assert_eq!(location(&res), "/account?confirm_link=gitlab#sign-ins");
    assert_eq!(count(&app, "select count(*) from oauth_flows").await, 0);

    // Confirmed with it: on to GitLab, tied to this account and session.
    sessions_now(&app, ann_id, "passkey_at").await;
    let res = app.send(link("/v1/auth/gitlab/link")).await;
    assert!(
        location(&res).contains("/gitlab/auth?"),
        "{}",
        location(&res)
    );
    let (user, return_to): (Option<Uuid>, String) =
        sqlx::query_as("select link_user_id, return_to from oauth_flows")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(user, Some(ann_id));
    assert_eq!(return_to, "/account#sign-ins");

    // An old sign-in confirmed with the passkey since counts as fresh.
    age_sessions(&app, ann_id).await;
    sqlx::query("update sessions set stepped_up_at = now(), passkey_at = now() where user_id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(link("/v1/auth/gitlab/link")).await;
    assert!(
        location(&res).contains("/gitlab/auth?"),
        "{}",
        location(&res)
    );

    // Where to come back to is kept on the site.
    for (asked, kept) in [
        ("%2F%2Fevil.example", "/"),
        ("https%3A%2F%2Fevil.example", "/"),
        ("%2Fsettings%23x", "/settings#x"),
    ] {
        sqlx::query("delete from oauth_flows")
            .execute(&app.db)
            .await
            .unwrap();
        let res = app
            .send(link(&format!("/v1/auth/google/link?return_to={asked}")))
            .await;
        assert!(res.status().is_redirection());
        let return_to: String = sqlx::query_scalar("select return_to from oauth_flows")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(return_to, kept, "{asked}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_ins_are_listed_only_for_their_owner(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, ann_id, "google", "g-ann").await;
    identity(&app, bob_id, "gitlab", "502").await;
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me/identities")).await,
        StatusCode::UNAUTHORIZED
    );
    let listed = body_json(
        app.send(req(Method::GET, "/v1/me/identities").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["provider"], "google");
    assert_eq!(listed[0]["provider_subject"], "g-ann");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unlinking_needs_a_fresh_proof_and_only_touches_your_own_sign_ins(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, ann_id, "google", "g-ann").await;
    identity(&app, ann_id, "gitlab", "501").await;
    identity(&app, bob_id, "gitlab", "502").await;
    let unlink = |path: &str| req(Method::DELETE, format!("/v1/me/identities/{path}")).cookie(&ann);
    let identities = || count(&app, "select count(*) from identities");

    // An old session can't.
    age_sessions(&app, ann_id).await;
    assert_eq!(
        error_code(&app, unlink("gitlab/501")).await,
        "step_up_required"
    );
    assert_eq!(identities().await, 3);
    sessions_now(&app, ann_id, "created_at").await;

    // Someone else's, or one that doesn't exist: not found, and nothing goes.
    assert_eq!(
        status(&app, unlink("gitlab/502")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, unlink("github/999")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(identities().await, 3);

    // With a passkey on the account, it must confirm.
    stored_passkey(&app, ann_id).await;
    assert_eq!(
        error_code(&app, unlink("gitlab/501")).await,
        "step_up_required"
    );
    sessions_now(&app, ann_id, "passkey_at").await;
    assert_eq!(
        status(&app, unlink("gitlab/501")).await,
        StatusCode::NO_CONTENT
    );

    // The last one stays, untouched.
    assert_eq!(
        status(&app, unlink("google/g-ann")).await,
        StatusCode::CONFLICT
    );
    let left: Vec<(String, Uuid)> =
        sqlx::query_as("select provider, user_id from identities order by provider")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        left,
        [
            ("gitlab".to_string(), bob_id),
            ("google".to_string(), ann_id)
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Sign-up

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_up_countries_are_the_ones_open_for_sign_up(db: PgPool) {
    let app = test_app(db).await;
    sqlx::query("update country_policies set signups_allowed = false where country_code = 'FR'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(req(Method::GET, "/v1/countries")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let countries = body_json(res).await;
    let names: Vec<&str> = countries
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "ordered by name");
    let codes: Vec<&str> = countries
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"GB") && codes.contains(&"US"));
    assert!(!codes.contains(&"FR"), "closed to sign-ups");
    let open: i64 = count(
        &app,
        "select count(*) from country_policies where signups_allowed",
    )
    .await;
    assert_eq!(codes.len() as i64, open);
    assert!(
        countries
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["min_age"] == 16)
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_pending_sign_up_is_only_shown_to_its_browser_while_it_lasts(db: PgPool) {
    let app = test_app(db).await;
    let info = |cookie: Option<&str>| {
        let mut r = req(Method::GET, "/v1/auth/signup");
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        r
    };
    assert_eq!(status(&app, info(None)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status(&app, info(Some("slop_signup=forged"))).await,
        StatusCode::NOT_FOUND
    );
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    assert_eq!(status(&app, info(Some(&cookie))).await, StatusCode::OK);

    // Older than 30 minutes: gone.
    sqlx::query("update pending_signups set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, info(Some(&cookie))).await,
        StatusCode::NOT_FOUND
    );

    // Used for a sign-up: gone.
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&cookie)
                .json(signup_body("Octo", "US")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        status(&app, info(Some(&cookie))).await,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_up_checks_the_name_the_cookie_and_the_body(db: PgPool) {
    let app = test_app(db).await;
    let signup = |cookie: Option<&str>, body: Value| {
        let mut r = req(Method::POST, "/v1/auth/signup").json(body);
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        r
    };
    assert_eq!(
        status(&app, signup(None, signup_body("Octo", "US"))).await,
        StatusCode::UNAUTHORIZED
    );
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    let long = "x".repeat(41);
    for bad in ["", "   ", long.as_str(), "Bad\u{7}Name", "Two\nLines"] {
        let res = app
            .send(signup(Some(&cookie), signup_body(bad, "US")))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad:?}");
        assert_eq!(body_json(res).await["error"]["field"], "display_name");
    }
    let mut extra = signup_body("Octo", "US");
    extra["is_admin"] = json!(true);
    let res = app.send(signup(Some(&cookie), extra)).await;
    assert!(res.status().is_client_error(), "{:?}", res.status());
    assert_eq!(count(&app, "select count(*) from users").await, 0);

    // A pending sign-up older than 30 minutes can't be finished.
    sqlx::query("update pending_signups set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, signup(Some(&cookie), signup_body("Octo", "US"))).await,
        StatusCode::UNAUTHORIZED
    );

    // Forty characters (not bytes) is fine.
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    let res = app
        .send(signup(Some(&cookie), signup_body(&"é".repeat(40), "US")))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_up_records_age_terms_and_the_under_18_defaults(db: PgPool) {
    let app = test_app(db).await;
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&cookie)
                .json(signup_body("Gwen", "gb")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert!(
        removes_cookie(&res, "slop_signup"),
        "the sign-up cookie goes"
    );
    let session = set_cookie(&res, "slop_session").unwrap();
    let body = body_json(res).await;
    // The UK under-18 view: recommendations off and the profile private by default (PRIV-6).
    assert_eq!(body["user"]["country_code"], "GB");
    assert_eq!(body["user"]["personalize"], false);
    assert_eq!(body["user"]["profile_public"], false);
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(me["personalize"], false);
    assert_eq!(me["profile_public"], false);

    let id: Uuid = body["user"]["id"].as_str().unwrap().parse().unwrap();
    let (age, version, current): (bool, String, String) = sqlx::query_as(
        "select u.age_16_confirmed_at is not null, a.version,
                (select version from legal_documents where kind = 'terms' and effective_at <= now()
                 order by effective_at desc, version desc limit 1)
         from users u join legal_acceptances a on a.user_id = u.id and a.kind = 'terms' where u.id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(age);
    assert_eq!(version, current, "the version in effect");

    // Elsewhere they're on.
    sqlx::query("delete from identities")
        .execute(&app.db)
        .await
        .unwrap();
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    let body = body_json(
        app.send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&cookie)
                .json(signup_body("Una", "US")),
        )
        .await,
    )
    .await;
    assert_eq!(body["user"]["personalize"], true);
    assert_eq!(body["user"]["profile_public"], true);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_sign_in_that_got_an_account_meanwhile_isnt_signed_up_twice(db: PgPool) {
    let app = test_app(db).await;
    let res = oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
    let cookie = set_cookie(&res, "slop_signup").unwrap();
    // Meanwhile the same GitHub account finished signing up in another tab.
    let (other, _) = app.user("Octo", "US").await;
    identity(&app, other, "github", "42").await;
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&cookie)
                .json(signup_body("Octo Again", "US")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(set_cookie(&res, "slop_session").is_none());
    assert_eq!(
        count(&app, "select count(*) from users").await,
        1,
        "no account without a sign-in"
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from users where display_name = 'Octo Again'"
        )
        .await,
        0
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signing_out_works_signed_out_on_the_desktop_and_while_suspended(db: PgPool) {
    let app = test_app(db).await;
    // Signed out already: nothing to do, and a stale cookie is cleared.
    let res = app.send(req(Method::POST, "/v1/auth/signout")).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = app
        .send(req(Method::POST, "/v1/auth/signout").cookie("slop_session=stale"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(removes_cookie(&res, "slop_session"));

    // The desktop app signs out with its bearer token: that device's session ends.
    let (user, web) = app.user("Octo", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    let res = app
        .send(req(Method::POST, "/v1/auth/signout").header("authorization", bearer.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/me").header("authorization", bearer)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&web)).await,
        StatusCode::OK,
        "the website stays signed in"
    );

    // Another site can't sign the visitor out.
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signout")
                .cookie(&web)
                .header("sec-fetch-site", "cross-site"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&web)).await,
        StatusCode::OK
    );

    // Suspended accounts can sign out.
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, req(Method::POST, "/v1/auth/signout").cookie(&web)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&web)).await,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------------------------
// Account settings

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn account_settings_need_a_signed_in_unsuspended_player(db: PgPool) {
    let app = test_app(db).await;
    let (user, cookie) = app.user("Pat", "US").await;
    let patch = |body: Value| req(Method::PATCH, "/v1/me").json(body);
    assert_eq!(
        status(&app, patch(json!({ "share_playtime": false }))).await,
        StatusCode::UNAUTHORIZED
    );
    let settings = || async {
        sqlx::query_as::<_, (bool, Option<bool>, Option<bool>)>(
            "select share_playtime, personalize, profile_public from users where id = $1",
        )
        .bind(user)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    // An empty change changes nothing.
    assert_eq!(
        status(&app, patch(json!({})).cookie(&cookie)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(settings().await, (true, None, None));
    assert!(
        status(&app, patch(json!({ "is_admin": true })).cookie(&cookie))
            .await
            .is_client_error()
    );

    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(patch(json!({ "share_playtime": false })).cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "suspended");
    assert_eq!(settings().await, (true, None, None));
}

/// b07: in the UK under-18 view, recommendations and the public profile start off (PRIV-6). It's a
/// default: the person may still switch them on themselves, and their choice is what's read.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn under_18_view_accounts_start_private_and_may_choose_otherwise(db: PgPool) {
    let app = test_app(db).await;
    let (_, cookie) = app.user("Gwen", "GB").await;
    let me = || async {
        let me = body_json(
            app.send(
                req(Method::GET, "/v1/me")
                    .cookie(&cookie)
                    .header("x-country", "GB"),
            )
            .await,
        )
        .await;
        (me["personalize"].clone(), me["profile_public"].clone())
    };
    assert_eq!(me().await, (json!(false), json!(false)));
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&cookie)
                .json(json!({ "personalize": true, "profile_public": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(me().await, (json!(true), json!(true)));
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&cookie)
                .json(json!({ "profile_public": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(me().await, (json!(true), json!(false)));
}

// ---------------------------------------------------------------------------------------------
// Sessions and step-up

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn website_sessions_end_thirty_days_after_sign_in_and_are_touched_sparingly(db: PgPool) {
    let app = test_app(db).await;
    let (user, cookie) = app.user("Octo", "US").await;
    let days: f64 = sqlx::query_scalar(
        "select (extract(epoch from expires_at - created_at) / 86400)::float8 from sessions where user_id = $1",
    )
    .bind(user)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((days - 30.0).abs() < 0.01, "{days}");
    let me = || status(&app, req(Method::GET, "/v1/me").cookie(&cookie));
    let seen_now = || async {
        sqlx::query_scalar::<_, bool>(
            "select last_seen_at > now() - interval '1 minute' from sessions where user_id = $1",
        )
        .bind(user)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };

    // Seen two minutes ago: not written again yet. Six minutes ago: written.
    sqlx::query(
        "update sessions set last_seen_at = now() - interval '2 minutes' where user_id = $1",
    )
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(me().await, StatusCode::OK);
    assert!(!seen_now().await);
    sqlx::query(
        "update sessions set last_seen_at = now() - interval '6 minutes' where user_id = $1",
    )
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(me().await, StatusCode::OK);
    assert!(seen_now().await);

    // Used every day, but 30 days after signing in it ends anyway.
    sqlx::query("update sessions set expires_at = now() - interval '1 second' where user_id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(me().await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_closed_accounts_sessions_stop_working_at_once(db: PgPool) {
    let app = test_app(db).await;
    let (user, web) = app.user("Octo", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    // Closed some other way than the close route (staff, a merge): the sessions are still there.
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        count(
            &app,
            "select count(*) from sessions where revoked_at is null"
        )
        .await,
        2
    );
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&web)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/me").header("authorization", bearer)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_sessions_last_longer_and_their_access_tokens_expire_quickly(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    let me = |b: &str| req(Method::GET, "/v1/me").header("authorization", b.to_string());
    let days: f64 = sqlx::query_scalar(
        "select (extract(epoch from expires_at - created_at) / 86400)::float8 from sessions where kind = 'desktop'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((days - 180.0).abs() < 0.01, "{days}");

    // Unused for 20 days: a website session would have ended, the desktop one hasn't.
    sqlx::query("update sessions set last_seen_at = now() - interval '20 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(status(&app, me(&bearer)).await, StatusCode::OK);
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&web)).await,
        StatusCode::UNAUTHORIZED
    );

    // An access token lasts 15 minutes; the refresh token still gets a new one.
    sqlx::query("update access_tokens set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(status(&app, me(&bearer)).await, StatusCode::UNAUTHORIZED);
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let rotated = body_json(res).await;
    let fresh = format!("Bearer {}", rotated["access_token"].as_str().unwrap());
    assert_eq!(status(&app, me(&fresh)).await, StatusCode::OK);

    // Unused for 31 days: ended, refresh token and all.
    sqlx::query(
        "update sessions set last_seen_at = now() - interval '31 days' where kind = 'desktop'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(status(&app, me(&fresh)).await, StatusCode::UNAUTHORIZED);
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": rotated["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_empty_bearer_token_is_no_token(db: PgPool) {
    let app = test_app(db).await;
    let (_, cookie) = app.user("Octo", "US").await;
    for header_value in ["Bearer ", "Bearer    ", "Basic b2N0bzpwdw=="] {
        assert_eq!(
            status(
                &app,
                req(Method::GET, "/v1/me").header("authorization", header_value)
            )
            .await,
            StatusCode::UNAUTHORIZED,
            "{header_value:?}"
        );
        assert_eq!(
            status(
                &app,
                req(Method::GET, "/v1/me")
                    .header("authorization", header_value)
                    .cookie(&cookie)
            )
            .await,
            StatusCode::OK,
            "the cookie is used: {header_value:?}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn suspended_accounts_may_only_read_appeal_accept_sign_out_and_leave(db: PgPool) {
    use slop_api::auth::session::allowed_while_suspended as allowed;
    for (method, path) in [
        (Method::GET, "/v1/apps"),
        (Method::HEAD, "/v1/me"),
        (Method::POST, "/v1/auth/signout"),
        (Method::POST, "/v1/me/close"),
        (Method::POST, "/v1/me/legal/accept"),
        (
            Method::DELETE,
            "/v1/me/sessions/0190a1b2-0000-7000-8000-000000000000",
        ),
        (Method::POST, "/v1/me/decisions/abc/appeal"),
        (Method::POST, "/me/close"),
    ] {
        assert!(allowed(&method, path), "{method} {path}");
    }
    for (method, path) in [
        (Method::POST, "/v1/apps"),
        (Method::PATCH, "/v1/me"),
        (Method::POST, "/v1/me/decisions/abc"),
        (Method::DELETE, "/v1/me/sessions"),
        (Method::DELETE, "/v1/me/passkeys/abc"),
        (Method::POST, "/v1/auth/signout/extra"),
        (Method::POST, "/v2/me/close"),
    ] {
        assert!(!allowed(&method, path), "{method} {path}");
    }

    // End to end: a suspended account signs out its other devices.
    let app = test_app(db).await;
    let (user, cookie) = app.user("Sus", "US").await;
    let other = slop_api::auth::session::create(&app.db, &app.config, user, Some("phone"))
        .await
        .unwrap();
    let other = format!("{}={}", other.name(), other.value());
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    let phone: Uuid = sqlx::query_scalar("select id from sessions where user_agent = 'phone'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(
            &app,
            req(Method::DELETE, format!("/v1/me/sessions/{phone}")).cookie(&cookie)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&other)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_recent_sign_in_means_the_last_ten_minutes(db: PgPool) {
    let app = test_app(db).await;
    let (user, cookie) = app.user("Octo", "US").await;
    let set = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query(sql).bind(user).execute(&db).await.unwrap();
        }
    };
    // Adding a passkey asks for a recent sign-in (and the account has none to confirm with).
    let probe = || async {
        let res = app
            .send(req(Method::POST, "/v1/me/passkeys/register/start").cookie(&cookie))
            .await;
        let status = res.status();
        let code = body_json(res).await["error"]["code"].clone();
        (status, code)
    };
    set("update sessions set created_at = now() - interval '9 minutes 50 seconds' where user_id = $1").await;
    assert_eq!(probe().await.0, StatusCode::OK);
    set("update sessions set created_at = now() - interval '10 minutes 10 seconds' where user_id = $1").await;
    assert_eq!(
        probe().await,
        (StatusCode::FORBIDDEN, json!("step_up_required"))
    );
    // A confirmation counts for the same ten minutes.
    set("update sessions set stepped_up_at = now() - interval '9 minutes 50 seconds' where user_id = $1").await;
    assert_eq!(probe().await.0, StatusCode::OK);
    set("update sessions set stepped_up_at = now() - interval '10 minutes 10 seconds' where user_id = $1").await;
    assert_eq!(
        probe().await,
        (StatusCode::FORBIDDEN, json!("step_up_required"))
    );

    // Other places that ask for it refuse the old session too.
    assert_eq!(
        error_code(
            &app,
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&cookie)
                .json(json!({ "name": "Claude", "scope": "read" }))
        )
        .await,
        "step_up_required"
    );
    assert_eq!(
        error_code(
            &app,
            req(
                Method::POST,
                format!("/v1/chat/devices/{}/start-over", Uuid::now_v7())
            )
            .cookie(&cookie)
        )
        .await,
        "step_up_required"
    );
    assert_eq!(
        count(&app, "select count(*) from sessions where kind = 'agent'").await,
        0
    );
}

// ---------------------------------------------------------------------------------------------
// Combining accounts

/// The signed-in `cookie` links the Google account `code` (which has its own Slop account).
/// Returns the pending combination's cookie.
async fn start_combining(app: &TestApp, cookie: &str, code: &str) -> String {
    let res = oauth_flow(app, "/v1/auth/google/link", &[cookie], code).await;
    assert_eq!(location(&res), "/account/combine");
    set_cookie(&res, "slop_combine").unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_is_shown_only_to_the_session_that_linked_while_it_lasts(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    let preview = |cookies: &[&str]| {
        let mut r = req(Method::GET, "/v1/auth/combine");
        for c in cookies {
            r = r.cookie(*c);
        }
        r
    };
    assert_eq!(
        status(&app, preview(&[&ann])).await,
        StatusCode::NOT_FOUND,
        "nothing pending"
    );
    let combine = start_combining(&app, &ann, "g-bob").await;
    let shown = body_json(app.send(preview(&[&ann, &combine])).await).await;
    assert_eq!(shown["name"], "Bob");
    assert_eq!(shown["provider"], "google");

    // Not someone else, and not Ann in another browser.
    let (_, eve) = app.user("Eve", "US").await;
    assert_eq!(
        status(&app, preview(&[&eve, &combine])).await,
        StatusCode::NOT_FOUND
    );
    let other = slop_api::auth::session::create(&app.db, &app.config, ann_id, None)
        .await
        .unwrap();
    let other = format!("{}={}", other.name(), other.value());
    assert_eq!(
        status(&app, preview(&[&other, &combine])).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, preview(&[&combine])).await,
        StatusCode::UNAUTHORIZED
    );

    // After 15 minutes it's gone, and can't be confirmed.
    sqlx::query("update pending_merges set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, preview(&[&ann, &combine])).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(
            &app,
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let closed: bool = sqlx::query_scalar("select closed_at is not null from users where id = $1")
        .bind(bob_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!closed);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_waits_for_one_people_page_one_stripe_account_and_isnt_for_staff_or_closed(
    db: PgPool,
) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    // Ann has a People page and receives donations through Stripe.
    let page = |user: Uuid, handle: &str| {
        let db = app.db.clone();
        let handle = handle.to_string();
        async move {
            sqlx::query(
                "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, $2, $2, 'none', false)",
            )
            .bind(user)
            .bind(handle)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    let stripe = |user: Uuid, account: &str| {
        let db = app.db.clone();
        let account = account.to_string();
        async move {
            sqlx::query(
                "insert into donation_accounts (user_id, stripe_account_id, tax_notice_ack_at) values ($1, $2, now())",
            )
            .bind(user)
            .bind(account)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    page(ann_id, "ann-art").await;
    stripe(ann_id, "acct_ann").await;

    let (bob, _) = app.user("Bob", "US").await;
    page(bob, "bob-art").await;
    let (cara, _) = app.user("Cara", "US").await;
    stripe(cara, "acct_cara").await;
    let (dan, _) = make_admin(&app, "Dan Staff").await;
    let (eve, _) = app.user("Eve", "US").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(eve)
        .execute(&app.db)
        .await
        .unwrap();
    for (user, code, why) in [
        (bob, "g-bob", "both accounts have a People page"),
        (
            cara,
            "g-cara",
            "both accounts receive donations through Stripe",
        ),
        (dan, "g-dan", "staff accounts can't be combined"),
        (eve, "g-eve", "that account is closed"),
    ] {
        identity(&app, user, "google", code).await;
        let combine = start_combining(&app, &ann, code).await;
        let shown = body_json(
            app.send(
                req(Method::GET, "/v1/auth/combine")
                    .cookie(&ann)
                    .cookie(&combine),
            )
            .await,
        )
        .await;
        assert!(
            shown["blocked"].as_str().unwrap_or("").starts_with(why),
            "{code}: {shown}"
        );
        let res = app
            .send(
                req(Method::POST, "/v1/auth/combine")
                    .cookie(&ann)
                    .cookie(&combine),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "{code}");
        let merged: Option<Uuid> =
            sqlx::query_scalar("select merged_into from users where id = $1")
                .bind(user)
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(merged, None, "{code}");
    }
    let pages: Vec<(Uuid, String)> =
        sqlx::query_as("select user_id, handle from creatives order by handle")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        pages,
        [
            (ann_id, "ann-art".to_string()),
            (bob, "bob-art".to_string())
        ]
    );
    let dan_admin: bool = sqlx::query_scalar("select is_admin from users where id = $1")
        .bind(dan)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(dan_admin);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_moves_pages_chats_requests_and_blocks_and_keeps_the_earlier_deadline(
    db: PgPool,
) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    let (cara_id, _) = app.user("Cara", "US").await;
    let (dave_id, _) = app.user("Dave", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    stored_passkey(&app, ann_id).await;
    let exec = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query(sql)
                .bind(ann_id)
                .bind(bob_id)
                .bind(cara_id)
                .bind(dave_id)
                .execute(&db)
                .await
                .unwrap();
        }
    };
    let pair = |a: Uuid, b: Uuid| {
        let (a, b) = (a.to_string(), b.to_string());
        if a < b {
            format!("{a}:{b}")
        } else {
            format!("{b}:{a}")
        }
    };
    // Bob's People page; Bob's 18+ confirmation; the earlier passkey deadline is Bob's.
    exec("insert into creatives (user_id, handle, name, ai_use, trader, skills) select $2, 'bob-art', 'Bob Art', 'none', false, '{pixel-art}' where $1 is not null and $3 is not null and $4 is not null").await;
    exec("update users set adult_confirmed_at = now() - interval '1 day', passkey_due_at = now() + interval '5 days' where id = $2 and $1 is not null and $3 is not null and $4 is not null").await;
    exec("update users set passkey_due_at = now() + interval '20 days' where id = $1 and $2 is not null and $3 is not null and $4 is not null").await;
    // Friend requests (one between Ann and Bob, which goes), and a block.
    exec("insert into friend_requests (from_id, to_id) values ($2, $3), ($4, $2), ($1, $2)").await;
    exec("insert into user_blocks (user_id, blocked_id) select $2, $4 where $1 is not null and $3 is not null").await;
    // Direct chats: both with Cara (Ann's stays), Bob with Dave (becomes Ann's), Ann with Bob.
    for (a, b) in [
        (ann_id, cara_id),
        (bob_id, cara_id),
        (bob_id, dave_id),
        (ann_id, bob_id),
    ] {
        sqlx::query(
            "insert into chat_conversations (id, kind, pair_key) values ($1, 'direct', $2)",
        )
        .bind(Uuid::now_v7())
        .bind(pair(a, b))
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Both have one game in their library; Bob has another too.
    listed_with(&app, cara_id, "shared-game", "released", "").await;
    listed_with(&app, cara_id, "bobs-pick", "released", "").await;
    exec("insert into library_entries (user_id, app_id) select u, a.id from apps a, unnest(array[$1, $2]) u where a.slug = 'shared-game' and $3 is not null and $4 is not null").await;
    exec("insert into library_entries (user_id, app_id) select $2, id from apps where slug = 'bobs-pick' and $1 is not null and $3 is not null and $4 is not null").await;

    // Linking needs Ann's passkey; then she must still confirm with it to combine.
    sessions_now(&app, ann_id, "passkey_at").await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    let confirm = || {
        req(Method::POST, "/v1/auth/combine")
            .cookie(&ann)
            .cookie(&combine)
    };
    sqlx::query("update sessions set passkey_at = null where user_id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(error_code(&app, confirm()).await, "step_up_required");
    sessions_now(&app, ann_id, "passkey_at").await;
    let res = app.send(confirm()).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(removes_cookie(&res, "slop_combine"));

    // The People page is Ann's now, under the same handle.
    let pages: Vec<(Uuid, String, Vec<String>)> =
        sqlx::query_as("select user_id, handle, skills from creatives")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        pages,
        [(ann_id, "bob-art".to_string(), vec!["pixel-art".to_string()])]
    );
    // Requests and blocks name Ann; the one between them is gone.
    let requests: Vec<(Uuid, Uuid)> =
        sqlx::query_as("select from_id, to_id from friend_requests order by created_at, from_id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    let mut requests = requests;
    requests.sort();
    let mut expected = vec![(ann_id, cara_id), (dave_id, ann_id)];
    expected.sort();
    assert_eq!(requests, expected);
    let blocks: Vec<(Uuid, Uuid)> = sqlx::query_as("select user_id, blocked_id from user_blocks")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(blocks, [(ann_id, dave_id)]);
    // Chats: Ann's with Cara stays, Bob's with Dave is Ann's, and none names Bob.
    let mut keys: Vec<String> = sqlx::query_scalar("select pair_key from chat_conversations")
        .fetch_all(&app.db)
        .await
        .unwrap();
    keys.sort();
    let mut expected = vec![pair(ann_id, cara_id), pair(ann_id, dave_id)];
    expected.sort();
    assert_eq!(keys, expected);
    // The library: no duplicate, and Bob's other game came along.
    let library: Vec<String> = sqlx::query_scalar(
        "select a.slug from library_entries l join apps a on a.id = l.app_id where l.user_id = $1 order by a.slug",
    )
    .bind(ann_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(library, ["bobs-pick", "shared-game"]);
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from library_entries where user_id = '{bob_id}'")
        )
        .await,
        0
    );
    // The earlier deadline and the 18+ confirmation carry over.
    let (adult, due_days): (bool, f64) = sqlx::query_as(
        "select adult_confirmed_at is not null, (extract(epoch from passkey_due_at - now()) / 86400)::float8
         from users where id = $1",
    )
    .bind(ann_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(adult);
    assert!((4.9..5.1).contains(&due_days), "{due_days}");
    // Bob's sessions ended, the pending combination is gone, and it's in the audit log.
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&bob)).await,
        StatusCode::UNAUTHORIZED
    );
    let live: i64 = sqlx::query_scalar(
        "select count(*) from sessions where token_hash = $1 and revoked_at is null",
    )
    .bind(slop_api::tokens::hash_token(bob.split_once('=').unwrap().1))
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(live, 0);
    assert_eq!(count(&app, "select count(*) from pending_merges").await, 0);
    let audited: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'account.combine' and subject = $1",
    )
    .bind(ann_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(audited["from"], json!(bob_id));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_account_with_passkeys_isnt_combined_into_another(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (_, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    stored_passkey(&app, bob_id).await;
    listed_with(&app, bob_id, "bobs-game", "released", "").await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    let res = app
        .send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let maker: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'bobs-game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(maker, bob_id, "nothing moved");
    let (closed, passkeys): (bool, i64) = sqlx::query_as(
        "select closed_at is not null, (select count(*) from passkeys where user_id = $1) from users where id = $1",
    )
    .bind(bob_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((closed, passkeys), (false, 1));
}

/// Confirming twice (a double click, or the page sent again) does nothing the second time. It
/// used to find the pending combination still there, now naming Ann on both sides (combining
/// had moved it along with everything else), and close Ann's own account into itself.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn confirming_a_combination_twice_does_nothing_the_second_time(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    let confirm = || {
        req(Method::POST, "/v1/auth/combine")
            .cookie(&ann)
            .cookie(&combine)
    };
    assert_eq!(status(&app, confirm()).await, StatusCode::NO_CONTENT);
    assert_eq!(count(&app, "select count(*) from pending_merges").await, 0);
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(status(&app, confirm()).await, StatusCode::NOT_FOUND);
    let (closed, merged): (bool, Option<Uuid>) =
        sqlx::query_as("select closed_at is not null, merged_into from users where id = $1")
            .bind(ann_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((closed, merged), (false, None), "Ann's account stays open");
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&ann)).await,
        StatusCode::OK
    );
}

/// b04: confirming doesn't ask for a sign-in in the last ten minutes. It doesn't need to: the
/// pending combination only comes from a link made with a fresh sign-in, belongs to that one
/// session, and lasts 15 minutes.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_is_confirmed_within_the_links_fifteen_minutes(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    // Ten minutes on, the sign-in isn't recent any more, but the link was.
    age_sessions(&app, ann_id).await;
    sqlx::query("update pending_merges set expires_at = now() + interval '4 minutes'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let merged: Option<Uuid> = sqlx::query_scalar("select merged_into from users where id = $1")
        .bind(bob_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(merged, Some(ann_id));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cancelling_a_combination_forgets_it(db: PgPool) {
    let (app, _) = app_with_providers(db).await;
    let (_, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, bob_id, "google", "g-bob").await;
    // Nothing to cancel: fine.
    let res = app.send(req(Method::DELETE, "/v1/auth/combine")).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let combine = start_combining(&app, &ann, "g-bob").await;
    assert_eq!(count(&app, "select count(*) from pending_merges").await, 1);
    let res = app
        .send(req(Method::DELETE, "/v1/auth/combine").cookie(&combine))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(removes_cookie(&res, "slop_combine"));
    assert_eq!(count(&app, "select count(*) from pending_merges").await, 0);
    assert_eq!(
        status(
            &app,
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let merged: Option<Uuid> = sqlx::query_scalar("select merged_into from users where id = $1")
        .bind(bob_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(merged, None);
}

// ---------------------------------------------------------------------------------------------
// Desktop sign-in

fn desktop_request(state: &str, device: &str) -> Value {
    json!({
        "code_challenge": pkce_challenge(&"v".repeat(43)),
        "redirect_port": 49152,
        "state": state,
        "device_name": device
    })
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_sign_in_requests_are_checked_up_front(db: PgPool) {
    let app = test_app(db).await;
    let create = |body: Value| req(Method::POST, "/v1/auth/desktop/requests").json(body);
    let field = |body: Value| {
        let app = &app;
        async move {
            let res = app.send(create(body)).await;
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
            body_json(res).await["error"]["field"].clone()
        }
    };
    let long_state = "s".repeat(129);
    for bad in [
        "s".repeat(15),
        long_state,
        "has spaces in it ok".into(),
        "plus+and/slash==ab".into(),
    ] {
        assert_eq!(
            field(desktop_request(&bad, "Laptop")).await,
            "state",
            "{bad}"
        );
    }
    let long_name = "n".repeat(41);
    for bad in ["", "   ", long_name.as_str(), "Two\nLines"] {
        assert_eq!(
            field(desktop_request("state-state-state-1", bad)).await,
            "device_name",
            "{bad:?}"
        );
    }
    let mut extra = desktop_request("state-state-state-1", "Laptop");
    extra["approved_by"] = json!(Uuid::now_v7());
    assert!(status(&app, create(extra)).await.is_client_error());
    assert_eq!(
        count(&app, "select count(*) from desktop_auth_requests").await,
        0
    );

    // The edges are fine.
    for ok in ["s".repeat(16), "s".repeat(128), "Az09-_".repeat(3)] {
        assert_eq!(
            status(&app, create(desktop_request(&ok, &"n".repeat(40)))).await,
            StatusCode::OK,
            "{ok}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_sign_in_requests_are_rate_limited_per_client(db: PgPool) {
    let app = test_app(db).await;
    let from = |ip: &str| {
        req(Method::POST, "/v1/auth/desktop/requests")
            .header("x-test-client-ip", ip.to_string())
            .json(desktop_request("state-state-state-1", "Laptop"))
    };
    for _ in 0..20 {
        assert_eq!(status(&app, from("203.0.113.9")).await, StatusCode::OK);
    }
    let res = app.send(from("203.0.113.9")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    assert_eq!(status(&app, from("203.0.113.10")).await, StatusCode::OK);
    // Starting a passkey sign-in counts as signing in too.
    for _ in 0..20 {
        assert_eq!(
            status(
                &app,
                req(Method::POST, "/v1/auth/passkey/start")
                    .header("x-test-client-ip", "203.0.113.11")
            )
            .await,
            StatusCode::OK
        );
    }
    assert_eq!(
        status(
            &app,
            req(Method::POST, "/v1/auth/passkey/start").header("x-test-client-ip", "203.0.113.11")
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_approval_page_shows_only_pending_requests_to_signed_in_people(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    let create = || async {
        let res = app
            .send(
                req(Method::POST, "/v1/auth/desktop/requests")
                    .json(desktop_request("state-state-state-1", "Slop for Linux")),
            )
            .await;
        body_json(res).await["id"].as_str().unwrap().to_string()
    };
    let show = |id: &str| req(Method::GET, format!("/v1/auth/desktop/requests/{id}"));
    let id = create().await;
    assert_eq!(status(&app, show(&id)).await, StatusCode::UNAUTHORIZED);
    let res = app.send(show(&id).cookie(&web)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["device_name"], "Slop for Linux");
    assert_eq!(
        status(&app, show(&Uuid::now_v7().to_string()).cookie(&web)).await,
        StatusCode::NOT_FOUND
    );

    // Approved: not shown again.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve"),
            )
            .cookie(&web),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        status(&app, show(&id).cookie(&web)).await,
        StatusCode::NOT_FOUND
    );

    // Expired: not shown, and can't be approved.
    let id = create().await;
    sqlx::query("update desktop_auth_requests set expires_at = now() - interval '1 second' where id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, show(&id).cookie(&web)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(
            &app,
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve")
            )
            .cookie(&web)
        )
        .await,
        StatusCode::NOT_FOUND
    );
}
