//! End-to-end API tests against a real Postgres (one fresh database per test, via `sqlx::test`)
//! and a fake GitHub server. Needs `DATABASE_URL`; see README.

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use slop_api::{
    AppState,
    auth::session,
    config::{CatalogRules, Config, GithubConfig},
    router,
    tokens::pkce_challenge,
};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Harness

struct TestApp {
    router: Router,
    db: PgPool,
    config: Config,
}

/// A fake GitHub: any code except "bad-code" gives a token for user 42.
async fn fake_github() -> String {
    async fn token(body: String) -> axum::Json<Value> {
        let form: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        let get = |k: &str| {
            form.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        if get("code").as_deref() == Some("bad-code")
            || get("code_verifier").is_none_or(|v| v.len() < 43)
        {
            return axum::Json(json!({ "error": "bad_verification_code" }));
        }
        axum::Json(json!({ "access_token": "gho_test", "token_type": "bearer" }))
    }
    let app = Router::new()
        .route("/login/oauth/access_token", post(token))
        .route(
            "/user",
            get(|| async { axum::Json(json!({ "id": 42, "login": "octo", "name": "Octo Cat" })) }),
        )
        .route(
            "/user/emails",
            get(|| async {
                axum::Json(json!([
                    { "email": "old@example.com", "primary": false, "verified": true },
                    { "email": "octo@example.com", "primary": true, "verified": true }
                ]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

async fn test_app(db: PgPool) -> TestApp {
    let github_base = fake_github().await;
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: String::new(),
        public_url: "http://localhost:8080".parse().unwrap(),
        github: Some(GithubConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            oauth_base: github_base.parse().unwrap(),
            api_base: github_base.parse().unwrap(),
        }),
        trusted_origins: vec![],
        cors_origins: vec!["tauri://localhost".into()],
        country_header: Some("x-country".into()),
        catalog: CatalogRules::default(),
    };
    let state = AppState::new(db.clone(), config.clone()).unwrap();
    TestApp {
        router: router(state).unwrap(),
        db,
        config,
    }
}

struct Req {
    method: Method,
    uri: String,
    cookies: Vec<String>,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}

fn req(method: Method, uri: impl Into<String>) -> Req {
    Req {
        method,
        uri: uri.into(),
        cookies: vec![],
        headers: vec![],
        body: None,
    }
}

impl Req {
    fn cookie(mut self, c: impl Into<String>) -> Self {
        self.cookies.push(c.into());
        self
    }
    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
    fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

impl TestApp {
    async fn send(&self, r: Req) -> Response {
        let mut builder = Request::builder()
            .method(r.method)
            .uri(r.uri)
            .header(header::HOST, "localhost:8080");
        if !r.cookies.is_empty() {
            builder = builder.header(header::COOKIE, r.cookies.join("; "));
        }
        for (name, value) in r.headers {
            builder = builder.header(name, value);
        }
        let body = match r.body {
            Some(json) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        self.router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    /// A signed-in user created directly in the database. Returns the session cookie pair.
    async fn user(&self, name: &str, country: &str) -> (Uuid, String) {
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, $3, now())",
        )
        .bind(id)
        .bind(name)
        .bind(country)
        .execute(&self.db)
        .await
        .unwrap();
        let cookie = session::create(&self.db, &self.config, id, Some("test"))
            .await
            .unwrap();
        (id, format!("{}={}", cookie.name(), cookie.value()))
    }
}

async fn body_json(res: Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// `name=value` of a Set-Cookie header with this name.
fn set_cookie(res: &Response, name: &str) -> Option<String> {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .map(|v| v.split(';').next().unwrap().to_string())
        .filter(|pair| !pair.ends_with('='))
}

fn location(res: &Response) -> String {
    res.headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

fn query_param(url: &str, key: &str) -> String {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == key)
        .unwrap()
        .1
        .into_owned()
}

fn new_app(slug: &str) -> Value {
    json!({
        "slug": slug,
        "title": "Pixel Dungeon Delver",
        "summary": "A tiny roguelike about going down.",
        "description_md": "Explore **procedural** dungeons.",
        "kind": "game",
        "stage": "released",
        "repo_url": "https://github.com/octo/pixel-dungeon",
        "license_spdx": "MIT",
        "min_age": 12,
        "tags": ["roguelike", "retro"]
    })
}

async fn list_slugs(app: &TestApp, r: Req) -> Vec<String> {
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap().to_string())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Identity

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn github_signup_then_signin(db: PgPool) {
    let app = test_app(db).await;

    // Start: redirect to GitHub with PKCE and a state cookie.
    let res = app
        .send(req(Method::GET, "/v1/auth/github/start?return_to=/apps/x"))
        .await;
    assert!(res.status().is_redirection());
    let to_github = location(&res);
    let state = query_param(&to_github, "state");
    assert_eq!(query_param(&to_github, "code_challenge_method"), "S256");
    assert_eq!(
        query_param(&to_github, "redirect_uri"),
        "http://localhost:8080/v1/auth/github/callback"
    );
    let state_cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let verifier: String = sqlx::query_scalar("select pkce_verifier from oauth_flows")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        query_param(&to_github, "code_challenge"),
        pkce_challenge(&verifier)
    );

    // Callback for a new GitHub account: no Slop account yet, so go to sign-up.
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&state_cookie),
        )
        .await;
    assert_eq!(location(&res), "/signup");
    let signup_cookie = set_cookie(&res, "slop_signup").unwrap();
    assert!(set_cookie(&res, "slop_session").is_none());

    // The flow is single use.
    let replay = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&state_cookie),
        )
        .await;
    assert_eq!(replay.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let res = app
        .send(req(Method::GET, "/v1/auth/signup").cookie(&signup_cookie))
        .await;
    let info = body_json(res).await;
    assert_eq!(info["suggested_name"], "Octo Cat");
    assert_eq!(info["email"], "octo@example.com");
    assert!(
        info["countries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["code"] == "GB")
    );

    // Age confirmation and a launch country are required.
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&signup_cookie)
                .json(json!({
                    "display_name": "Octo", "country_code": "US", "age_confirmed": false
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "age_confirmed");
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&signup_cookie)
                .json(json!({
                    "display_name": "Octo", "country_code": "AU", "age_confirmed": true
                })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "country_code");

    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&signup_cookie)
                .json(json!({
                    "display_name": "  Octo  ", "country_code": "us", "age_confirmed": true
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let session_cookie = set_cookie(&res, "slop_session").unwrap();
    let body = body_json(res).await;
    assert_eq!(body["return_to"], "/apps/x");
    assert_eq!(body["user"]["display_name"], "Octo");
    assert_eq!(body["user"]["country_code"], "US");

    let me = body_json(
        app.send(req(Method::GET, "/v1/me").cookie(&session_cookie))
            .await,
    )
    .await;
    assert_eq!(me["email"], "octo@example.com");

    // Signing in again with the same GitHub account goes straight to a session.
    let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
    let state = query_param(&location(&res), "state");
    let state_cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(&state_cookie),
        )
        .await;
    assert_eq!(location(&res), "/");
    assert!(set_cookie(&res, "slop_session").is_some());
    let users: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn callback_rejects_state_from_another_browser(db: PgPool) {
    let app = test_app(db).await;
    let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
    let state = query_param(&location(&res), "state");

    let res = app
        .send(req(
            Method::GET,
            format!("/v1/auth/github/callback?code=ok&state={state}"),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie("slop_oauth_state=other"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn provider_failure_redirects_to_signin(db: PgPool) {
    let app = test_app(db).await;
    let res = app.send(req(Method::GET, "/v1/auth/github/start")).await;
    let state = query_param(&location(&res), "state");
    let state_cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=bad-code&state={state}"),
            )
            .cookie(&state_cookie),
        )
        .await;
    assert_eq!(location(&res), "/signin?error=provider");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sessions_can_be_listed_revoked_and_signed_out(db: PgPool) {
    let app = test_app(db).await;
    let (_, laptop) = app.user("Octo", "US").await;

    assert_eq!(
        app.send(req(Method::GET, "/v1/me")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie("slop_session=forged"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let sessions = body_json(
        app.send(req(Method::GET, "/v1/me/sessions").cookie(&laptop))
            .await,
    )
    .await;
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["current"], true);
    let id = sessions[0]["id"].as_str().unwrap().to_string();

    // Another user can't revoke it.
    let (_, other) = app.user("Mallory", "US").await;
    let res = app
        .send(req(Method::DELETE, format!("/v1/me/sessions/{id}")).cookie(&other))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .send(req(Method::DELETE, format!("/v1/me/sessions/{id}")).cookie(&laptop))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&laptop))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let res = app
        .send(req(Method::POST, "/v1/auth/signout").cookie(&other))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&other))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn idle_sessions_expire(db: PgPool) {
    let app = test_app(db).await;
    let (_, cookie) = app.user("Octo", "US").await;
    sqlx::query("update sessions set last_seen_at = now() - interval '8 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&cookie))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------------------------
// Catalog

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn categories_include_their_tags(db: PgPool) {
    let app = test_app(db).await;
    let body = body_json(app.send(req(Method::GET, "/v1/categories")).await).await;
    let categories = body.as_array().unwrap();
    assert_eq!(categories.len(), 7);
    assert_eq!(categories[0]["slug"], "games");
    assert!(
        categories[0]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["slug"] == "roguelike")
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn drafts_are_private_until_listed(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let (_, stranger) = app.user("Stranger", "US").await;

    assert_eq!(
        app.send(req(Method::POST, "/v1/apps").json(new_app("pixel-dungeon")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("pixel-dungeon")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    assert_eq!(created["status"], "draft");
    assert_eq!(created["is_mine"], true);
    assert_eq!(created["tags"], json!(["retro", "roguelike"]));
    assert_eq!(created["maker"]["display_name"], "Maker");

    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("pixel-dungeon")),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "slug");

    // Not in the store, and invisible to anyone but the maker.
    assert!(
        list_slugs(&app, req(Method::GET, "/v1/apps").header("x-country", "US"))
            .await
            .is_empty()
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/pixel-dungeon").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/pixel-dungeon").cookie(&maker))
            .await
            .status(),
        StatusCode::OK
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/apps").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["slug"], "pixel-dungeon");

    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(req(Method::GET, "/v1/apps/pixel-dungeon").cookie(&stranger))
        .await;
    let page = body_json(res).await;
    assert_eq!(page["is_mine"], false);
    assert!(page.get("status").is_none());
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps").header("x-country", "US")).await,
        ["pixel-dungeon"]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_maker_can_edit(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    app.send(
        req(Method::POST, "/v1/apps")
            .cookie(&maker)
            .json(new_app("pixel-dungeon")),
    )
    .await;

    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/pixel-dungeon")
                .cookie(&stranger)
                .json(json!({ "title": "Mine now" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/pixel-dungeon")
                .cookie(&maker)
                .json(json!({ "repo_url": "http://insecure.example/x" })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "repo_url");
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/pixel-dungeon")
                .cookie(&maker)
                .json(json!({ "stage": "beta" })),
        )
        .await;
    assert!(res.status().is_client_error());

    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/pixel-dungeon")
                .cookie(&maker)
                .json(json!({ "title": "Pixel Dungeon 2", "tags": ["puzzle"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["title"], "Pixel Dungeon 2");
    assert_eq!(body["tags"], json!(["puzzle"]));
    assert_eq!(body["tag_details"][0]["name"], "Puzzle");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn invalid_apps_are_rejected(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let cases = [
        ("slug", json!("No Caps")),
        ("title", json!("")),
        ("summary", json!("x".repeat(201))),
        ("license_spdx", json!("<b>")),
        ("min_age", json!(13)),
        ("tags", json!(["not-a-tag"])),
        ("tags", json!(["cozy", "cozy"])),
    ];
    for (field, value) in cases {
        let mut body = new_app("valid-slug");
        body[field] = value;
        let res = app
            .send(req(Method::POST, "/v1/apps").cookie(&maker).json(body))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(body_json(res).await["error"]["field"], field);
    }
}

/// Inserts a listed app directly, for store-visibility tests. `set` adjusts columns afterwards.
async fn listed_with(app: &TestApp, maker: Uuid, slug: &str, stage: &str, set: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx)
         values ($1, $2, $2, 'An app about ' || $2, 'game', '{stage}', 'listed', $3, 'https://github.com/x/y', 'MIT')"
    )))
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(maker)
    .execute(&app.db)
    .await
    .unwrap();
    if !set.is_empty() {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update apps set {set} where slug = $1"
        )))
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn store_hides_18_plus_in_the_under_18_view(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "for-everyone", "released", "").await;
    listed_with(&app, maker_id, "grown-ups", "released", "min_age = 18").await;
    let (_, uk_user) = app.user("Brit", "GB").await;
    let (_, us_user) = app.user("Yank", "US").await;

    let full = ["for-everyone", "grown-ups"];
    let mut us = list_slugs(
        &app,
        req(Method::GET, "/v1/apps?sort=title").header("x-country", "US"),
    )
    .await;
    us.sort();
    assert_eq!(us, full);
    // Signed out with no country, or from the UK: under-18 view.
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps")).await,
        ["for-everyone"]
    );
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps").header("x-country", "GB")).await,
        ["for-everyone"]
    );
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps").header("x-country", "ZZ")).await,
        ["for-everyone"]
    );
    // Signed in, the account's country wins over the header.
    assert_eq!(
        list_slugs(
            &app,
            req(Method::GET, "/v1/apps")
                .cookie(&uk_user)
                .header("x-country", "US")
        )
        .await,
        ["for-everyone"]
    );
    assert_eq!(
        list_slugs(
            &app,
            req(Method::GET, "/v1/apps?sort=title").cookie(&us_user)
        )
        .await,
        full
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/grown-ups").cookie(&uk_user))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stage_deadlines_and_dead_projects(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "fresh-soon",
        "coming_soon",
        "stage_entered_at = now() - interval '29 days'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "late-soon",
        "coming_soon",
        "stage_entered_at = now() - interval '31 days'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "late-beta",
        "beta",
        "stage_entered_at = now() - interval '366 days'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "quiet-beta",
        "beta",
        "last_commit_at = now() - interval '91 days'",
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "quiet-release",
        "released",
        "last_commit_at = now() - interval '400 days'",
    )
    .await;

    let res = app
        .send(req(Method::GET, "/v1/apps?sort=title").header("x-country", "US"))
        .await;
    let apps = body_json(res).await["apps"].as_array().unwrap().clone();
    let slugs: Vec<&str> = apps.iter().map(|a| a["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["fresh-soon", "quiet-beta", "quiet-release"]);
    let dead = |slug: &str| {
        apps.iter().find(|a| a["slug"] == slug).unwrap()["is_dead"]
            .as_bool()
            .unwrap()
    };
    assert!(!dead("fresh-soon"));
    assert!(dead("quiet-beta"));
    assert!(!dead("quiet-release"), "released apps are never dead");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn search_filter_and_paging(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    for (slug, title, tags) in [
        ("dungeon-a", "Deep Dungeon", vec!["roguelike"]),
        ("dungeon-b", "Dungeon Cook", vec!["cozy", "simulation"]),
        ("notes-c", "Tidy Notes", vec!["note-taking"]),
    ] {
        let mut body = new_app(slug);
        body["title"] = json!(title);
        body["summary"] = json!(format!("{title} for testing"));
        body["description_md"] = json!("");
        body["tags"] = json!(tags);
        assert_eq!(
            app.send(req(Method::POST, "/v1/apps").cookie(&maker).json(body))
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    sqlx::query("update apps set status = 'listed' where maker_id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();

    let get = |q: &str| req(Method::GET, format!("/v1/apps?{q}")).header("x-country", "US");
    let mut found = list_slugs(&app, get("q=dungeon")).await;
    found.sort();
    assert_eq!(found, ["dungeon-a", "dungeon-b"]);
    assert_eq!(list_slugs(&app, get("tag=cozy")).await, ["dungeon-b"]);
    assert_eq!(list_slugs(&app, get("category=tools")).await, ["notes-c"]);
    assert_eq!(
        list_slugs(&app, get("q=dungeon&category=games&tag=roguelike")).await,
        ["dungeon-a"]
    );

    let res = app.send(get("sort=title&limit=2")).await;
    let page = body_json(res).await;
    assert_eq!(page["apps"].as_array().unwrap().len(), 2);
    assert_eq!(page["next_offset"], 2);
    let res = app.send(get("sort=title&limit=2&offset=2")).await;
    let page = body_json(res).await;
    assert_eq!(page["apps"][0]["slug"], "notes-c");
    assert!(page["next_offset"].is_null());
}

// ---------------------------------------------------------------------------------------------
// Web hardening

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cross_site_writes_are_refused(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .header("sec-fetch-site", "cross-site")
                .json(new_app("csrf-app")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .header("origin", "https://evil.example")
                .json(new_app("csrf-app")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .header("sec-fetch-site", "same-origin")
                .json(new_app("csrf-app")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn responses_carry_security_headers(db: PgPool) {
    let app = test_app(db).await;
    let res = app.send(req(Method::GET, "/v1/categories")).await;
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        res.headers()["content-security-policy"],
        "default-src 'none'; frame-ancestors 'none'"
    );
    let res = app.send(req(Method::GET, "/nope")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_app_can_read_cross_origin(db: PgPool) {
    let app = test_app(db).await;
    let res = app
        .send(req(Method::GET, "/v1/categories").header("origin", "tauri://localhost"))
        .await;
    assert_eq!(
        res.headers()["access-control-allow-origin"],
        "tauri://localhost"
    );
    assert!(
        res.headers()
            .get("access-control-allow-credentials")
            .is_none()
    );
    let res = app
        .send(req(Method::GET, "/v1/categories").header("origin", "https://evil.example"))
        .await;
    assert!(res.headers().get("access-control-allow-origin").is_none());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn providers_and_stage_deadlines(db: PgPool) {
    let app = test_app(db).await;
    assert_eq!(
        body_json(app.send(req(Method::GET, "/v1/auth/providers")).await).await["github"],
        true
    );

    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "soon-app",
        "coming_soon",
        "stage_entered_at = date_trunc('second', now()) - interval '10 days'",
    )
    .await;
    listed_with(&app, maker_id, "done-app", "released", "").await;
    let expected: OffsetDateTime = sqlx::query_scalar(
        "select stage_entered_at + interval '30 days' from apps where slug = 'soon-app'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    let res = app.send(req(Method::GET, "/v1/apps/soon-app")).await;
    let deadline = body_json(res).await["stage_deadline"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        OffsetDateTime::parse(&deadline, &Rfc3339).unwrap(),
        expected
    );
    let res = app.send(req(Method::GET, "/v1/apps/done-app")).await;
    assert!(body_json(res).await["stage_deadline"].is_null());
}
