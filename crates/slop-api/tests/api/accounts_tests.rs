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
async fn linking_works_from_an_old_session_even_with_a_passkey(db: PgPool) {
    // No "confirm it's you" before linking since 2026-10-25 (SECURITY §4.3): an old session of
    // an account with a passkey goes straight on to the provider.
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    stored_passkey(&app, ann_id).await;
    age_sessions(&app, ann_id).await;
    sqlx::query("update sessions set created_at = now() - interval '30 days' where user_id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    let link = |path: &str| req(Method::GET, path.to_string()).cookie(&ann);

    // On to GitLab, tied to this account and session.
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

    // Signed out, it's sign in first, then straight back here.
    let res = app
        .send(req(
            Method::GET,
            "/v1/auth/gitlab/link?return_to=%2Faccount%3Fpicture%3Drefresh%23picture",
        ))
        .await;
    assert_eq!(
        location(&res),
        "/signin?reason=link&return_to=%2Fv1%2Fauth%2Fgitlab%2Flink%3Freturn_to%3D%252Faccount%253Fpicture%253Drefresh%2523picture"
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
async fn unlinking_works_from_an_old_session_and_only_touches_your_own_sign_ins(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, ann_id, "google", "g-ann").await;
    identity(&app, ann_id, "gitlab", "501").await;
    identity(&app, bob_id, "gitlab", "502").await;
    let unlink = |path: &str| req(Method::DELETE, format!("/v1/me/identities/{path}")).cookie(&ann);
    let identities = || count(&app, "select count(*) from identities");

    // An old session, of an account with a passkey: no "confirm it's you" (SECURITY §4.3).
    age_sessions(&app, ann_id).await;
    stored_passkey(&app, ann_id).await;

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
async fn an_old_session_is_never_asked_for_a_recent_sign_in(db: PgPool) {
    // The owner's decision of 2026-10-25 (SECURITY §4.3): a signed-in session is trusted. This
    // one signed in 30 days ago (and was used just now, so it hasn't timed out).
    let app = test_app(db).await;
    let (user, cookie) = app.user("Octo", "US").await;
    sqlx::query(
        "update sessions set created_at = now() - interval '30 days', stepped_up_at = null, passkey_at = null where user_id = $1",
    )
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
    let code = |res: axum::response::Response| async move {
        let status = res.status();
        (status, body_json(res).await["error"]["code"].clone())
    };

    // Adding a passkey (with one already on the account too: main.rs
    // `passkeys_are_added_from_any_session_and_still_confirm_staff_tokens`).
    let res = app
        .send(req(Method::POST, "/v1/me/passkeys/register/start").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // A signing key (refused only for not being a real key) and chat start-over (no such device).
    let (status, err) = code(
        app.send(
            req(Method::POST, "/v1/me/signing-keys")
                .cookie(&cookie)
                .json(json!({ "name": "Laptop", "public_key": "ssh-ed25519 AAAA x@y" })),
        )
        .await,
    )
    .await;
    assert_ne!(err, json!("step_up_required"), "{status}");
    let (status, err) = code(
        app.send(
            req(
                Method::POST,
                format!("/v1/chat/devices/{}/start-over", Uuid::now_v7()),
            )
            .cookie(&cookie),
        )
        .await,
    )
    .await;
    assert_ne!(err, json!("step_up_required"), "{status}");
    // Making an AI agent token doesn't: a signed-in session is enough (maker_agents.rs).
    assert_eq!(
        count(&app, "select count(*) from sessions where kind = 'agent'").await,
        0
    );
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&cookie)
                .json(json!({ "name": "Claude", "scope": "read" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        count(&app, "select count(*) from sessions where kind = 'agent'").await,
        1
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
async fn combining_moves_pages_chats_requests_and_blocks_with_no_passkey_asked(db: PgPool) {
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
    // Bob's People page; Bob's 18+ confirmation.
    exec("insert into creatives (user_id, handle, name, ai_use, trader, skills) select $2, 'bob-art', 'Bob Art', 'none', false, '{pixel-art}' where $1 is not null and $3 is not null and $4 is not null").await;
    exec("update users set adult_confirmed_at = now() - interval '1 day' where id = $2 and $1 is not null and $3 is not null and $4 is not null").await;
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

    // Ann has a passkey and an old session: neither linking nor combining asks her to confirm
    // with it (SECURITY §4.3, 2026-10-25).
    age_sessions(&app, ann_id).await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    let confirm = || {
        req(Method::POST, "/v1/auth/combine")
            .cookie(&ann)
            .cookie(&combine)
    };
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
    // The 18+ confirmation carries over.
    let adult: bool =
        sqlx::query_scalar("select adult_confirmed_at is not null from users where id = $1")
            .bind(ann_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(adult);
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
async fn an_account_with_passkeys_is_combined_like_any_other(db: PgPool) {
    // Until 2026-10-25 it could only be combined from itself, signed in with a passkey. Signing in
    // to it is proof enough now (SECURITY §4.3): its apps, and its passkeys, like its sign-ins,
    // open the combined account.
    let (app, _) = app_with_providers(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
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
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let maker: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'bobs-game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(maker, ann_id);
    let (closed, passkeys): (bool, i64) = sqlx::query_as(
        "select closed_at is not null, (select count(*) from passkeys where user_id = $2) from users where id = $1",
    )
    .bind(bob_id)
    .bind(ann_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((closed, passkeys), (true, 1));
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

// ---------------------------------------------------------------------------------------------
// Passkeys

type Authenticator = webauthn_authenticator_rs::WebauthnAuthenticator<
    webauthn_authenticator_rs::softpasskey::SoftPasskey,
>;

fn authenticator() -> Authenticator {
    webauthn_authenticator_rs::WebauthnAuthenticator::new(
        webauthn_authenticator_rs::softpasskey::SoftPasskey::new(true),
    )
}

fn origin() -> url::Url {
    "http://localhost:8080".parse().unwrap()
}

/// Starts adding a passkey and makes the credential. Returns (challenge id, credential JSON).
async fn new_credential(app: &TestApp, cookie: &str, auth: &mut Authenticator) -> (Value, Value) {
    let res = app
        .send(req(Method::POST, "/v1/me/passkeys/register/start").cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let started = body_json(res).await;
    let options = serde_json::from_value(started["options"].clone()).unwrap();
    let credential = auth.do_registration(origin(), options).unwrap();
    (
        started["challenge_id"].clone(),
        serde_json::to_value(credential).unwrap(),
    )
}

/// Adds a passkey to the signed-in account. Returns (its id, its credential ID as sent).
async fn add_passkey(app: &TestApp, cookie: &str, auth: &mut Authenticator) -> (Uuid, String) {
    let (challenge, credential) = new_credential(app, cookie, auth).await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/passkeys/register/finish")
                .cookie(cookie)
                .json(json!({ "challenge_id": challenge, "name": "Laptop", "credential": credential })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    (id, credential["id"].as_str().unwrap().to_string())
}

/// Starts a passkey sign-in and signs it with `cred_id`. Returns (challenge id, assertion JSON).
/// SoftPasskey can't pick a credential by itself, so the request names the one to use.
async fn passkey_assertion(
    app: &TestApp,
    auth: &mut Authenticator,
    cred_id: &str,
) -> (Value, Value) {
    let started = body_json(app.send(req(Method::POST, "/v1/auth/passkey/start")).await).await;
    let mut options = started["options"].clone();
    options["publicKey"]["allowCredentials"] = json!([{ "type": "public-key", "id": cred_id }]);
    let assertion = auth
        .do_authentication(origin(), serde_json::from_value(options).unwrap())
        .unwrap();
    (
        started["challenge_id"].clone(),
        serde_json::to_value(assertion).unwrap(),
    )
}

fn passkey_finish(challenge: &Value, credential: &Value) -> Req {
    req(Method::POST, "/v1/auth/passkey/finish")
        .json(json!({ "challenge_id": challenge, "credential": credential }))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn passkeys_are_listed_only_for_their_owner(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, bob) = app.user("Bob", "US").await;
    let id = stored_passkey(&app, ann_id).await;
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me/passkeys")).await,
        StatusCode::UNAUTHORIZED
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/passkeys").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], json!(id));
    assert!(mine[0].get("credential").is_none(), "{mine}");
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/passkeys").cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(theirs, json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_a_passkey_works_from_an_old_session_and_only_for_your_own(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    let laptop = stored_passkey(&app, ann_id).await;
    let bobs = stored_passkey(&app, bob_id).await;
    let remove = |id: Uuid| req(Method::DELETE, format!("/v1/me/passkeys/{id}")).cookie(&ann);
    let passkeys = || count(&app, "select count(*) from passkeys");

    // An old session that never used a passkey may (SECURITY §4.3, 2026-10-25), but only Ann's own.
    age_sessions(&app, ann_id).await;
    assert_eq!(status(&app, remove(bobs)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status(&app, remove(Uuid::now_v7())).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(passkeys().await, 2);
    assert_eq!(status(&app, remove(laptop)).await, StatusCode::NO_CONTENT);
    let left = body_json(
        app.send(req(Method::GET, "/v1/me/passkeys").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(left, json!([]));
    let owner: Uuid = sqlx::query_scalar("select user_id from passkeys")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, bob_id);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn adding_a_passkey_checks_the_name_the_challenge_and_the_credential(db: PgPool) {
    use base64::Engine;
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    let mut auth = authenticator();
    let finish = |cookie: &str, challenge: &Value, name: &str, credential: &Value| {
        req(Method::POST, "/v1/me/passkeys/register/finish")
            .cookie(cookie)
            .json(json!({ "challenge_id": challenge, "name": name, "credential": credential }))
    };

    // A name is needed, of up to 60 characters; the challenge survives a bad name.
    let (challenge, credential) = new_credential(&app, &ann, &mut auth).await;
    for bad in ["", "   ", &"n".repeat(61)] {
        let res = app.send(finish(&ann, &challenge, bad, &credential)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad:?}");
        assert_eq!(body_json(res).await["error"]["field"], "name");
    }
    // Not finished by someone else.
    assert_eq!(
        status(&app, finish(&bob, &challenge, "Laptop", &credential)).await,
        StatusCode::CONFLICT
    );

    // A credential made for one challenge doesn't finish another.
    let (first, credential) = new_credential(&app, &ann, &mut auth).await;
    let (second, _) = new_credential(&app, &ann, &mut auth).await;
    let res = app.send(finish(&ann, &second, "Laptop", &credential)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "credential");
    // Five minutes on, a challenge has expired.
    sqlx::query("update webauthn_challenges set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(&app, finish(&ann, &first, "Laptop", &credential)).await,
        StatusCode::CONFLICT
    );
    assert_eq!(count(&app, "select count(*) from passkeys").await, 0);

    // A passkey already on another account isn't added to this one.
    let (challenge, credential) = new_credential(&app, &bob, &mut auth).await;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(credential["id"].as_str().unwrap())
        .unwrap();
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, '{}', 'Ann''s')",
    )
    .bind(Uuid::now_v7())
    .bind(ann_id)
    .bind(&raw)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(finish(&bob, &challenge, "Mine", &credential))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let bobs: i64 = sqlx::query_scalar("select count(*) from passkeys where user_id = $1")
        .bind(bob_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(bobs, 0);

    // Added: the owner is told, in case it wasn't them.
    let (id, _) = add_passkey(&app, &bob, &mut auth).await;
    let (title, category, body): (String, String, String) =
        sqlx::query_as("select title, category, body from notifications where user_id = $1")
            .bind(bob_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(title, "A passkey was added to your account");
    assert_eq!(category, "security");
    assert!(body.contains("\"Laptop\""), "{body}");
    let listed = body_json(
        app.send(req(Method::GET, "/v1/me/passkeys").cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(listed[0]["id"], json!(id));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signing_in_with_a_passkey_starts_a_session_that_has_used_it(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let mut auth = authenticator();
    let (passkey, cred_id) = add_passkey(&app, &ann, &mut auth).await;
    let counter = || async {
        sqlx::query_scalar::<_, Option<i64>>(
            "select (credential->'cred'->>'counter')::int8 from passkeys where id = $1",
        )
        .bind(passkey)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    let before = counter().await;

    let (challenge, assertion) = passkey_assertion(&app, &mut auth, &cred_id).await;
    let res = app
        .send(passkey_finish(&challenge, &assertion).header("user-agent", "Firefox"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let session = set_cookie(&res, "slop_session").unwrap();
    assert_eq!(body_json(res).await["signed_in"], true);
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(me["id"], json!(ann_id));
    let used: bool =
        sqlx::query_scalar("select last_used_at is not null from passkeys where id = $1")
            .bind(passkey)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(used);
    assert!(counter().await >= before);
    // The same ceremony works once.
    assert_eq!(
        status(&app, passkey_finish(&challenge, &assertion)).await,
        StatusCode::CONFLICT
    );

    // A signature that doesn't check out.
    let (challenge, mut assertion) = passkey_assertion(&app, &mut auth, &cred_id).await;
    let sig = assertion["response"]["signature"]
        .as_str()
        .unwrap()
        .to_string();
    let flipped = if sig.starts_with('A') { "B" } else { "A" };
    assertion["response"]["signature"] = json!(format!("{flipped}{}", &sig[1..]));
    let res = app.send(passkey_finish(&challenge, &assertion)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "credential");

    // A passkey Slop doesn't know.
    let (_, bob) = app.user("Bob", "US").await;
    let mut other = authenticator();
    let (_, unknown) = add_passkey(&app, &bob, &mut other).await;
    sqlx::query("delete from passkeys where credential_id <> (select credential_id from passkeys where id = $1)")
        .bind(passkey)
        .execute(&app.db)
        .await
        .unwrap();
    let (challenge, assertion) = passkey_assertion(&app, &mut other, &unknown).await;
    let res = app.send(passkey_finish(&challenge, &assertion)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "credential");

    // The stored passkey decides whose account it opens (combining moves them along).
    let (cara_id, _) = app.user("Cara", "US").await;
    sqlx::query("update passkeys set user_id = $1 where id = $2")
        .bind(cara_id)
        .bind(passkey)
        .execute(&app.db)
        .await
        .unwrap();
    let (challenge, assertion) = passkey_assertion(&app, &mut auth, &cred_id).await;
    let res = app.send(passkey_finish(&challenge, &assertion)).await;
    let session = set_cookie(&res, "slop_session").unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(me["id"], json!(cara_id));

    // A closed account's passkey signs nobody in.
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(cara_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (challenge, assertion) = passkey_assertion(&app, &mut auth, &cred_id).await;
    let res = app.send(passkey_finish(&challenge, &assertion)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(set_cookie(&res, "slop_session").is_none());
}

/// Adding a passkey is for the session that asked for it: finished from another session of the
/// same account, it is refused.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn adding_a_passkey_counts_only_for_the_session_that_started_it(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, laptop) = app.user("Ann", "US").await;
    let mut auth = authenticator();
    add_passkey(&app, &laptop, &mut auth).await;
    let phone = slop_api::auth::session::create(&app.db, &app.config, ann_id, Some("phone"))
        .await
        .unwrap();
    let phone = format!("{}={}", phone.name(), phone.value());
    // Adding a passkey is tied to its session the same way.
    sessions_now(&app, ann_id, "created_at").await;
    sessions_now(&app, ann_id, "passkey_at").await;
    let (challenge, credential) = new_credential(&app, &laptop, &mut auth).await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/passkeys/register/finish")
                .cookie(&phone)
                .json(
                    json!({ "challenge_id": challenge, "name": "Key", "credential": credential }),
                ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn expired_passkey_ceremonies_are_cleaned_up(db: PgPool) {
    let app = test_app(db).await;
    for _ in 0..3 {
        assert_eq!(
            status(&app, req(Method::POST, "/v1/auth/passkey/start")).await,
            StatusCode::OK
        );
    }
    sqlx::query(
        "update webauthn_challenges set expires_at = now() - interval '1 second'
         where id in (select id from webauthn_challenges order by id limit 2)",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(slop_api::passkeys::cleanup(&app.db).await.unwrap(), 2);
    assert_eq!(
        count(
            &app,
            "select count(*) from webauthn_challenges where expires_at > now()"
        )
        .await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from webauthn_challenges").await,
        1
    );
    // The hourly cleanup runs it too.
    sqlx::query("update webauthn_challenges set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(
        count(&app, "select count(*) from webauthn_challenges").await,
        0
    );
}

// ---------------------------------------------------------------------------------------------
// Staff

/// Staff with only the support role.
async fn support_staff(app: &TestApp, name: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'support', 'test')",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    (id, cookie)
}

/// Passkeys aren't required since 2026-10-25, so there's no grace for staff to give: the route
/// and the agents' tool are gone, and a grace asked for before then and still waiting is
/// approved to nothing.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn there_is_no_passkey_grace_any_more(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (ada_id, ada) = make_admin(&app, "Ada Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{maker}/passkey-grace"),
            )
            .cookie(&ada)
            .json(json!({ "reason": "Video call; matched their recent releases." })),
        )
        .await;
    assert!(res.status().is_client_error(), "{}", res.status());
    assert_eq!(count(&app, "select count(*) from staff_actions").await, 0);

    // One left waiting from before.
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into staff_actions (id, kind, summary, payload, requested_by, requested_actor, approvals_needed)
         values ($1, 'account.passkey_grace', 'Let Mia publish without a passkey for a week', $2, $3, 'Ada Owner', 1)",
    )
    .bind(id)
    .bind(json!({ "user_id": maker, "reason": "Video call; matched their recent releases." }))
    .bind(ada_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{id}/approve")).cookie(&ben))
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    let status: String = sqlx::query_scalar("select status from staff_actions where id = $1")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "done");
    let until: Option<OffsetDateTime> =
        sqlx::query_scalar("select passkey_grace_until from users where id = $1")
            .bind(maker)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(until, None);
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from notifications where user_id = '{maker}'")
        )
        .await,
        0
    );

    // The agents' tool is gone.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&ada)
                .json(json!({ "name": "Claude", "scope": "moderate", "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let token = format!("Bearer {}", body_json(res).await["token"].as_str().unwrap());
    let tools = body_json(
        app.send(
            req(Method::POST, "/v1/admin/mcp")
                .header("authorization", token)
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await,
    )
    .await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(!names.is_empty());
    assert!(!names.contains(&"passkey_grace"), "{names:?}");
}

// ---------------------------------------------------------------------------------------------
// Closing an account

fn close(cookie: &str) -> Req {
    req(Method::POST, "/v1/me/close")
        .cookie(cookie)
        .json(json!({ "confirm": "close my account" }))
}

/// Inserts with the given ids bound as $1, $2, ….
async fn exec(app: &TestApp, sql: &str, ids: &[Uuid]) {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
    for id in ids {
        q = q.bind(*id);
    }
    q.execute(&app.db).await.unwrap();
}

async fn app_id_of(app: &TestApp, slug: &str) -> Uuid {
    sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_waits_for_open_people_work_and_monthly_server_donations(db: PgPool) {
    let app = test_app(db).await;
    let (cleo, cleo_cookie) = app.user("Cleo Client", "US").await;
    let (cris, cris_cookie) = app.user("Cris Creative", "US").await;
    let (dan, _) = app.user("Dan Donor", "US").await;
    listed_with(&app, cleo, "cleos-game", "released", "").await;
    let game = app_id_of(&app, "cleos-game").await;
    let job = Uuid::now_v7();
    exec(
        &app,
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'cris', 'Cris', 'none', false)",
        &[cris],
    )
    .await;
    exec(
        &app,
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md)
         values ($1, $2, $3, $4, 'pixel-art', 'requested', 'Draw a cat.')",
        &[job, cris, cleo, game],
    )
    .await;
    let refused = |cookie: &str| {
        let app = &app;
        let r = close(cookie);
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::CONFLICT);
            body_json(res).await["error"]["message"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let why = refused(&cleo_cookie).await;
    assert!(
        why.starts_with("You can't close your account yet: 1 People job is still open"),
        "{why}"
    );
    assert!(
        refused(&cris_cookie)
            .await
            .contains("People job is still open")
    );

    // Finished, but not paid for.
    exec(
        &app,
        "update jobs set status = 'completed' where id = $1",
        &[job],
    )
    .await;
    exec(
        &app,
        "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status)
         values ($1, $2, 'full', 'delivery', 1000, 'usd', 'due')",
        &[Uuid::now_v7(), job],
    )
    .await;
    assert!(refused(&cleo_cookie).await.contains("you owe a payment"));
    // Paid, but a problem is still open.
    exec(
        &app,
        "update job_payments set status = 'paid' where job_id = $1",
        &[job],
    )
    .await;
    exec(
        &app,
        "insert into job_problems (id, job_id, opened_by, statement) values ($1, $2, $3, 'The cat has five legs.')",
        &[Uuid::now_v7(), job, cleo],
    )
    .await;
    assert!(
        refused(&cleo_cookie)
            .await
            .contains("a problem on a People job")
    );
    assert!(
        refused(&cris_cookie)
            .await
            .contains("a problem on a People job")
    );
    exec(
        &app,
        "update job_problems set status = 'resolved' where job_id = $1",
        &[job],
    )
    .await;

    // Giving monthly to a game's servers, and receiving it.
    listed_with(&app, dan, "dans-server", "released", "").await;
    let server = app_id_of(&app, "dans-server").await;
    let donation = |id: Uuid, app_id: Uuid, donor: Uuid| {
        let app = &app;
        async move {
            exec(
                app,
                "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                               slop_fee_minor, total_minor, consent_text, status)
                 values ($1, $2, $3, 'acct_x', 'usd', 500, 25, 525, 'Monthly.', 'active')",
                &[id, app_id, donor],
            )
            .await
        }
    };
    let giving = Uuid::now_v7();
    donation(giving, server, cleo).await;
    assert!(
        refused(&cleo_cookie)
            .await
            .contains("you give a monthly server donation")
    );
    exec(
        &app,
        "update server_donations set status = 'cancelled' where id = $1",
        &[giving],
    )
    .await;
    let receiving = Uuid::now_v7();
    donation(receiving, game, dan).await;
    assert!(
        refused(&cleo_cookie)
            .await
            .contains("1 people give monthly to your apps' servers")
    );
    let closed: bool = sqlx::query_scalar("select closed_at is not null from users where id = $1")
        .bind(cleo)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!closed);
    exec(
        &app,
        "update server_donations set status = 'cancelled' where id = $1",
        &[receiving],
    )
    .await;
    assert_eq!(
        status(&app, close(&cleo_cookie)).await,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_takes_away_everything_personal_and_hands_teams_on(db: PgPool) {
    let app = test_app(db).await;
    let (zoe, cookie) = app.user("Zoe", "US").await;
    let (finn, _) = app.user("Finn", "US").await;
    let (gia, _) = app.user("Gia", "US").await;
    let tokens = desktop_sign_in(&app, &cookie).await;
    listed_with(&app, gia, "gias-game", "released", "").await;
    listed_with(
        &app,
        zoe,
        "zoes-secret",
        "released",
        "visibility = 'private', status = 'draft'",
    )
    .await;
    let game = app_id_of(&app, "gias-game").await;
    let secret = app_id_of(&app, "zoes-secret").await;
    let (team_kept, team_gone, bounty) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (sql, ids) in [
        (
            "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, '\\x0a0b', '{}', 'Key')",
            vec![Uuid::now_v7(), zoe],
        ),
        (
            "insert into admin_tokens (id, user_id, name, scope, token_hash, expires_at) values ($1, $2, 'Bot', 'read', '\\x01', now() + interval '1 day')",
            vec![Uuid::now_v7(), zoe],
        ),
        (
            "insert into save_blobs (user_id, app_id, generation, body, device) values ($1, $2, 1, '\\x00', 'pc')",
            vec![zoe, game],
        ),
        (
            "insert into bounties (id, app_id, created_by, title, body, currency) values ($1, $2, $3, 'Fix it', 'Please.', 'usd')",
            vec![bounty, game, gia],
        ),
        (
            "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status) values ($1, $2, $3, 1000, 'active')",
            vec![Uuid::now_v7(), bounty, zoe],
        ),
        (
            "insert into library_entries (user_id, app_id) values ($1, $2)",
            vec![zoe, game],
        ),
        (
            "insert into interests (user_id, app_id) values ($1, $2)",
            vec![zoe, game],
        ),
        (
            "insert into user_app_usage (user_id, app_id, playtime_seconds) values ($1, $2, 3600)",
            vec![zoe, game],
        ),
        (
            "insert into notifications (id, user_id, category, title, link) values ($1, $2, 'apps', 'Hello', '/')",
            vec![Uuid::now_v7(), zoe],
        ),
        // Shared with her, and hers shared with Finn.
        (
            "insert into app_shares (app_id, user_id) values ($1, $2), ($3, $4)",
            vec![game, zoe, secret, finn],
        ),
        // A team she owns with Finn (joined first) and Gia, and one only she is in.
        (
            "insert into teams (id, slug, name, kind, created_by) values ($1, 'zoes-team', 'Zoe''s team', 'player', $3), ($2, 'zoe-alone', 'Alone', 'player', $3)",
            vec![team_kept, team_gone, zoe],
        ),
        (
            "insert into team_members (team_id, user_id, role, joined_at) values ($1, $2, 'owner', now() - interval '3 days'), ($1, $3, 'member', now() - interval '2 days'), ($1, $4, 'admin', now() - interval '1 day'), ($5, $2, 'owner', now())",
            vec![team_kept, zoe, finn, gia, team_gone],
        ),
        (
            "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'zoe-art', 'Zoe Art', 'none', false)",
            vec![zoe],
        ),
    ] {
        exec(&app, sql, &ids).await;
    }

    // Suspended accounts can close too.
    exec(
        &app,
        "update users set suspended_at = now() where id = $1",
        &[zoe],
    )
    .await;
    assert_eq!(status(&app, close(&cookie)).await, StatusCode::NO_CONTENT);

    for (what, sql) in [
        (
            "passkeys",
            "select count(*) from passkeys where user_id = $1",
        ),
        (
            "admin tokens",
            "select count(*) from admin_tokens where user_id = $1",
        ),
        (
            "saves",
            "select count(*) from save_blobs where user_id = $1",
        ),
        (
            "library",
            "select count(*) from library_entries where user_id = $1",
        ),
        (
            "interests",
            "select count(*) from interests where user_id = $1",
        ),
        (
            "playtime",
            "select count(*) from user_app_usage where user_id = $1",
        ),
        (
            "notifications",
            "select count(*) from notifications where user_id = $1",
        ),
        ("shares", "select count(*) from app_shares"),
        (
            "teams",
            "select count(*) from team_members where user_id = $1",
        ),
        (
            "live pledges",
            "select count(*) from bounty_pledges where user_id = $1 and status <> 'withdrawn'",
        ),
        (
            "live sessions",
            "select count(*) from sessions where user_id = $1 and revoked_at is null",
        ),
    ] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(zoe)
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(n, 0, "{what}");
    }
    let members: Vec<(Uuid, String)> = sqlx::query_as(
        "select user_id, role from team_members where team_id = $1 order by joined_at",
    )
    .bind(team_kept)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        members,
        [(finn, "owner".to_string()), (gia, "admin".to_string())],
        "the longest-standing member takes over"
    );
    assert_eq!(
        count(
            &app,
            &format!("select count(*) from teams where id = '{team_gone}'")
        )
        .await,
        0
    );
    let taking_work: bool =
        sqlx::query_scalar("select taking_work from creatives where user_id = $1")
            .bind(zoe)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!taking_work);
    // The desktop app is signed out too.
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/me").header(
                "authorization",
                format!("Bearer {}", tokens["access_token"].as_str().unwrap())
            )
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
    let actor: String = sqlx::query_scalar(
        "select actor from audit_log where action = 'account.close' and subject = $1",
    )
    .bind(zoe.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(actor, "the account's owner");
}

/// b02: closing an account also stops its signing keys, forgets its repository tokens and its
/// phones' push subscriptions (combining already drops push subscriptions).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_an_account_ends_its_signing_keys_repository_tokens_and_pushes(db: PgPool) {
    let app = test_app(db).await;
    let (mia, cookie) = app.user("Mia Maker", "US").await;
    let (other, _) = app.user("Other Maker", "US").await;
    listed_with(&app, mia, "mias-game", "released", "").await;
    listed_with(&app, other, "others-game", "released", "").await;
    let mine = app_id_of(&app, "mias-game").await;
    let theirs = app_id_of(&app, "others-game").await;
    for user in [mia, other] {
        exec(
            &app,
            "insert into maker_keys (id, user_id, public_key, fingerprint, name, active_from)
             values ($1, $2, 'ssh-ed25519 AAAA', 'SHA256:' || $2::text, 'Laptop', now() - interval '4 days')",
            &[Uuid::now_v7(), user],
        )
        .await;
        exec(
            &app,
            "insert into push_subscriptions (id, user_id, endpoint) values ($1, $2, 'https://push.example/' || $2::text)",
            &[Uuid::now_v7(), user],
        )
        .await;
    }
    for app_id in [mine, theirs] {
        exec(
            &app,
            "insert into repo_tokens (app_id, repo_url, kind, sealed) select id, repo_url, 'gitea', '\\x00' from apps where id = $1",
            &[app_id],
        )
        .await;
    }
    assert_eq!(status(&app, close(&cookie)).await, StatusCode::NO_CONTENT);
    let left = |sql: &'static str, id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let live_keys = "select count(*) from maker_keys where user_id = $1 and revoked_at is null";
    let pushes = "select count(*) from push_subscriptions where user_id = $1";
    let repo_tokens = "select count(*) from repo_tokens where app_id = $1";
    assert_eq!(left(live_keys, mia).await, 0, "signing keys");
    assert_eq!(left(pushes, mia).await, 0, "push subscriptions");
    assert_eq!(left(repo_tokens, mine).await, 0, "repository tokens");
    // Nobody else's.
    assert_eq!(left(live_keys, other).await, 1);
    assert_eq!(left(pushes, other).await, 1);
    assert_eq!(left(repo_tokens, theirs).await, 1);
}

// ---------------------------------------------------------------------------------------------
// Proof-of-work challenges

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn proof_of_work_answers_are_for_one_account_once_and_in_time(db: PgPool) {
    use slop_api::challenge::{Answer, check, solve};
    use slop_api::error::ApiError;
    let app = test_app(db).await;
    let (ann, _) = app.user("Ann", "US").await;
    let (bob, _) = app.user("Bob", "US").await;
    let challenge = |user: Uuid, expires: &'static str| {
        let db = app.db.clone();
        async move {
            let id = Uuid::now_v7();
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "insert into donation_challenges (id, user_id, prefix, difficulty, expires_at)
                 values ($1, $2, 'prefix-{id}', 8, now() + interval '{expires}')"
            )))
            .bind(id)
            .bind(user)
            .execute(&db)
            .await
            .unwrap();
            Answer {
                id,
                nonce: solve(&format!("prefix-{id}"), 8),
            }
        }
    };
    let refused = |r: Result<(), ApiError>| matches!(r, Err(ApiError::ChallengeRequired));

    assert!(refused(check(&app.db, ann, None).await));
    // Ann's challenge doesn't help Bob, and Bob trying doesn't use it up.
    let answer = challenge(ann, "10 minutes").await;
    assert!(refused(check(&app.db, bob, Some(&answer)).await));
    assert!(check(&app.db, ann, Some(&answer)).await.is_ok());
    assert!(refused(check(&app.db, ann, Some(&answer)).await), "once");

    // Expired.
    let answer = challenge(ann, "-1 second").await;
    assert!(refused(check(&app.db, ann, Some(&answer)).await));

    // An over-long answer is refused without looking.
    let answer = challenge(ann, "10 minutes").await;
    let long = Answer {
        id: answer.id,
        nonce: format!("{}{}", "0".repeat(41 - answer.nonce.len()), answer.nonce),
    };
    assert!(long.nonce.len() > 40);
    assert!(refused(check(&app.db, ann, Some(&long)).await));
    assert!(
        check(&app.db, ann, Some(&answer)).await.is_ok(),
        "still unused"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bounty_pledges_after_unfinished_ones_need_a_proof_of_work(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (maker, _) = app.user("Maker", "US").await;
    let (ann, cookie) = app.user("Ann Backer", "US").await;
    listed_with(&app, maker, "tower", "released", "").await;
    approved_release(&app, "tower", maker).await;
    let tower = app_id_of(&app, "tower").await;
    let mut bounties = vec![];
    for _ in 0..4 {
        let id = Uuid::now_v7();
        exec(
            &app,
            "insert into bounties (id, app_id, created_by, title, body, currency) values ($1, $2, $3, 'Fix saves', 'Saves vanish.', 'usd')",
            &[id, tower, maker],
        )
        .await;
        bounties.push(id);
    }
    // Three pledges in a day that came to nothing.
    for id in &bounties[..3] {
        exec(
            &app,
            "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status) values ($1, $2, $3, 1000, 'withdrawn')",
            &[Uuid::now_v7(), *id, ann],
        )
        .await;
    }
    let pledge = |extra: Value| {
        let mut body = json!({ "amount_minor": 1000, "confirm_adult": true });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        req(Method::POST, format!("/v1/bounties/{}/pledge", bounties[3]))
            .cookie(&cookie)
            .json(body)
    };
    let res = app.send(pledge(json!({}))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(res).await["error"]["code"], "challenge_required");
    let res = app
        .send(pledge(
            json!({ "challenge": { "id": Uuid::now_v7(), "nonce": "1" } }),
        ))
        .await;
    assert_eq!(body_json(res).await["error"]["code"], "challenge_required");

    let id = Uuid::now_v7();
    exec(
        &app,
        "insert into donation_challenges (id, user_id, prefix, difficulty, expires_at) values ($1, $2, 'p', 8, now() + interval '10 minutes')",
        &[id, ann],
    )
    .await;
    let nonce = slop_api::challenge::solve("p", 8);
    let res = app
        .send(pledge(json!({ "challenge": { "id": id, "nonce": nonce } })))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let live: i64 = sqlx::query_scalar(
        "select count(*) from bounty_pledges where user_id = $1 and status = 'active'",
    )
    .bind(ann)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(live, 1);
}

// ---------------------------------------------------------------------------------------------
// Cleanup, sealed fields, cross-site writes, the simulation

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cleanup_deletes_ended_sign_in_leftovers_and_keeps_live_ones(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    // A desktop session that refreshed once: the used refresh token is kept until it expires.
    let tokens = desktop_sign_in(&app, &web).await;
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    sqlx::query(
        "update access_tokens set expires_at = now() - interval '1 second' where token_hash = $1",
    )
    .bind(slop_api::tokens::hash_token(
        tokens["access_token"].as_str().unwrap(),
    ))
    .execute(&app.db)
    .await
    .unwrap();

    // Two pending sign-ups and two desktop requests, one of each expired.
    for _ in 0..2 {
        oauth_flow(&app, "/v1/auth/github/start", &[], "ok").await;
        app.send(
            req(Method::POST, "/v1/auth/desktop/requests")
                .json(desktop_request("state-state-state-1", "Laptop")),
        )
        .await;
    }
    exec(
        &app,
        "update pending_signups set expires_at = now() - interval '1 second' where ctid = (select ctid from pending_signups limit 1)",
        &[],
    )
    .await;
    exec(
        &app,
        "update desktop_auth_requests set expires_at = now() - interval '1 second' where ctid = (select ctid from desktop_auth_requests limit 1)",
        &[],
    )
    .await;
    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.pending_signups, 1, "{deleted:?}");
    assert_eq!(deleted.desktop_requests, 1, "{deleted:?}");
    assert_eq!(deleted.access_tokens, 1, "{deleted:?}");
    assert_eq!(deleted.refresh_tokens, 0, "{deleted:?}");
    assert_eq!(count(&app, "select count(*) from refresh_tokens").await, 2);
    // The used one still gives the theft away.
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    sqlx::query("update refresh_tokens set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.refresh_tokens, 2, "{deleted:?}");
}

/// b01: desktop sessions count as unused only after 30 days (cleanup used the website's 7 days for
/// them). AI agent tokens have no idle limit at all: they last until the expiry their owner chose.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cleanup_keeps_desktop_sessions_for_thirty_idle_days_and_agent_tokens_until_they_expire(
    db: PgPool,
) {
    let app = test_app(db).await;
    let (_, web) = app.user("Mia Maker", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&web)
                .json(json!({ "name": "Claude", "scope": "read" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let agent = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    let tokens = desktop_sign_in(&app, &web).await;
    let desktop = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    sqlx::query("update sessions set last_seen_at = now() - interval '20 days' where kind in ('agent', 'desktop')")
        .execute(&app.db)
        .await
        .unwrap();
    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.sessions, 0, "{deleted:?}");
    for bearer in [&agent, &desktop] {
        assert_eq!(
            status(
                &app,
                req(Method::GET, "/v1/me").header("authorization", bearer.clone())
            )
            .await,
            StatusCode::OK,
            "still signed in after 20 unused days"
        );
    }
    // 30 idle days and the week's keeping after: the desktop session is gone, the agent's token
    // (made for 30 days, so still live) stays.
    sqlx::query("update sessions set last_seen_at = now() - interval '38 days' where kind in ('agent', 'desktop')")
        .execute(&app.db)
        .await
        .unwrap();
    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.sessions, 1, "{deleted:?}");
    assert_eq!(
        status(
            &app,
            req(Method::GET, "/v1/me").header("authorization", agent.clone())
        )
        .await,
        StatusCode::OK,
        "an agent token works until its expiry, however long unused"
    );
    // Expired, and the week's keeping after: gone too.
    sqlx::query("update sessions set expires_at = now() - interval '8 days' where kind = 'agent'")
        .execute(&app.db)
        .await
        .unwrap();
    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.sessions, 1, "{deleted:?}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn vat_numbers_stored_before_the_key_are_sealed_at_start_up(db: PgPool) {
    use slop_api::fields::{open, seal, seal_existing};
    let key = [11u8; 32];
    let app = test_app_with(db.clone(), |c| c.field_key = None).await;
    let (plain, _) = app.user("Plain", "DE").await;
    let (sealed, _) = app.user("Sealed", "DE").await;
    let already = seal(Some(&key), "users.vat_number", "DE999999999").unwrap();
    exec(
        &app,
        "update users set vat_number = 'DE123456789' where id = $1",
        &[plain],
    )
    .await;
    sqlx::query("update users set vat_number = $2 where id = $1")
        .bind(sealed)
        .bind(&already)
        .execute(&app.db)
        .await
        .unwrap();
    // No key: nothing to do.
    assert_eq!(seal_existing(&app.state).await.unwrap(), 0);
    let vat = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("select vat_number from users where id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(vat(plain).await, "DE123456789");

    // With the key: the plain one is sealed, the sealed one left as it is.
    let keyed = test_app_with(db, |c| c.field_key = Some(key)).await;
    assert_eq!(seal_existing(&keyed.state).await.unwrap(), 1);
    let now_sealed = vat(plain).await;
    assert!(now_sealed.starts_with("enc1:") && !now_sealed.contains("DE123"));
    assert_eq!(
        open(Some(&key), "users.vat_number", &now_sealed).unwrap(),
        "DE123456789"
    );
    assert_eq!(vat(sealed).await, already);
    assert_eq!(seal_existing(&keyed.state).await.unwrap(), 0, "once");

    // Damaged values don't open.
    let mut damaged = now_sealed.clone();
    let last = damaged.pop().unwrap();
    damaged.push(if last == 'A' { 'B' } else { 'A' });
    for bad in [damaged.as_str(), "enc1:AAAA", "enc1:not base64!"] {
        assert!(open(Some(&key), "users.vat_number", bad).is_err(), "{bad}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn account_changes_from_other_sites_are_refused(db: PgPool) {
    let (base, _) = fake_providers().await;
    let app = test_app_with(db, move |c| {
        all_providers(c, &base);
        c.trusted_origins = vec!["https://admin.slop.example".into()];
    })
    .await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    identity(&app, ann_id, "github", "1").await;
    identity(&app, ann_id, "gitlab", "2").await;
    identity(&app, bob_id, "google", "g-bob").await;
    let combine = start_combining(&app, &ann, "g-bob").await;
    let request = body_json(
        app.send(
            req(Method::POST, "/v1/auth/desktop/requests")
                .json(desktop_request("state-state-state-1", "Laptop")),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let writes = || {
        vec![
            close(&ann),
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{request}/approve"),
            )
            .cookie(&ann),
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine),
            req(Method::POST, "/v1/me/passkeys/register/start").cookie(&ann),
            req(Method::DELETE, "/v1/me/identities/gitlab/2").cookie(&ann),
            req(Method::PATCH, "/v1/me")
                .cookie(&ann)
                .json(json!({ "share_playtime": false })),
        ]
    };
    for r in writes() {
        let uri = r.uri.clone();
        assert_eq!(
            status(&app, r.header("sec-fetch-site", "cross-site")).await,
            StatusCode::FORBIDDEN,
            "{uri}"
        );
    }
    for r in writes() {
        let uri = r.uri.clone();
        assert_eq!(
            status(&app, r.header("origin", "https://evil.example")).await,
            StatusCode::FORBIDDEN,
            "{uri}"
        );
    }
    // None of it happened.
    let (closed, share): (bool, bool) =
        sqlx::query_as("select closed_at is not null, share_playtime from users where id = $1")
            .bind(ann_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((closed, share), (false, true));
    assert_eq!(
        count(
            &app,
            "select count(*) from desktop_auth_requests where approved_by is not null"
        )
        .await,
        0
    );
    assert_eq!(count(&app, "select count(*) from identities").await, 3);
    assert_eq!(
        count(&app, "select count(*) from webauthn_challenges").await,
        0
    );
    let merged: Option<Uuid> = sqlx::query_scalar("select merged_into from users where id = $1")
        .bind(bob_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(merged, None);

    // A trusted origin may, even across sites.
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&ann)
                .header("sec-fetch-site", "cross-site")
                .header("origin", "https://admin.slop.example")
                .json(json!({ "share_playtime": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn simulation_sign_in_exists_only_on_a_simulation_server(db: PgPool) {
    let real = test_app(db.clone()).await;
    let (person, _) = real.user("Real Person", "US").await;
    let (sim_maker, _) = real.user("Sim maker 1", "US").await;
    let (sim_player, _) = real.user("Sim player 1", "US").await;
    exec(
        &real,
        "create table sim_users (user_id uuid primary key, personas text not null, taste jsonb not null, activity real not null)",
        &[],
    )
    .await;
    exec(
        &real,
        "insert into sim_users (user_id, personas, taste, activity) values ($1, 'casual', '{}', 0.5)",
        &[sim_player],
    )
    .await;
    let login = |id: Uuid| req(Method::POST, format!("/v1/sim/login/{id}"));
    // A real server never signs anyone in this way, even a simulated person.
    for id in [sim_player, sim_maker, person] {
        let res = real.send(login(id)).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(set_cookie(&res, "slop_session").is_none());
    }

    let sim = test_app_with(db, |c| c.simulation = true).await;
    let res = sim.send(login(person)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "real accounts never");
    assert!(set_cookie(&res, "slop_session").is_none());
    for id in [sim_player, sim_maker] {
        let res = sim.send(login(id)).await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let session = set_cookie(&res, "slop_session").unwrap();
        let me = body_json(sim.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
        assert_eq!(me["id"], json!(id));
    }
}

// ---------------------------------------------------------------------------------------------
// Makers' signing keys

/// An Ed25519 public key line for a key made from `seed`.
fn public_key(seed: u8) -> String {
    let key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    ssh_sign(&key, b"").0
}

fn add_key(cookie: &str, name: &str, key: &str) -> Req {
    req(Method::POST, "/v1/me/signing-keys")
        .cookie(cookie)
        .json(json!({ "name": name, "public_key": key }))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signing_keys_are_listed_with_their_wait_and_without_removed_ones(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Other", "US").await;
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me/signing-keys")).await,
        StatusCode::UNAUTHORIZED
    );
    let mut ids = vec![];
    for (seed, name) in [(1, "Old laptop"), (2, "New laptop"), (3, "Lost")] {
        let res = app.send(add_key(&mia, name, &public_key(seed))).await;
        assert_eq!(res.status(), StatusCode::CREATED);
        ids.push(body_json(res).await["id"].as_str().unwrap().to_string());
    }
    sqlx::query("update maker_keys set active_from = now() - interval '1 day' where id = $1::uuid")
        .bind(&ids[0])
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{}", ids[2])).cookie(&mia)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let listed = body_json(
        app.send(req(Method::GET, "/v1/me/signing-keys").cookie(&mia))
            .await,
    )
    .await;
    let rows: Vec<(String, bool)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            (
                k["name"].as_str().unwrap().to_string(),
                k["active"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Old laptop".to_string(), true),
            ("New laptop".to_string(), false)
        ]
    );
    assert!(listed[1]["active_from"].is_string());
    assert!(listed[0].get("public_key").is_none());
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/signing-keys").cookie(&other))
            .await,
    )
    .await;
    assert_eq!(theirs, json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn adding_a_signing_key_checks_the_key_the_name_and_the_count(db: PgPool) {
    let app = test_app(db).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let field = |r: Req| {
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
            body_json(res).await["error"]["field"].clone()
        }
    };
    for bad in [
        "not a key",
        "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC7 maker@laptop",
        "ssh-ed25519 AAAA maker@laptop",
        "",
    ] {
        assert_eq!(
            field(add_key(&mia, "Laptop", bad)).await,
            "public_key",
            "{bad}"
        );
    }
    for bad in ["", "  ", &"n".repeat(61)] {
        assert_eq!(
            field(add_key(&mia, bad, &public_key(1))).await,
            "name",
            "{bad:?}"
        );
    }
    assert_eq!(count(&app, "select count(*) from maker_keys").await, 0);

    // Five at most.
    let mut first = String::new();
    for seed in 1..=5 {
        let res = app
            .send(add_key(&mia, &format!("Key {seed}"), &public_key(seed)))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        if seed == 1 {
            first = body_json(res).await["id"].as_str().unwrap().to_string();
        }
    }
    assert_eq!(
        status(&app, add_key(&mia, "Key 6", &public_key(6))).await,
        StatusCode::CONFLICT
    );
    let audited: i64 = sqlx::query_scalar(
        "select count(*) from audit_log where action = 'maker_key.add' and actor = $1 and subject like 'SHA256:%'",
    )
    .bind(mia_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(audited, 5);

    // A removed key added again waits its 72 hours again, however long it worked before.
    sqlx::query(
        "update maker_keys set active_from = now() - interval '30 days' where id = $1::uuid",
    )
    .bind(&first)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        status(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{first}")).cookie(&mia)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let res = app.send(add_key(&mia, "Back again", &public_key(1))).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let back = body_json(res).await;
    assert_eq!(back["id"], json!(first), "the same key, restarted");
    assert_eq!(back["active"], false);
    let hours: f64 = sqlx::query_scalar(
        "select (extract(epoch from active_from - now()) / 3600)::float8 from maker_keys where id = $1::uuid",
    )
    .bind(&first)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((71.9..72.1).contains(&hours), "{hours}");
}

/// A release host of this test's own that serves a game file and the maker's `.sig` beside it.
async fn signed_release_host(sig: Arc<Mutex<String>>) -> String {
    let app = Router::new().route(
        "/octo/game/releases/download/{tag}/{file}",
        get(
            move |axum::extract::Path((_, file)): axum::extract::Path<(String, String)>| {
                let sig = sig.clone();
                async move {
                    if file.ends_with(".sig") {
                        let sig = sig.lock().unwrap().clone();
                        if sig.is_empty() {
                            StatusCode::NOT_FOUND.into_response()
                        } else {
                            sig.into_response()
                        }
                    } else {
                        BUILD_BYTES.into_response()
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_a_signing_key_stops_it_signing_at_once(db: PgPool) {
    let app = test_app(db).await;
    let sig = Arc::new(Mutex::new(String::new()));
    let host = signed_release_host(sig.clone()).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Other", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'game', 'Game', 'A game', 'game', 'released', $2, $3, 'MIT', '{windows}')",
    )
    .bind(Uuid::now_v7())
    .bind(mia_id)
    .bind(format!("{host}octo/game"))
    .execute(&app.db)
    .await
    .unwrap();
    let key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    let (line, signature) = ssh_sign(&key, BUILD_BYTES);
    *sig.lock().unwrap() = signature;
    let added = body_json(app.send(add_key(&mia, "Laptop", &line)).await).await;
    let id = added["id"].as_str().unwrap().to_string();
    sqlx::query("update maker_keys set active_from = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();
    let submit = |version: &str| {
        let app = &app;
        let body = json!({
            "version": version,
            "commit_sha": "a".repeat(40),
            "files": [{
                "platform": "windows", "kind": "archive",
                "url": format!("{host}octo/game/releases/download/v1/game-windows.zip"),
                "exec": "bin/game"
            }]
        });
        let r = req(Method::POST, "/v1/apps/game/releases")
            .cookie(&mia)
            .json(body);
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::CREATED);
            let id = body_json(res).await["id"].as_str().unwrap().to_string();
            assert!(releases::fetch_next(&app.state).await.unwrap());
            sqlx::query_scalar::<_, Option<String>>(
                "select provenance_problem from release_files where release_id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    assert_eq!(submit("1.0.0").await, None, "signed with the key");

    // Not someone else's to remove; an unknown one isn't there.
    let remove = |cookie: &str, id: &str| {
        req(Method::DELETE, format!("/v1/me/signing-keys/{id}")).cookie(cookie)
    };
    assert_eq!(
        status(&app, remove(&other, &id)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, remove(&mia, &Uuid::now_v7().to_string())).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, remove(&mia, &id)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&app, remove(&mia, &id)).await,
        StatusCode::NOT_FOUND,
        "already removed"
    );

    // The next release signed with it isn't the maker's any more.
    let problem = submit("1.0.1").await.unwrap();
    assert!(problem.contains("no signing key"), "{problem}");
}

// ---------------------------------------------------------------------------------------------
// Repository tokens

async fn private_app(app: &TestApp, maker: Uuid, slug: &str, repo: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms, visibility)
         values ($1, $2, 'Secret', 'A secret game', 'game', 'released', $3, $4, 'MIT', '{windows}', 'private')",
    )
    .bind(id)
    .bind(slug)
    .bind(maker)
    .bind(repo)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

fn save_token(cookie: &str, slug: &str, kind: &str, token: &str) -> Req {
    req(Method::PUT, format!("/v1/apps/{slug}/repo-token"))
        .cookie(cookie)
        .json(json!({ "kind": kind, "token": token }))
}

/// The error message of a refused request, checking it was refused for `field`.
async fn refused_because(app: &TestApp, r: Req, field: &str) -> String {
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(res).await;
    assert_eq!(body["error"]["field"], field, "{body}");
    body["error"]["message"].as_str().unwrap().to_string()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saving_a_repository_token_checks_it_with_the_host_and_keeps_it_sealed(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (finn_id, finn) = app.user("Finn", "US").await;
    let repo = format!("{}ann/secret-game", app.fake_base);
    let game = private_app(&app, ann_id, "secret-game", &repo).await;
    private_app(
        &app,
        ann_id,
        "missing-game",
        &format!("{}ann/missing", app.fake_base),
    )
    .await;
    private_app(&app, ann_id, "github-game", "https://github.com/ann/game").await;
    // Someone it's shared with isn't its maker.
    exec(
        &app,
        "insert into app_shares (app_id, user_id) values ($1, $2)",
        &[game, finn_id],
    )
    .await;
    assert_eq!(
        status(&app, save_token(&finn, "secret-game", "gitea", "good")).await,
        StatusCode::NOT_FOUND
    );

    // The token on its own, and one the host takes.
    let long = "t".repeat(501);
    for bad in ["", "  ", "go od", "go\tod", long.as_str()] {
        let why = refused_because(
            &app,
            save_token(&ann, "secret-game", "gitea", bad),
            "repo_token",
        )
        .await;
        assert!(why.contains("on its own"), "{bad:?}: {why}");
    }
    let why = refused_because(
        &app,
        save_token(&ann, "secret-game", "gitea", "bad"),
        "repo_token",
    )
    .await;
    assert!(why.contains("the host refused the token"), "{why}");
    let why = refused_because(
        &app,
        save_token(&ann, "missing-game", "gitea", "good"),
        "repo_token",
    )
    .await;
    assert!(why.contains("can't find the repository"), "{why}");
    let why = refused_because(
        &app,
        save_token(&ann, "github-game", "gitlab", "glpat"),
        "repo_token",
    )
    .await;
    assert!(why.contains("GitHub App"), "{why}");
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 0);

    let res = app
        .send(save_token(&ann, "secret-game", "gitea", "good"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let saved = body_json(res).await;
    assert_eq!(
        (saved["saved"].clone(), saved["kind"].clone()),
        (json!(true), json!("gitea"))
    );
    assert!(!saved.to_string().contains("good"));
    let (sealed, by): (Vec<u8>, Uuid) =
        sqlx::query_as("select sealed, created_by from repo_tokens where app_id = $1")
            .bind(game)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!sealed.windows(4).any(|w| w == b"good"));
    assert_eq!(by, ann_id);
    let audited: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'repo_token.save' and subject = 'secret-game'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(audited, json!({ "kind": "gitea" }));

    // Forgetting: only the maker, and the token stays until they do.
    assert_eq!(
        status(
            &app,
            req(Method::DELETE, "/v1/apps/secret-game/repo-token").cookie(&finn)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 1);
}

/// b06: an app has one maker, and only they manage it: its releases, store page and repository
/// token alike. A team the app is shared with plays it; it doesn't manage it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_team_an_app_is_shared_with_doesnt_manage_its_repository_token(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (finn_id, finn) = app.user("Finn", "US").await;
    let repo = format!("{}ann/secret-game", app.fake_base);
    let game = private_app(&app, ann_id, "secret-game", &repo).await;
    let team = Uuid::now_v7();
    exec(
        &app,
        "insert into teams (id, slug, name, kind, created_by) values ($1, 'anns-team', 'Ann''s team', 'player', $2)",
        &[team, ann_id],
    )
    .await;
    exec(
        &app,
        "insert into team_members (team_id, user_id, role) values ($1, $2, 'owner'), ($1, $3, 'admin')",
        &[team, ann_id, finn_id],
    )
    .await;
    exec(
        &app,
        "insert into app_team_shares (app_id, team_id) values ($1, $2)",
        &[game, team],
    )
    .await;
    assert_eq!(
        status(&app, save_token(&ann, "secret-game", "gitea", "good")).await,
        StatusCode::OK
    );
    for r in [
        req(Method::GET, "/v1/apps/secret-game/repo-token").cookie(&finn),
        save_token(&finn, "secret-game", "gitea", "good"),
        req(Method::DELETE, "/v1/apps/secret-game/repo-token").cookie(&finn),
        req(Method::POST, "/v1/apps/secret-game/releases")
            .cookie(&finn)
            .json(release_body(&app, "1.0.0", "game.zip")),
    ] {
        let uri = r.uri.clone();
        assert_eq!(status(&app, r).await, StatusCode::NOT_FOUND, "{uri}");
    }
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repository_tokens_need_the_servers_key_and_a_public_https_host(db: PgPool) {
    let no_key = test_app_with(db.clone(), |c| c.releases.repo_token_key = None).await;
    let (ann_id, ann) = no_key.user("Ann", "US").await;
    private_app(
        &no_key,
        ann_id,
        "secret-game",
        &format!("{}ann/secret-game", no_key.fake_base),
    )
    .await;
    let res = no_key
        .send(save_token(&ann, "secret-game", "gitea", "good"))
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let shown = body_json(
        no_key
            .send(req(Method::GET, "/v1/apps/secret-game/repo-token").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(shown["available"], false);

    // A real server: https only, never a private address, and Bitbucket tokens for bitbucket.org.
    let real = test_app_with(db, |c| c.releases.allow_http_downloads = false).await;
    for (slug, repo) in [
        ("plain-http", "http://example.com/ann/game"),
        ("loopback", "https://127.0.0.1/ann/game"),
        ("private-net", "https://10.0.0.8/ann/game"),
        ("elsewhere", "https://1.1.1.1/ann/game"),
    ] {
        private_app(&real, ann_id, slug, repo).await;
    }
    let why = refused_because(
        &real,
        save_token(&ann, "plain-http", "gitea", "good"),
        "repo_token",
    )
    .await;
    assert!(why.contains("only https"), "{why}");
    for slug in ["loopback", "private-net"] {
        let why = refused_because(
            &real,
            save_token(&ann, slug, "gitlab", "good"),
            "repo_token",
        )
        .await;
        assert!(why.contains("private network"), "{slug}: {why}");
    }
    let why = refused_because(
        &real,
        save_token(&ann, "elsewhere", "bitbucket", "good"),
        "repo_token",
    )
    .await;
    assert!(why.contains("bitbucket.org"), "{why}");
    assert_eq!(count(&real, "select count(*) from repo_tokens").await, 0);
}

/// A forge of this test's own: a GitLab API, a Bitbucket API (at /bitbucket-api, standing in for
/// api.bitbucket.org) and file links that answer in different ways. Only "secret" reads.
async fn fake_forge() -> String {
    fn authorized(headers: &axum::http::HeaderMap) -> bool {
        headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer secret")
    }
    let readme = "# Game\n\nslop-verify: proof-123\n";
    let app = Router::new()
        .route(
            "/api/v4/projects/{id}",
            get(|headers: axum::http::HeaderMap, axum::extract::Path(id): axum::extract::Path<String>| async move {
                if !authorized(&headers) || id != "ann/game" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!({ "default_branch": "trunk" })).into_response()
            }),
        )
        .route(
            "/api/v4/projects/{id}/repository/tree",
            get(|headers: axum::http::HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                if !authorized(&headers) || q.get("ref").map(String::as_str) != Some("trunk") {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!([
                    { "type": "blob", "name": "README.md" },
                    { "type": "tree", "name": "src" },
                    { "type": "blob", "name": "notes.txt" }
                ]))
                .into_response()
            }),
        )
        .route(
            "/api/v4/projects/{id}/repository/files/{name}/raw",
            get(move |headers: axum::http::HeaderMap, axum::extract::Path((_, name)): axum::extract::Path<(String, String)>| async move {
                if !authorized(&headers) || name != "README.md" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                readme.into_response()
            }),
        )
        .route(
            "/bitbucket-api/2.0/repositories/ann/game",
            get(|headers: axum::http::HeaderMap| async move {
                if !authorized(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                axum::Json(json!({ "mainbranch": { "name": "main" } })).into_response()
            }),
        )
        .route(
            "/bitbucket-api/2.0/repositories/ann/game/src/main/",
            get(|headers: axum::http::HeaderMap| async move {
                if !authorized(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                axum::Json(json!({ "values": [
                    { "type": "commit_file", "path": "slop-verify.txt" },
                    { "type": "commit_directory", "path": "src" }
                ] }))
                .into_response()
            }),
        )
        .route(
            "/bitbucket-api/2.0/repositories/ann/game/src/main/{name}",
            get(|headers: axum::http::HeaderMap, axum::extract::Path(name): axum::extract::Path<String>| async move {
                if !authorized(&headers) || name != "slop-verify.txt" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                "proof-456\n".into_response()
            }),
        )
        // File links: to the same host, to plain http elsewhere, to https elsewhere, or the file.
        .route("/same/game.zip", get(|| async { axum::response::Redirect::to("/storage/game.zip") }))
        .route(
            "/to-http/game.zip",
            get(|| async { axum::response::Redirect::to("http://objects.example/game.zip?sig=1") }),
        )
        .route(
            "/to-https/game.zip",
            get(|| async { axum::response::Redirect::to("https://objects.example/game.zip?sig=1") }),
        )
        .route("/direct/game.zip", get(|| async { BUILD_BYTES }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saved_repository_tokens_are_sent_only_to_their_repository(db: PgPool) {
    use slop_api::repo_tokens::{Given, Kind, auth_for, saved_kind, store};
    let app = test_app(db).await;
    let (ann_id, _) = app.user("Ann", "US").await;
    let repo = format!("{}ann/secret-game", app.fake_base);
    let game = private_app(&app, ann_id, "secret-game", &repo).await;
    let given = Given {
        kind: Kind::Gitea,
        token: "good".into(),
    };
    let mut conn = app.db.acquire().await.unwrap();
    store(&mut conn, &[9; 32], game, &repo, &given, ann_id)
        .await
        .unwrap();
    drop(conn);
    let file = format!("{repo}/releases/download/v1/game.zip");
    assert_eq!(
        auth_for(&app.state, game, &file).await.unwrap().as_deref(),
        Some("token good")
    );
    let used: bool =
        sqlx::query_scalar("select last_used_at is not null from repo_tokens where app_id = $1")
            .bind(game)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(used);
    assert_eq!(saved_kind(&app.db, game).await.unwrap(), Some(Kind::Gitea));

    // Another host, port or scheme never gets it.
    let port = url::Url::parse(&app.fake_base).unwrap().port().unwrap();
    for elsewhere in [
        format!("http://localhost:{port}/ann/secret-game/releases/download/v1/game.zip"),
        "http://127.0.0.1:1/ann/secret-game/releases/download/v1/game.zip".to_string(),
        format!("https://127.0.0.1:{port}/ann/secret-game/releases/download/v1/game.zip"),
        "not a url".to_string(),
    ] {
        assert_eq!(
            auth_for(&app.state, game, &elsewhere).await.unwrap(),
            None,
            "{elsewhere}"
        );
    }
    // Moved to another repository: the token is for the old one, so it's not used.
    sqlx::query("update apps set repo_url = $2 where id = $1")
        .bind(game)
        .bind(format!("{}ann/other-game", app.fake_base))
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(auth_for(&app.state, game, &file).await.unwrap(), None);
    assert_eq!(saved_kind(&app.db, game).await.unwrap(), None);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ownership_proofs_are_read_through_gitlab_and_bitbucket(db: PgPool) {
    use slop_api::repo_tokens::{Given, Kind, read_proof};
    let app = test_app(db).await;
    let forge = fake_forge().await;
    let repo = format!("{forge}ann/game");
    let given = |kind: Kind, token: &str| Given {
        kind,
        token: token.into(),
    };
    let gitlab = read_proof(&app.state, &repo, &given(Kind::Gitlab, "secret"))
        .await
        .unwrap()
        .unwrap();
    assert!(gitlab.contains("proof-123"), "{gitlab}");
    let bitbucket = read_proof(&app.state, &repo, &given(Kind::Bitbucket, "secret"))
        .await
        .unwrap()
        .unwrap();
    assert!(bitbucket.contains("proof-456"), "{bitbucket}");
    // A token the host refuses.
    for kind in [Kind::Gitlab, Kind::Bitbucket] {
        assert!(
            read_proof(&app.state, &repo, &given(kind, "wrong"))
                .await
                .is_err(),
            "{kind:?}"
        );
    }
    // A repository URL without an owner and a name.
    assert!(
        read_proof(
            &app.state,
            &format!("{forge}game"),
            &given(Kind::Gitlab, "secret")
        )
        .await
        .is_err()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_get_the_hosts_short_lived_link_or_nothing(db: PgPool) {
    use slop_api::repo_tokens::short_lived_link;
    let app = test_app(db.clone()).await;
    let forge = fake_forge().await;
    let link = |state: &AppState, path: &str| {
        let url = format!("{forge}{path}");
        let state = state.clone();
        async move { short_lived_link(&state, &url, "Bearer secret").await }
    };
    // Tests allow plain http downloads: a redirect to another host is the link.
    assert_eq!(
        link(&app.state, "to-http/game.zip").await.unwrap(),
        "http://objects.example/game.zip?sig=1"
    );
    // A redirect on the same host would carry the token's access: not a link to hand out.
    assert!(link(&app.state, "same/game.zip").await.is_err());
    // A host that serves the file itself: Slop doesn't relay files.
    let err = link(&app.state, "direct/game.zip").await.unwrap_err();
    assert!(format!("{err:#}").contains("never relays files"), "{err:#}");

    // A real server hands out https links only.
    let real = test_app_with(db, |c| c.releases.allow_http_downloads = false).await;
    assert!(link(&real.state, "to-http/game.zip").await.is_err());
    assert_eq!(
        link(&real.state, "to-https/game.zip").await.unwrap(),
        "https://objects.example/game.zip?sig=1"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_repository_token_that_stops_working_freezes_the_app(db: PgPool) {
    use slop_api::frozen::{Access, apply, check_app};
    use slop_api::repo_tokens::{Given, Kind, still_works, store};
    let app = test_app(db).await;
    let (ann_id, _) = app.user("Ann", "US").await;
    let repo = format!("{}ann/secret-game", app.fake_base);
    let game = private_app(&app, ann_id, "secret-game", &repo).await;
    // No token: nothing to say.
    assert_eq!(still_works(&app.state, game).await, None);
    let save = |token: &str| {
        let given = Given {
            kind: Kind::Gitea,
            token: token.into(),
        };
        let db = app.db.clone();
        let repo = repo.clone();
        async move {
            let mut conn = db.acquire().await.unwrap();
            store(&mut conn, &[9; 32], game, &repo, &given, ann_id)
                .await
                .unwrap();
        }
    };
    save("good").await;
    assert_eq!(still_works(&app.state, game).await, Some(true));
    assert_eq!(
        check_app(&app.state, game, &repo, false).await,
        Access::Readable
    );

    // Revoked at the host (the saved token no longer reads it).
    save("revoked").await;
    assert_eq!(still_works(&app.state, game).await, Some(false));
    let access = check_app(&app.state, game, &repo, false).await;
    assert!(
        matches!(&access, Access::Lost(why) if why.contains("token can't read")),
        "{access:?}"
    );
    apply(&app.state, game, access).await.unwrap();
    let (frozen, reason): (bool, Option<String>) =
        sqlx::query_as("select frozen_at is not null, frozen_reason from apps where id = $1")
            .bind(game)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(frozen);
    assert!(reason.unwrap().contains("Save a new one"));

    // A token that doesn't decrypt (the server's key changed): Slop can't tell, so it doesn't
    // freeze the app on it.
    sqlx::query("update repo_tokens set sealed = '\\x00010203' where app_id = $1")
        .bind(game)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(still_works(&app.state, game).await, None);
}

// ---------------------------------------------------------------------------------------------
// Legal requests and the transparency report

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap()
}

fn log_request(cookie: &str, body: Value) -> Req {
    req(Method::POST, "/v1/admin/legal-requests")
        .cookie(cookie)
        .json(body)
}

fn answer_request(cookie: &str, id: &str, body: Value) -> Req {
    req(
        Method::POST,
        format!("/v1/admin/legal-requests/{id}/answer"),
    )
    .cookie(cookie)
    .json(body)
}

fn removal_order(cookie: &str, kind: &str, subject: &str) -> Req {
    req(Method::POST, "/v1/admin/removal-orders")
        .cookie(cookie)
        .json(json!({
            "received_at": rfc3339(OffsetDateTime::now_utc() - time::Duration::minutes(5)),
            "country": "FR", "authority": "OCLCTIC", "order": "Order 2026-118",
            "subject_kind": kind, "subject": subject
        }))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn legal_requests_are_only_for_staff_with_sensitive_rights(db: PgPool) {
    let app = test_app(db).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, sue) = support_staff(&app, "Sue Support").await;
    let (_, pat) = app.user("Pat Player", "US").await;
    let body = json!({
        "received_at": rfc3339(OffsetDateTime::now_utc()), "country": "US",
        "authority": "County Sheriff", "kind": "subpoena", "asked": "Account details."
    });
    let res = app.send(log_request(&ada, body.clone())).await;
    assert_eq!(res.status(), StatusCode::OK);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let answer = json!({ "status": "answered", "response": "Gave the sign-up date.", "user_notice": "notified" });
    // Players don't know it's there; support staff are refused.
    for (cookie, expected) in [(&pat, StatusCode::NOT_FOUND), (&sue, StatusCode::FORBIDDEN)] {
        for r in [
            req(Method::GET, "/v1/admin/legal-requests").cookie(cookie),
            log_request(cookie, body.clone()),
            answer_request(cookie, &id, answer.clone()),
            removal_order(cookie, "app", "castle"),
        ] {
            let uri = r.uri.clone();
            assert_eq!(status(&app, r).await, expected, "{uri}");
        }
    }
    assert_eq!(
        status(&app, req(Method::GET, "/v1/admin/legal-requests")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from legal_requests where status = 'open'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn logging_a_legal_request_checks_it_and_sets_the_deadline(db: PgPool) {
    let app = test_app(db).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (pat, _) = app.user("Pat Player", "US").await;
    let received = OffsetDateTime::now_utc() - time::Duration::hours(1);
    let base = json!({
        "received_at": rfc3339(received), "country": "de", "authority": "Bundeskriminalamt",
        "kind": "emergency", "asked": "Where is the account holder?"
    });
    let with = |changes: Value| {
        let mut body = base.clone();
        body.as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        body
    };
    for (changes, field) in [
        (json!({ "country": "D" }), "country"),
        (json!({ "country": "DEU" }), "country"),
        (json!({ "country": "1A" }), "country"),
        (json!({ "authority": "  " }), "authority"),
        (json!({ "authority": "a".repeat(201) }), "authority"),
        (json!({ "asked": "" }), "asked"),
        (json!({ "asked": "a".repeat(4001) }), "asked"),
        (json!({ "legal_basis": "a".repeat(501) }), "legal_basis"),
        (
            json!({ "subject_user_id": Uuid::now_v7() }),
            "subject_user_id",
        ),
    ] {
        let res = app.send(log_request(&ada, with(changes.clone()))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{changes}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{changes}");
    }
    assert!(
        status(&app, log_request(&ada, with(json!({ "secret": true }))))
            .await
            .is_client_error()
    );
    assert_eq!(count(&app, "select count(*) from legal_requests").await, 0);

    // Each kind's deadline, unless the request gives its own; the person is to be told when
    // there is one.
    let logged = |changes: Value| {
        let app = &app;
        let r = log_request(&ada, with(changes));
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            let id = body_json(res).await["id"].as_str().unwrap().to_string();
            sqlx::query_as::<_, (String, Option<f64>, String)>(
                "select country, (extract(epoch from deadline - received_at) / 3600)::float8, user_notice
                 from legal_requests where id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        logged(json!({})).await,
        ("DE".to_string(), Some(8.0), "not_needed".to_string())
    );
    assert_eq!(
        logged(json!({ "kind": "eu_production_order", "subject_user_id": pat })).await,
        ("DE".to_string(), Some(240.0), "pending".to_string())
    );
    assert_eq!(logged(json!({ "kind": "subpoena" })).await.1, None);
    assert_eq!(
        logged(json!({ "deadline": rfc3339(received + time::Duration::hours(2)) }))
            .await
            .1,
        Some(2.0),
        "an explicit deadline wins"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn answering_a_legal_request_checks_the_outcome(db: PgPool) {
    let app = test_app(db).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (_, pat) = app.user("Pat Player", "US").await;
    let res = app
        .send(log_request(
            &ada,
            json!({
                "received_at": rfc3339(OffsetDateTime::now_utc()), "country": "US",
                "authority": "County Sheriff", "kind": "subpoena", "asked": "Account details."
            }),
        ))
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let good = json!({ "status": "refused", "response": "Not a valid order for us.", "user_notice": "barred" });
    let with = |changes: Value| {
        let mut body = good.clone();
        body.as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        body
    };
    for (changes, field) in [
        (json!({ "status": "maybe" }), "status"),
        (json!({ "status": "open" }), "status"),
        (json!({ "user_notice": "later" }), "user_notice"),
        (json!({ "response": " " }), "response"),
        (json!({ "response": "a".repeat(4001) }), "response"),
    ] {
        let res = app
            .send(answer_request(&ada, &id, with(changes.clone())))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{changes}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{changes}");
    }
    assert_eq!(
        status(&app, answer_request(&pat, &id, good.clone())).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(
            &app,
            answer_request(&ada, &Uuid::now_v7().to_string(), good.clone())
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from legal_requests where status = 'open'"
        )
        .await,
        1
    );
    let res = app.send(answer_request(&ada, &id, good)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let (status_, notice, by, at): (String, String, Option<String>, bool) = sqlx::query_as(
        "select status, user_notice, answered_by, answered_at is not null from legal_requests where id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status_.as_str(), notice.as_str(), at),
        ("refused", "barred", true)
    );
    assert!(by.unwrap().starts_with("Ada Owner"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removal_orders_hide_reviews_profiles_and_teams_and_refuse_what_they_cant(db: PgPool) {
    let app = test_app(db).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (mia, _) = app.user("Mia Maker", "US").await;
    let (rex, _) = app.user("Rex Reviewer", "US").await;
    let (cris, _) = app.user("Cris Creative", "US").await;
    let (tom, _) = app.user("Tom Team", "US").await;
    listed_with(&app, mia, "castle", "released", "").await;
    listed_with(&app, mia, "drafty", "released", "status = 'draft'").await;
    let review = Uuid::now_v7();
    let castle = app_id_of(&app, "castle").await;
    exec(
        &app,
        "insert into app_reviews (id, user_id, app_id, body) values ($1, $2, $3, 'Join our cause.')",
        &[review, rex, castle],
    )
    .await;
    exec(
        &app,
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'cris-art', 'Cris', 'none', false)",
        &[cris],
    )
    .await;
    exec(
        &app,
        "insert into teams (id, slug, name, kind, created_by) values ($1, 'bad-team', 'Bad team', 'player', $2)",
        &[Uuid::now_v7(), tom],
    )
    .await;

    // What it can't take: an unknown kind, a bad country, nothing by that name.
    let res = app.send(removal_order(&ada, "message", "x")).await;
    assert_eq!(body_json(res).await["error"]["field"], "subject_kind");
    let mut bad_country = removal_order(&ada, "team", "bad-team");
    bad_country.body.as_mut().unwrap()["country"] = json!("FRA");
    assert_eq!(
        body_json(app.send(bad_country).await).await["error"]["field"],
        "country"
    );
    assert_eq!(
        body_json(app.send(removal_order(&ada, "profile", "nobody")).await).await["error"]["field"],
        "subject"
    );
    // An app that isn't in the store can't be taken out of it: nothing is logged.
    let res = app.send(removal_order(&ada, "app", "drafty")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(count(&app, "select count(*) from legal_requests").await, 0);
    assert_eq!(count(&app, "select count(*) from removed_content").await, 0);
    assert_eq!(
        count(&app, "select count(*) from moderation_decisions").await,
        0
    );

    for (kind, subject) in [
        ("app_review", review.to_string()),
        ("profile", "CRIS-ART".to_string()),
        ("team", "bad-team".to_string()),
    ] {
        let res = app.send(removal_order(&ada, kind, &subject)).await;
        assert_eq!(res.status(), StatusCode::OK, "{kind}");
    }
    let hidden: (bool, bool, bool) = sqlx::query_as(
        "select (select hidden_at is not null from app_reviews where id = $1),
                (select hidden_reason is not null from creatives where user_id = $2),
                (select hidden_reason is not null from teams where slug = 'bad-team')",
    )
    .bind(review)
    .bind(cris)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(hidden, (true, true, true));
    let decisions: Vec<(Uuid, String)> =
        sqlx::query_as("select user_id, action from moderation_decisions order by created_at")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        decisions,
        [
            (rex, "hide_review".to_string()),
            (cris, "hide_profile".to_string()),
            (tom, "hide_team".to_string())
        ]
    );
    let kept: Vec<String> =
        sqlx::query_scalar("select subject_kind from removed_content order by removed_at")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(kept, ["app_review", "profile", "team"]);
    assert_eq!(
        count(&app, "select count(*) from legal_requests where kind = 'removal_order' and status = 'answered'").await,
        3
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_transparency_report_counts_the_year_so_far(db: PgPool) {
    let app = test_app(db).await;
    let (_, ada) = make_admin(&app, "Ada Owner").await;
    let (mia, mia_cookie) = app.user("Mia Maker", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, mia, "castle", "released", "").await;
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&rita)
                .json(report_app("castle", "scam")),
        )
        .await;
    assert!(res.status().is_success());
    sqlx::query(
        "update reports set status = 'actioned', decided_at = created_at + interval '3 hours'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        status(&app, removal_order(&ada, "app", "castle")).await,
        StatusCode::OK
    );
    let decision = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&mia_cookie))
            .await,
    )
    .await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(&mia_cookie)
                .json(json!({ "appeal": "It's a game about a castle." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let year = OffsetDateTime::now_utc().year();
    let report = body_json(
        app.send(req(Method::GET, format!("/v1/transparency/{year}")))
            .await,
    )
    .await;
    assert_eq!(report["year"], year);
    assert_eq!(report["complete"], false, "the year isn't over");
    assert_eq!(
        report["legal_requests"]["by_kind"],
        json!([{ "key": "removal_order", "count": 1 }])
    );
    assert_eq!(
        report["reports"]["by_reason"],
        json!([{ "key": "scam", "count": 1 }])
    );
    assert_eq!(
        report["reports"]["by_outcome"],
        json!([{ "key": "actioned", "count": 1 }])
    );
    assert_eq!(report["reports"]["median_hours_to_decide"], 3.0);
    assert_eq!(
        report["decisions"]["by_action"],
        json!([{ "key": "delist_app", "count": 1 }])
    );
    assert_eq!(report["decisions"]["automated"], 0);
    let appeals = report["appeals"]["by_outcome"].as_array().unwrap();
    assert_eq!(appeals.len(), 1, "{report}");
    assert_eq!(appeals[0]["count"], 1);
    for not_yet in [year + 1, 2025] {
        assert_eq!(
            status(
                &app,
                req(Method::GET, format!("/v1/transparency/{not_yet}"))
            )
            .await,
            StatusCode::NOT_FOUND,
            "{not_yet}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn legal_deadlines_alert_admins_until_answered(db: PgPool) {
    let app = test_app(db).await;
    let (ada_id, ada) = make_admin(&app, "Ada Owner").await;
    let now = rfc3339(OffsetDateTime::now_utc());
    let mut refs = vec![];
    for (kind, deadline) in [
        ("removal_order", None),
        ("emergency", None),
        (
            "subpoena",
            Some(rfc3339(OffsetDateTime::now_utc() + time::Duration::days(5))),
        ),
    ] {
        let mut body = json!({
            "received_at": now, "country": "DE", "authority": "BKA", "kind": kind, "asked": "Act."
        });
        if let Some(d) = deadline {
            body["deadline"] = json!(d);
        }
        let logged = body_json(app.send(log_request(&ada, body)).await).await;
        refs.push((
            logged["id"].as_str().unwrap().to_string(),
            logged["reference"].as_str().unwrap().to_string(),
        ));
    }
    let legal_alerts = || async {
        let mut alerts: Vec<(String, String, String)> = slop_api::ops::collect(&app.state)
            .await
            .unwrap()
            .into_iter()
            .filter(|a| a.key.starts_with("legal-request-"))
            .map(|a| (a.key, a.title, a.link))
            .collect();
        alerts.sort();
        alerts
    };
    let alerts = legal_alerts().await;
    assert_eq!(alerts.len(), 2, "not the one due in five days: {alerts:?}");
    let removal = &refs[0].1;
    let emergency = &refs[1].1;
    let find = |reference: &str| {
        alerts
            .iter()
            .find(|a| a.0 == format!("legal-request-{reference}"))
            .unwrap()
            .clone()
    };
    assert_eq!(
        find(removal).1,
        format!("Removal order {removal}: act within the hour")
    );
    assert!(
        find(emergency)
            .1
            .starts_with(&format!("Legal request {emergency} is due ")),
        "{:?}",
        find(emergency)
    );
    assert!(alerts.iter().all(|a| a.2 == "/admin?tab=legal"));
    slop_api::ops::run_once(&app.state).await.unwrap();
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and category = 'ops' and link = '/admin?tab=legal'",
    )
    .bind(ada_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 2);

    // Answered: that alert clears.
    let res = app
        .send(answer_request(
            &ada,
            &refs[1].0,
            json!({ "status": "answered", "response": "Gave the last sign-in country.", "user_notice": "barred" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let alerts = legal_alerts().await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].0, format!("legal-request-{removal}"));
    slop_api::ops::run_once(&app.state).await.unwrap();
    let resolved: bool =
        sqlx::query_scalar("select resolved_at is not null from ops_alerts where key = $1")
            .bind(format!("legal-request-{emergency}"))
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(resolved);
}

// ---------------------------------------------------------------------------------------------
// Makers' AI agent tokens

fn new_agent_token(cookie: &str, body: Value) -> Req {
    req(Method::POST, "/v1/me/maker-tokens")
        .cookie(cookie)
        .json(body)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agent_tokens_listed_are_the_live_and_just_expired_ones(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Other", "US").await;
    let mut ids = vec![];
    for name in ["Revoked", "Expired", "Plain"] {
        let res = app
            .send(new_agent_token(
                &mia,
                json!({ "name": name, "scope": "read" }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        ids.push(body_json(res).await["id"].as_str().unwrap().to_string());
    }
    // A publishing token, with no passkey anywhere: nothing about passkeys in it.
    let res = app
        .send(new_agent_token(
            &mia,
            json!({ "name": "Proved", "scope": "publish", "days": 7 }),
        ))
        .await;
    let proved = body_json(res).await;
    assert!(proved.get("passkey_proved").is_none(), "{proved}");
    assert_eq!(
        status(
            &app,
            req(Method::DELETE, format!("/v1/me/maker-tokens/{}", ids[0])).cookie(&mia)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    sqlx::query("update sessions set expires_at = now() - interval '1 second' where id = $1::uuid")
        .bind(&ids[1])
        .execute(&app.db)
        .await
        .unwrap();
    let listed = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&mia))
            .await,
    )
    .await;
    let rows: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(rows, ["Proved", "Plain", "Expired"]);
    // An expired token stays listed for a week, marked expired, so it can be replaced; a removed
    // one doesn't.
    let expired: Vec<bool> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["expired"].as_bool().unwrap())
        .collect();
    assert_eq!(expired, [false, false, true]);
    assert!(listed[0].get("secret").is_none());
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&other))
            .await,
    )
    .await;
    assert_eq!(theirs, json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn making_an_agent_token_checks_its_scope_lifetime_and_count(db: PgPool) {
    let app = test_app(db).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let field = |body: Value| {
        let app = &app;
        let r = new_agent_token(&mia, body);
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
            body_json(res).await["error"]["field"].clone()
        }
    };
    assert_eq!(
        field(json!({ "name": "Claude", "scope": "admin" })).await,
        "scope"
    );
    assert_eq!(
        field(json!({ "name": "Claude", "scope": "" })).await,
        "scope"
    );
    for days in [0, 366, -1] {
        assert_eq!(
            field(json!({ "name": "Claude", "scope": "read", "days": days })).await,
            "days",
            "{days}"
        );
    }
    for name in ["", "   ", &"n".repeat(61)] {
        assert_eq!(
            field(json!({ "name": name, "scope": "read" })).await,
            "name"
        );
    }
    assert_eq!(
        field(json!({ "name": "Claude", "scope": "read", "apps": ["not-mine"] })).await,
        "apps"
    );
    assert_eq!(count(&app, "select count(*) from maker_tokens").await, 0);

    let made = body_json(
        app.send(new_agent_token(
            &mia,
            json!({ "name": "Old proof", "scope": "publish", "days": 1 }),
        ))
        .await,
    )
    .await;
    let days: f64 = sqlx::query_scalar(
        "select (extract(epoch from expires_at - created_at) / 86400)::float8 from sessions where id = $1::uuid",
    )
    .bind(made["id"].as_str().unwrap())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((days - 1.0).abs() < 0.01, "{days}");
    let made = body_json(
        app.send(new_agent_token(
            &mia,
            json!({ "name": "New proof", "scope": "publish" }),
        ))
        .await,
    )
    .await;
    assert!(made["secret"].as_str().unwrap().starts_with("slopmk_"));

    // Ten live at most; revoked and expired ones don't count.
    for n in 2..10 {
        let res = app
            .send(new_agent_token(
                &mia,
                json!({ "name": format!("Agent {n}"), "scope": "read" }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    let res = app
        .send(new_agent_token(
            &mia,
            json!({ "name": "Eleventh", "scope": "read" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    sqlx::query("update sessions set revoked_at = now() where id = $1::uuid")
        .bind(made["id"].as_str().unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(new_agent_token(
            &mia,
            json!({ "name": "Eleventh", "scope": "read" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);

    // An old sign-in makes one too: no step-up (below the limit again).
    age_sessions(&app, mia_id).await;
    sqlx::query("update sessions set revoked_at = now() where user_id = $1 and kind = 'agent'")
        .bind(mia_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(new_agent_token(
            &mia,
            json!({ "name": "Late", "scope": "read" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agent_tokens_are_revoked_only_by_their_maker_and_only_as_tokens(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Other Maker", "US").await;
    let made = body_json(
        app.send(new_agent_token(
            &mia,
            json!({ "name": "Claude", "scope": "read" }),
        ))
        .await,
    )
    .await;
    let id = made["id"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {}", made["secret"].as_str().unwrap());
    let works = || {
        status(
            &app,
            req(Method::GET, "/v1/me").header("authorization", bearer.clone()),
        )
    };
    let revoke = |cookie: &str, id: &str| {
        req(Method::DELETE, format!("/v1/me/maker-tokens/{id}")).cookie(cookie)
    };

    assert_eq!(
        status(&app, revoke(&other, &id)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(works().await, StatusCode::OK);
    // A website session isn't an agent token, even Mia's own.
    let web: String = body_json(
        app.send(req(Method::GET, "/v1/me/sessions").cookie(&mia))
            .await,
    )
    .await
    .as_array()
    .unwrap()
    .iter()
    .find(|s| s["kind"] == "web")
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        status(&app, revoke(&mia, &web)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, req(Method::GET, "/v1/me").cookie(&mia)).await,
        StatusCode::OK
    );
    // The token can't manage tokens itself.
    assert!(
        status(
            &app,
            req(Method::DELETE, format!("/v1/me/maker-tokens/{id}"))
                .header("authorization", bearer.clone())
        )
        .await
        .is_client_error()
    );
    assert_eq!(works().await, StatusCode::OK);

    assert_eq!(
        status(&app, revoke(&mia, &id)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(works().await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(&app, revoke(&mia, &id)).await, StatusCode::NOT_FOUND);
}
