//! End-to-end API tests against a real Postgres (one fresh database per test, via `sqlx::test`)
//! and a fake GitHub server. Needs `DATABASE_URL`; see README.

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::IntoResponse,
    response::Response,
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use slop_api::{
    AppState, admin,
    auth::session,
    config::{CatalogRules, Config, GithubConfig, ReleaseConfig},
    releases, router,
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
    state: AppState,
    /// Base URL of the fake GitHub, which also serves makers' release files.
    fake_base: String,
    signing_key: slop_manifest::ed25519_dalek::SigningKey,
}

/// The bytes the fake release page serves for every build.
const BUILD_BYTES: &[u8] = b"pretend this is a zip of a game";

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
        )
        .route(
            "/octo/game/releases/download/{tag}/{file}",
            get(|| async { BUILD_BYTES }),
        )
        // GitHub Actions' OIDC signing keys (a test key), and a repository for Slop Build setup.
        .route(
            "/.well-known/jwks",
            get(|| async {
                axum::Json(serde_json::from_str::<Value>(include_str!("fixtures/oidc-test-jwks.json")).unwrap())
            }),
        )
        .route("/repos/octo/game", get(|| async { axum::Json(json!({ "default_branch": "trunk" })) }))
        .route(
            "/repos/octo/game/license",
            get(|| async { axum::Json(json!({ "license": { "spdx_id": "MIT", "name": "MIT License" } })) }),
        )
        .route(
            "/repos/octo/odd/license",
            get(|| async { axum::Json(json!({ "license": { "spdx_id": "NOASSERTION", "name": "Other" } })) }),
        )
        // A private repository's release: its asset API redirects to a short-lived signed link.
        .route(
            "/repos/octo/game/releases/tags/{tag}",
            get(|headers: axum::http::HeaderMap| async move {
                let host = headers["host"].to_str().unwrap().to_string();
                axum::Json(json!({ "assets": [{ "name": "game.zip", "url": format!("http://{host}/assets/1") }] }))
            }),
        )
        .route(
            "/assets/1",
            get(|headers: axum::http::HeaderMap| async move {
                let authorized = headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer gh-test-token");
                let accepts_file = headers.get("accept").and_then(|v| v.to_str().ok()) == Some("application/octet-stream");
                if authorized && accepts_file {
                    (StatusCode::FOUND, [("location", "https://objects.example/signed?token=short-lived")]).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        // The repository's releases, for picking one and for the release watcher.
        .route(
            "/repos/octo/game/releases",
            get(|| async {
                axum::Json(json!([
                    { "tag_name": "v1.1.0", "name": "Spring update", "draft": false, "prerelease": false,
                      "published_at": "2026-10-01T00:00:00Z", "html_url": "https://github.com/octo/game/releases/tag/v1.1.0",
                      "assets": [
                          { "name": "game-1.1.0-windows.zip", "size": 10 },
                          { "name": "Game-Setup.exe", "size": 12 },
                          { "name": "checksums.txt", "size": 1 }
                      ] },
                    { "tag_name": "v1.0.0", "name": null, "draft": false, "prerelease": false,
                      "published_at": "2026-09-01T00:00:00Z", "html_url": "https://github.com/octo/game/releases/tag/v1.0.0",
                      "assets": [{ "name": "game-1.0.0-windows.zip", "size": 10 }] },
                    { "tag_name": "v2.0.0", "draft": true, "html_url": "https://github.com/octo/game/releases/tag/v2.0.0" }
                ]))
            }),
        )
        .route(
            "/repos/octo/game/releases/latest",
            get(|| async {
                axum::Json(json!({
                    "tag_name": "v1.1.0", "draft": false, "prerelease": false,
                    "published_at": "2026-10-01T00:00:00Z", "html_url": "https://github.com/octo/game/releases/tag/v1.1.0",
                    "assets": [{ "name": "game-1.1.0-windows.zip", "size": 10 }]
                }))
            }),
        )
        .route(
            "/repos/octo/game/commits/{reference}",
            get(|axum::extract::Path(reference): axum::extract::Path<String>| async move {
                let sha = if reference == "v1.0.0" { "1".repeat(40) } else { "b".repeat(40) };
                axum::Json(json!({ "sha": sha }))
            }),
        )
        .route(
            "/repos/octo/game/contents/",
            get(|| async { axum::Json(json!([{ "name": "Cargo.toml" }, { "name": "src" }])) }),
        )
        // GitHub's attestations API: one repo has an attestation (for some other file), the rest
        // have none.
        .route(
            "/repos/{owner}/{repo}/attestations/{digest}",
            get(
                |axum::extract::Path((owner, _repo, _digest)): axum::extract::Path<(String, String, String)>| async move {
                    if owner == "sigstore" {
                        let bundle: Value = serde_json::from_str(include_str!(
                            "../../slop-provenance/tests/fixtures/github-actions-provenance.sigstore.json"
                        ))
                        .unwrap();
                        (StatusCode::OK, axum::Json(json!({ "attestations": [{ "bundle": bundle }] })))
                    } else {
                        (StatusCode::NOT_FOUND, axum::Json(json!({ "message": "Not Found" })))
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

async fn test_app(db: PgPool) -> TestApp {
    let github_base = fake_github().await;
    let signing_key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
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
        country_header: Some("x-country".into()),
        catalog: CatalogRules::default(),
        releases: ReleaseConfig {
            signing_key: Some(signing_key.clone()),
            max_file_bytes: 1024,
            github_api_base: github_base.parse().unwrap(),
            allow_http_downloads: true,
            require_maker_signature: false,
            scanner: slop_api::scan::Scanner::Off,
            slop_build_workflow: Some("slop/slop-build/.github/workflows/build.yml".into()),
            slop_build_version: "v1".into(),
            github_oidc_issuer: github_base.trim_end_matches('/').to_string(),
            github_token: None,
        },
        rate_limits: slop_api::ratelimit::Limits::default(),
        desktop: Default::default(),
        web_dir: None,
        client_ip_header: Some("x-test-client-ip".into()),
    };
    let state = AppState::new(db.clone(), config.clone()).unwrap();
    TestApp {
        router: router(state.clone()).unwrap(),
        db,
        config,
        state,
        fake_base: github_base,
        signing_key,
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

// ---------------------------------------------------------------------------------------------
// Desktop sign-in

/// Runs the desktop flow up to the token response: the app's request, the website approval and
/// the code exchange. Returns the token JSON.
async fn desktop_sign_in(app: &TestApp, web_cookie: &str) -> Value {
    let verifier = "v".repeat(43);
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/requests").json(json!({
            "code_challenge": pkce_challenge(&verifier),
            "redirect_port": 49152,
            "state": "state-state-state-1",
            "device_name": "Slop for Linux"
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(
        created["approve_url"],
        format!("http://localhost:8080/desktop-signin/{id}")
    );

    let info = body_json(
        app.send(req(Method::GET, format!("/v1/auth/desktop/requests/{id}")).cookie(web_cookie))
            .await,
    )
    .await;
    assert_eq!(info["device_name"], "Slop for Linux");

    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve"),
            )
            .cookie(web_cookie),
        )
        .await;
    let redirect = body_json(res).await["redirect_url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(redirect.starts_with("http://127.0.0.1:49152/callback?"));
    assert_eq!(query_param(&redirect, "state"), "state-state-state-1");
    let code = query_param(&redirect, "code");

    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "authorization_code", "code": code, "code_verifier": verifier
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_sign_in_and_refresh(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    assert_eq!(tokens["expires_in"], 900);
    let access = tokens["access_token"].as_str().unwrap();

    let me = app
        .send(req(Method::GET, "/v1/me").header("authorization", format!("Bearer {access}")))
        .await;
    assert_eq!(body_json(me).await["display_name"], "Octo");
    // The website lists the desktop app as a device.
    let sessions = body_json(
        app.send(req(Method::GET, "/v1/me/sessions").cookie(&web))
            .await,
    )
    .await;
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "desktop" && s["user_agent"] == "Slop for Linux")
    );

    // Refresh rotates both tokens.
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let rotated = body_json(res).await;
    assert_ne!(rotated["refresh_token"], tokens["refresh_token"]);
    let new_access = rotated["access_token"].as_str().unwrap().to_string();
    let ok = app
        .send(req(Method::GET, "/v1/me").header("authorization", format!("Bearer {new_access}")))
        .await;
    assert_eq!(ok.status(), StatusCode::OK);

    // Replaying the old refresh token is treated as theft: the whole session ends.
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let gone = app
        .send(req(Method::GET, "/v1/me").header("authorization", format!("Bearer {new_access}")))
        .await;
    assert_eq!(gone.status(), StatusCode::UNAUTHORIZED);
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": rotated["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_codes_need_the_verifier_and_work_once(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    let verifier = "w".repeat(43);
    let created = body_json(
        app.send(req(Method::POST, "/v1/auth/desktop/requests").json(json!({
            "code_challenge": pkce_challenge(&verifier), "redirect_port": 50000,
            "state": "another-long-state", "device_name": "Laptop"
        })))
        .await,
    )
    .await;
    let id = created["id"].as_str().unwrap();

    // Approving needs a signed-in website session.
    let res = app
        .send(req(
            Method::POST,
            format!("/v1/auth/desktop/requests/{id}/approve"),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve"),
            )
            .cookie(&web),
        )
        .await;
    let code = query_param(
        body_json(res).await["redirect_url"].as_str().unwrap(),
        "code",
    );
    // Approving twice isn't possible.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve"),
            )
            .cookie(&web),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // A wrong verifier burns the code.
    let bad = json!({ "grant_type": "authorization_code", "code": code, "code_verifier": "x".repeat(43) });
    assert_eq!(
        app.send(req(Method::POST, "/v1/auth/desktop/token").json(bad))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let good =
        json!({ "grant_type": "authorization_code", "code": code, "code_verifier": verifier });
    assert_eq!(
        app.send(req(Method::POST, "/v1/auth/desktop/token").json(good))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // Bad requests are rejected up front.
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/requests").json(json!({
            "code_challenge": "short", "redirect_port": 50000, "state": "another-long-state", "device_name": "Laptop"
        })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "code_challenge");
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/requests").json(json!({
            "code_challenge": pkce_challenge(&verifier), "redirect_port": 80, "state": "another-long-state", "device_name": "Laptop"
        })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "redirect_port");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn revoking_a_desktop_device_signs_it_out(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    let sessions = body_json(
        app.send(req(Method::GET, "/v1/me/sessions").cookie(&web))
            .await,
    )
    .await;
    let desktop = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "desktop")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = app
        .send(req(Method::DELETE, format!("/v1/me/sessions/{desktop}")).cookie(&web))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").header("authorization", bearer))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/token").json(json!({
            "grant_type": "refresh_token", "refresh_token": tokens["refresh_token"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_bearer_token_is_not_a_cookie(db: PgPool) {
    let app = test_app(db).await;
    let (_, web) = app.user("Octo", "US").await;
    // A website cookie value sent as a bearer token is refused, and vice versa.
    let cookie_value = web.split_once('=').unwrap().1.to_string();
    let res = app
        .send(req(Method::GET, "/v1/me").header("authorization", format!("Bearer {cookie_value}")))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let tokens = desktop_sign_in(&app, &web).await;
    let res = app
        .send(req(Method::GET, "/v1/me").cookie(format!(
            "slop_session={}",
            tokens["access_token"].as_str().unwrap()
        )))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------------------------
// Releases, review and revocation

/// Review rules for test releases, which have no maker attestation and aren't scanned.
fn lax() -> admin::ReviewRules {
    admin::ReviewRules {
        require_maker_signature: false,
        scanner: slop_api::scan::Scanner::Off,
    }
}

/// A maker with an app whose repo is on the fake release host. Returns (maker cookie, slug).
async fn maker_with_app(app: &TestApp, stage: &str) -> (Uuid, String) {
    let (maker_id, cookie) = app.user("Maker", "US").await;
    let repo = format!("{}octo/game", app.fake_base);
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx)
         values ($1, 'game', 'Game', 'A game', 'game', '{stage}', $2, $3, 'MIT')"
    )))
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .bind(&repo)
    .execute(&app.db)
    .await
    .unwrap();
    (maker_id, cookie)
}

fn release_body(app: &TestApp, version: &str, file: &str) -> Value {
    json!({
        "version": version,
        "commit_sha": "a".repeat(40),
        "files": [{
            "platform": "linux",
            "kind": "archive",
            "url": format!("{}octo/game/releases/download/v1/{file}", app.fake_base),
            "exec": "bin/game",
            "args": ["--fullscreen"]
        }]
    })
}

async fn submit_and_fetch(app: &TestApp, cookie: &str, version: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(cookie)
                .json(release_body(app, version, "game-linux.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["status"], "fetching");
    assert!(releases::fetch_next(&app.state).await.unwrap());
    release["id"].as_str().unwrap().to_string()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn release_review_and_signed_manifest(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;

    // Fetched and hashed, waiting for a person.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["status"], "awaiting_review");
    let expected_sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(BUILD_BYTES));
    assert_eq!(mine[0]["files"][0]["sha256"], expected_sha);
    assert_eq!(mine[0]["files"][0]["size_bytes"], BUILD_BYTES.len());
    assert_eq!(mine[0]["files"][0]["file_name"], "game-linux.zip");
    let pending = admin::pending(&app.db).await.unwrap();
    assert_eq!(pending.len(), 1);

    // Nothing to install yet; the store can't list a beta app without an approved release.
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/manifest").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(admin::list_app(&app.db, "game", "rev").await.is_err());

    let release_id: Uuid = id.parse().unwrap();
    assert!(
        admin::approve(&app.db, Some(&app.signing_key), lax(), release_id, "")
            .await
            .is_err(),
        "reviewer required"
    );
    admin::approve(&app.db, Some(&app.signing_key), lax(), release_id, "rev")
        .await
        .unwrap();
    admin::list_app(&app.db, "game", "rev").await.unwrap();

    // Anyone can now fetch the signed manifest, and it verifies with Slop's key.
    let res = app
        .send(req(Method::GET, "/v1/apps/game/manifest").header("x-country", "US"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let signed: slop_manifest::Signed = serde_json::from_value(body_json(res).await).unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.maker, "Maker");
    assert_eq!(manifest.release_id, id);
    let file = manifest.file_for(slop_manifest::Platform::Linux).unwrap();
    assert_eq!(file.sha256, expected_sha);
    assert_eq!(file.size, BUILD_BYTES.len() as u64);
    assert_eq!(file.launch.as_ref().unwrap().exec, "bin/game");
    assert_eq!(file.launch.as_ref().unwrap().args, ["--fullscreen"]);

    // The public sees approved releases only.
    let public = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(public.as_array().unwrap().len(), 1);

    // Revoking takes it off the manifest endpoint and puts it on the signed revocation list.
    admin::revoke(&app.db, release_id, "rev", "Malware found by re-scan")
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/manifest").header("x-country", "US"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let signed: slop_manifest::Signed = serde_json::from_value(
        body_json(app.send(req(Method::GET, "/v1/revocations")).await).await,
    )
    .unwrap();
    let list: slop_manifest::RevocationList = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::RevocationList,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert_eq!(
        list.is_revoked(&id).unwrap().reason,
        "Malware found by re-scan"
    );
    assert!(!list.is_expired(OffsetDateTime::now_utc()));

    // Every decision is in the audit log, which can't be edited.
    let actions: Vec<String> = sqlx::query_scalar("select action from audit_log order by id")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(actions, ["release.approve", "app.list", "release.revoke"]);
    assert!(
        sqlx::query("delete from audit_log")
            .execute(&app.db)
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_are_checked_on_submit(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let (_, stranger) = app.user("Stranger", "US").await;

    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&stranger)
                .json(release_body(&app, "1.0.0", "x.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let cases: Vec<(&str, Value)> = vec![
        ("version", json!("not a version")),
        ("commit_sha", json!("main")),
    ];
    for (field, value) in cases {
        let mut body = release_body(&app, "1.0.0", "x.zip");
        body[field] = value;
        let res = app
            .send(
                req(Method::POST, "/v1/apps/game/releases")
                    .cookie(&maker)
                    .json(body),
            )
            .await;
        assert_eq!(body_json(res).await["error"]["field"], field);
    }
    for file in [
        json!({ "platform": "linux", "kind": "archive", "url": "https://evil.example/x.zip", "exec": "game" }),
        json!({ "platform": "linux", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.zip", app.fake_base) }),
        json!({ "platform": "linux", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.zip", app.fake_base), "exec": "../../bin/sh" }),
        json!({ "platform": "linux", "kind": "apk", "url": format!("{}octo/game/releases/download/v1/x.apk", app.fake_base) }),
        json!({ "platform": "beos", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.zip", app.fake_base), "exec": "g" }),
    ] {
        let mut body = release_body(&app, "1.0.0", "x.zip");
        body["files"] = json!([file]);
        let res = app
            .send(
                req(Method::POST, "/v1/apps/game/releases")
                    .cookie(&maker)
                    .json(body),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{file}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_missing_file_fails_the_release(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let mut body = release_body(&app, "1.0.0", "x.zip");
    body["files"][0]["url"] = json!(format!("{}octo/game/missing/x.zip", app.fake_base));
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    releases::fetch_next(&app.state).await.unwrap();
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["status"], "failed");
    assert!(
        mine[0]["status_reason"].as_str().unwrap().contains("404"),
        "{}",
        mine[0]["status_reason"]
    );
    // A failed release can't be approved.
    let id: Uuid = mine[0]["id"].as_str().unwrap().parse().unwrap();
    assert!(
        admin::approve(&app.db, Some(&app.signing_key), lax(), id, "rev")
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stages_need_an_approved_release(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "coming_soon").await;
    // A Coming soon page can be listed without a build...
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    // ...but can't move to beta until a release is approved.
    assert!(
        admin::set_stage(&app.db, "game", slop_api::catalog::AppStage::Beta, "rev")
            .await
            .is_err()
    );
    let id = submit_and_fetch(&app, &maker, "0.1.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    admin::set_stage(&app.db, "game", slop_api::catalog::AppStage::Beta, "rev")
        .await
        .unwrap();
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/game").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(page["stage"], "beta");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn library_add_list_remove(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "listed-game", "released", "").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx)
         values ($1, 'secret-draft', 'Draft', 'Draft', 'game', 'beta', $2, 'https://github.com/x/y', 'MIT')",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let (_, player) = app.user("Player", "US").await;

    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/library/listed-game"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/library/listed-game").cookie(&player))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    // Adding twice is fine; someone else's draft can't be added.
    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/library/listed-game").cookie(&player))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/library/secret-draft").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let library = body_json(
        app.send(req(Method::GET, "/v1/me/library").cookie(&player))
            .await,
    )
    .await;
    assert_eq!(library.as_array().unwrap().len(), 1);
    assert_eq!(library[0]["slug"], "listed-game");
    assert!(library[0]["added_at"].is_string());

    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/library/listed-game").cookie(&player))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let library = body_json(
        app.send(req(Method::GET, "/v1/me/library").cookie(&player))
            .await,
    )
    .await;
    assert!(library.as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Maker signatures

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unsigned_builds_cant_be_approved(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;

    // The fetch still works; the missing signature is recorded and shown to the maker.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["status"], "awaiting_review");
    assert!(mine[0]["files"][0]["provenance"].is_null());
    let problem = mine[0]["files"][0]["provenance_problem"].as_str().unwrap();
    assert!(problem.contains("only supported for GitHub"), "{problem}");

    // With signatures required (the default), approval is refused with that reason.
    let err = admin::approve(
        &app.db,
        Some(&app.signing_key),
        admin::ReviewRules {
            require_maker_signature: true,
            ..lax()
        },
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("no verified maker signature"), "{err}");
    let status: String = sqlx::query_scalar("select status::text from releases")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "awaiting_review");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn attestations_are_looked_up_and_checked(db: PgPool) {
    let app = test_app(db).await;
    let sha = "ab".repeat(32);
    let commit = "f0b49a04e5a62250e0f60fb128004a73110fe311";

    // No attestation on GitHub.
    let err = releases::find_attestation(&app.state, "https://github.com/octo/game", commit, &sha)
        .await
        .unwrap_err();
    assert!(err.contains("no build attestation found"), "{err}");

    // A genuine attestation, but for a different file, doesn't count.
    let err = releases::find_attestation(
        &app.state,
        "https://github.com/sigstore/sigstore-js",
        commit,
        &sha,
    )
    .await
    .unwrap_err();
    assert!(err.contains("doesn't name this file's hash"), "{err}");

    // Repositories off GitHub can't be checked yet.
    let err = releases::find_attestation(&app.state, "https://gitlab.com/octo/game", commit, &sha)
        .await
        .unwrap_err();
    assert!(err.contains("only supported for GitHub"), "{err}");
}

// ---------------------------------------------------------------------------------------------
// Rate limits and cleanup

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_in_is_rate_limited_per_client(db: PgPool) {
    let app = test_app(db).await;
    let from = |ip: &str| {
        req(Method::GET, "/v1/auth/github/start").header("x-test-client-ip", ip.to_string())
    };
    for _ in 0..20 {
        assert!(
            app.send(from("203.0.113.7"))
                .await
                .status()
                .is_redirection()
        );
    }
    let res = app.send(from("203.0.113.7")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=3).contains(&retry), "{retry}");
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    // Another client, and reads from the same one, are fine.
    assert!(
        app.send(from("203.0.113.8"))
            .await
            .status()
            .is_redirection()
    );
    let read = app
        .send(req(Method::GET, "/v1/categories").header("x-test-client-ip", "203.0.113.7"))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    // Health checks are never limited.
    assert_ne!(
        app.send(req(Method::GET, "/healthz")).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cleanup_deletes_only_what_has_ended(db: PgPool) {
    let app = test_app(db).await;
    let (_, live) = app.user("Live", "US").await;
    let (_, _) = app.user("Revoked", "US").await;
    let (_, _) = app.user("Idle", "US").await;
    let (_, recent) = app.user("RecentlyRevoked", "US").await;
    sqlx::query("update sessions s set revoked_at = now() - interval '8 days' from users u where u.id = s.user_id and u.display_name = 'Revoked'")
        .execute(&app.db).await.unwrap();
    sqlx::query("update sessions s set last_seen_at = now() - interval '15 days' from users u where u.id = s.user_id and u.display_name = 'Idle'")
        .execute(&app.db).await.unwrap();
    sqlx::query("update sessions s set revoked_at = now() - interval '1 day' from users u where u.id = s.user_id and u.display_name = 'RecentlyRevoked'")
        .execute(&app.db).await.unwrap();
    // An expired OAuth flow and a live one.
    app.send(req(Method::GET, "/v1/auth/github/start")).await;
    app.send(req(Method::GET, "/v1/auth/github/start")).await;
    sqlx::query("update oauth_flows set expires_at = now() - interval '1 minute' where ctid = (select ctid from oauth_flows limit 1)")
        .execute(&app.db).await.unwrap();

    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(deleted.sessions, 2, "{deleted:?}");
    assert_eq!(deleted.oauth_flows, 1);
    let left: i64 = sqlx::query_scalar("select count(*) from sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 2);
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&live))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&recent))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Nothing more to do the second time.
    assert_eq!(
        slop_api::cleanup::run_once(&app.db).await.unwrap(),
        slop_api::cleanup::Deleted::default()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unscanned_files_are_marked_and_the_server_says_so(db: PgPool) {
    let app = test_app(db).await;
    let info = body_json(app.send(req(Method::GET, "/v1/server")).await).await;
    assert_eq!(info["scanning"], false);
    let (_, maker) = maker_with_app(&app, "beta").await;
    submit_and_fetch(&app, &maker, "1.0.0").await;
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["files"][0]["scan_result"], "skipped");
    assert!(
        mine[0]["files"][0]["scan_detail"]
            .as_str()
            .unwrap()
            .contains("scanning is off")
    );

    // A file the scan step never saw can't be approved.
    sqlx::query("update release_files set scan_result = null")
        .execute(&app.db)
        .await
        .unwrap();
    let id: Uuid = mine[0]["id"].as_str().unwrap().parse().unwrap();
    let err = admin::approve(&app.db, Some(&app.signing_key), lax(), id, "rev")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("hasn't been through the scan step"), "{err}");
    sqlx::query("update release_files set scan_result = 'rejected', scan_detail = 'loot box'")
        .execute(&app.db)
        .await
        .unwrap();
    let err = admin::approve(&app.db, Some(&app.signing_key), lax(), id, "rev")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("rejected: loot box"), "{err}");
}

// ---------------------------------------------------------------------------------------------
// Slop Build

const RECIPE_REF: &str = "slop/slop-build/.github/workflows/build.yml@refs/tags/v1";
const BUILD_COMMIT: &str = "1111111111111111111111111111111111111111";

/// A GitHub Actions OIDC token signed with the test key.
fn github_token(app: &TestApp, overrides: Value) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let mut claims = json!({
        "iss": app.config.releases.github_oidc_issuer,
        "aud": "http://localhost:8080",
        "iat": now, "nbf": now - 5, "exp": now + 300,
        "repository": "octo/game",
        "sha": BUILD_COMMIT,
        "job_workflow_ref": RECIPE_REF,
        "ref": "refs/heads/main",
    });
    for (k, v) in overrides.as_object().unwrap() {
        claims[k] = v.clone();
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("fixtures/oidc-test-key.pem"))
        .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

async fn github_app(app: &TestApp) -> (Uuid, String) {
    let (maker_id, cookie) = app.user("Maker", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx)
         values ($1, 'game', 'Game', 'A game', 'game', 'beta', $2, 'https://github.com/octo/game', 'MIT')",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    (maker_id, cookie)
}

fn ci_release() -> Value {
    json!({
        "app": "game", "version": "build.12", "tag": "slop-build-12", "commit": BUILD_COMMIT,
        "files": [
            { "platform": "linux", "file_name": "game-build.12-linux.zip", "exec": "game" },
            { "platform": "windows", "file_name": "game-build.12-windows.zip", "exec": "game.exe" }
        ]
    })
}

async fn submit_ci(app: &TestApp, token: &str, body: Value) -> Response {
    app.send(
        req(Method::POST, "/v1/slop-build/releases")
            .header("authorization", format!("Bearer {token}"))
            .json(body),
    )
    .await
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_submits_releases_with_a_github_token(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = github_app(&app).await;
    let res = submit_ci(&app, &github_token(&app, json!({})), ci_release()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["status"], "fetching");
    assert_eq!(release["version"], "build.12");
    // URLs are built by Slop from the app's repository.
    let urls: Vec<&str> = release["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["url"].as_str().unwrap())
        .collect();
    assert!(urls.contains(
        &"https://github.com/octo/game/releases/download/slop-build-12/game-build.12-linux.zip"
    ));
    let submitter: Uuid = sqlx::query_scalar("select submitted_by from releases")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(submitter, maker_id);
    // The maker sees it like any other release.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_tokens_are_checked(db: PgPool) {
    let app = test_app(db).await;
    github_app(&app).await;
    let cases = [
        // Another workflow in the maker's repo can't submit.
        (
            json!({ "job_workflow_ref": "octo/game/.github/workflows/evil.yml@refs/heads/main" }),
            StatusCode::FORBIDDEN,
        ),
        // A lookalike recipe path.
        (
            json!({ "job_workflow_ref": "slop/slop-build/.github/workflows/build.yml.evil@v1" }),
            StatusCode::FORBIDDEN,
        ),
        // Someone else's repository can't release this app.
        (
            json!({ "repository": "mallory/game" }),
            StatusCode::FORBIDDEN,
        ),
        // A token minted for another site.
        (
            json!({ "aud": "https://other.example" }),
            StatusCode::UNAUTHORIZED,
        ),
        // Expired.
        (
            json!({ "exp": OffsetDateTime::now_utc().unix_timestamp() - 600 }),
            StatusCode::UNAUTHORIZED,
        ),
        // A different commit than the one being released.
        (
            json!({ "sha": "2222222222222222222222222222222222222222" }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (overrides, status) in cases {
        let res = submit_ci(&app, &github_token(&app, overrides.clone()), ci_release()).await;
        assert_eq!(res.status(), status, "{overrides}");
    }
    // No token, a garbage token, or one signed by an unknown key.
    let res = app
        .send(req(Method::POST, "/v1/slop-build/releases").json(ci_release()))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        submit_ci(&app, "not-a-jwt", ci_release()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let mut forged = github_token(&app, json!({}));
    forged.push('x');
    assert_eq!(
        submit_ci(&app, &forged, ci_release()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let count: i64 = sqlx::query_scalar("select count(*) from releases")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_setup_detects_the_project(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = github_app(&app).await;
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let setup = body_json(
        app.send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(setup["supported"], true);
    assert_eq!(setup["project_type"], "rust");
    let workflow = setup["workflow"].as_str().unwrap();
    assert!(
        workflow.contains("uses: slop/slop-build/.github/workflows/build.yml@v1"),
        "{workflow}"
    );
    assert!(workflow.contains("branches: [trunk]"));
    assert!(workflow.contains("app: game"));
    let create = setup["create_url"].as_str().unwrap();
    assert!(
        create.starts_with(
            "https://github.com/octo/game/new/trunk?filename=.github%2Fworkflows%2Fslop.yml&value="
        ),
        "{create}"
    );
}

// ---------------------------------------------------------------------------------------------
// Private repositories

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn private_release_files_get_short_lived_links(db: PgPool) {
    let app = test_app(db).await;
    let url = "https://github.com/octo/game/releases/download/v1/game.zip";
    // Without a GitHub token, files are fetched from their own URL.
    assert_eq!(releases::download_link(&app.state, url).await.unwrap(), url);

    // With one, GitHub's asset API hands out a signed link (and only with the token).
    let mut config = app.config.clone();
    config.releases.github_token = Some("gh-test-token".into());
    let state = AppState::new(app.db.clone(), config).unwrap();
    assert_eq!(
        releases::download_link(&state, url).await.unwrap(),
        "https://objects.example/signed?token=short-lived"
    );
    let missing = releases::download_link(
        &state,
        "https://github.com/octo/game/releases/download/v1/other.zip",
    )
    .await;
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("no file named other.zip")
    );
}

// ---------------------------------------------------------------------------------------------
// Serving the website

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_api_can_serve_the_website(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-web-test-{}", Uuid::now_v7()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>Slop</title>").unwrap();
    std::fs::write(
        dir.join("login.html"),
        "<!doctype html><title>Sign in</title>",
    )
    .unwrap();
    std::fs::write(dir.join("assets/app-1234.js"), "console.log(1)").unwrap();

    let base = test_app(db).await;
    let mut config = base.config.clone();
    config.web_dir = Some(dir.clone());
    let router = router(AppState::new(base.db.clone(), config.clone()).unwrap()).unwrap();
    let app = TestApp { router, ..base };

    let text = |res: Response| async move {
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    };
    let home = app.send(req(Method::GET, "/")).await;
    assert_eq!(home.status(), StatusCode::OK);
    let csp = home.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    assert_eq!(home.headers()["cache-control"], "no-cache");
    assert!(text(home).await.contains("<title>Slop</title>"));
    // Browser-side routes get the app; real files are served as they are.
    assert!(
        text(app.send(req(Method::GET, "/app/some-game")).await)
            .await
            .contains("<title>Slop</title>")
    );
    assert!(
        text(app.send(req(Method::GET, "/login.html")).await)
            .await
            .contains("Sign in")
    );
    let asset = app.send(req(Method::GET, "/assets/app-1234.js")).await;
    assert_eq!(
        asset.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    // The API is unchanged: JSON, strict CSP, and JSON 404s under /v1.
    let api = app.send(req(Method::GET, "/v1/categories")).await;
    assert_eq!(
        api.headers()["content-security-policy"],
        "default-src 'none'; frame-ancestors 'none'"
    );
    let missing = app.send(req(Method::GET, "/v1/nope")).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(missing).await["error"]["code"], "not_found");
    assert_ne!(
        app.send(req(Method::GET, "/healthz")).await.status(),
        StatusCode::NOT_FOUND
    );
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Desktop app updates

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn desktop_builds_are_uploaded_signed_and_offered(db: PgPool) {
    use base64::Engine;
    let dir = std::env::temp_dir().join(format!("slop-desktop-test-{}", Uuid::now_v7()));
    let (secret, public) = slop_api::desktop_updates::generate_updater_key().unwrap();
    let base = test_app(db).await;
    let mut config = base.config.clone();
    config.desktop = slop_api::config::DesktopConfig {
        dir: Some(dir.clone()),
        updater_key: Some(slop_api::desktop_updates::updater_key_from_base64(&secret).unwrap()),
        repo: Some("octo/slop".into()),
    };
    let router = router(AppState::new(base.db.clone(), config).unwrap()).unwrap();
    let app = TestApp { router, ..base };

    let ci = |overrides: Value| {
        let mut claims = json!({
            "repository": "octo/slop",
            "job_workflow_ref": "octo/slop/.github/workflows/desktop.yml@refs/heads/main",
            "sha": BUILD_COMMIT,
        });
        for (k, v) in overrides.as_object().unwrap() {
            claims[k] = v.clone();
        }
        github_token(&app, claims)
    };
    let upload = |token: String, version: &str, bytes: &'static [u8]| {
        let uri =
            format!("/v1/desktop/builds?platform=windows&version={version}&commit={BUILD_COMMIT}");
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::HOST, "localhost:8080")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(Body::from(bytes))
            .unwrap();
        app.router.clone().oneshot(request)
    };

    // Nothing published yet.
    assert_eq!(
        app.send(req(Method::GET, "/v1/desktop/update/windows/x86_64/0.1.0"))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );

    // Only the desktop workflow on main in Slop's repository may upload.
    for bad in [
        json!({ "repository": "mallory/slop", "job_workflow_ref": "mallory/slop/.github/workflows/desktop.yml@refs/heads/main" }),
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/other.yml@refs/heads/main" }),
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/desktop.yml@refs/heads/feature" }),
    ] {
        assert_eq!(
            upload(ci(bad.clone()), "0.1.5", b"x")
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN,
            "{bad}"
        );
    }

    let installer: &'static [u8] = b"pretend this is Slop_0.1.5_x64-setup.exe";
    let res = upload(ci(json!({})), "0.1.5", installer).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    // Versions only go forward.
    assert_eq!(
        upload(ci(json!({})), "0.1.5", b"again")
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        upload(ci(json!({})), "0.1.4", b"older")
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // An older app is offered the update, signed the way Tauri checks it.
    let res = app
        .send(req(Method::GET, "/v1/desktop/update/windows/x86_64/0.1.2"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let update = body_json(res).await;
    assert_eq!(update["version"], "0.1.5");
    let url = update["url"].as_str().unwrap();
    assert_eq!(
        url,
        "http://localhost:8080/v1/desktop/files/Slop_0.1.5_x64-setup.exe"
    );
    let b64 = base64::engine::general_purpose::STANDARD;
    let pk = minisign_verify::PublicKey::decode(
        &String::from_utf8(b64.decode(&public).unwrap()).unwrap(),
    )
    .unwrap();
    let sig_text =
        String::from_utf8(b64.decode(update["signature"].as_str().unwrap()).unwrap()).unwrap();
    let sig = minisign_verify::Signature::decode(&sig_text).unwrap();
    pk.verify(installer, &sig, true).unwrap();
    assert!(sig.trusted_comment().contains("version:0.1.5"));

    // The download is the uploaded file.
    let res = app
        .send(req(
            Method::GET,
            "/v1/desktop/files/Slop_0.1.5_x64-setup.exe",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.into_body().collect().await.unwrap().to_bytes().as_ref(),
        installer
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/desktop/files/..%2F..%2Fetc%2Fpasswd"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Up to date, other platforms and the website's download button.
    assert_eq!(
        app.send(req(Method::GET, "/v1/desktop/update/windows/x86_64/0.1.5"))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/desktop/update/linux/x86_64/0.1.0"))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let latest = body_json(
        app.send(req(Method::GET, "/v1/desktop/latest/windows"))
            .await,
    )
    .await;
    assert_eq!(latest["version"], "0.1.5");
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Licence detection

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn licences_are_detected_from_github(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let detect = |url: &str| {
        req(
            Method::GET,
            format!(
                "/v1/repos/license?url={}",
                url::form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>()
            ),
        )
    };

    assert_eq!(
        app.send(detect("https://github.com/octo/game"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let found = body_json(
        app.send(detect("https://github.com/octo/game").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(found["spdx"], "MIT");
    assert_eq!(found["name"], "MIT License");
    let odd = body_json(
        app.send(detect("https://github.com/octo/odd").cookie(&maker))
            .await,
    )
    .await;
    assert!(odd["spdx"].is_null());
    assert!(odd["note"].as_str().unwrap().contains("couldn't tell"));
    let missing = body_json(
        app.send(detect("https://github.com/octo/none").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        missing["note"]
            .as_str()
            .unwrap()
            .contains("no licence file")
    );
    let gitlab = body_json(
        app.send(detect("https://gitlab.com/octo/game").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        gitlab["note"]
            .as_str()
            .unwrap()
            .contains("GitHub repositories")
    );
    let junk = body_json(app.send(detect("not a url").cookie(&maker)).await).await;
    assert!(junk["spdx"].is_null());
}

// ---------------------------------------------------------------------------------------------
// Admin panel

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_admin_panel_reviews_and_moderates(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let (admin_id, admin) = app.user("Ada Admin", "US").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;

    // Not an admin: the admin API doesn't exist.
    for path in ["/v1/admin/queue", "/v1/admin/apps", "/v1/admin/audit"] {
        assert_eq!(
            app.send(req(Method::GET, path).cookie(&maker))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
        assert_eq!(
            app.send(req(Method::GET, path)).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/admin/releases/{id}/approve")).cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        body_json(app.send(req(Method::GET, "/v1/me").cookie(&admin)).await).await["is_admin"],
        false
    );

    let name = admin::set_admin(&app.db, &admin_id.to_string(), true, "test")
        .await
        .unwrap();
    assert_eq!(name, "Ada Admin");
    assert_eq!(
        body_json(app.send(req(Method::GET, "/v1/me").cookie(&admin)).await).await["is_admin"],
        true
    );

    // The queue shows the release with its files and the app it belongs to.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(queue["releases"][0]["id"], id.as_str());
    assert_eq!(queue["releases"][0]["status"], "awaiting_review");
    assert_eq!(queue["releases"][0]["app"]["slug"], "game");
    assert_eq!(queue["releases"][0]["files"][0]["scan_result"], "skipped");
    assert_eq!(queue["checks"]["signing_key"], true);
    assert!(queue["ready_to_list"].as_array().unwrap().is_empty());
    let apps = body_json(
        app.send(req(Method::GET, "/v1/admin/apps").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(apps[0]["pending_releases"], 1);
    assert!(apps[0]["releases"].as_array().unwrap().is_empty());

    // A beta app can't be listed before a release is approved; the refusal says why.
    let res = app
        .send(req(Method::POST, "/v1/admin/apps/game/list").cookie(&admin))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("approve a release")
    );

    // Approve, then list.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/releases/{id}/approve")).cookie(&admin))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert!(queue["releases"].as_array().unwrap().is_empty());
    assert_eq!(queue["ready_to_list"][0]["slug"], "game");
    assert_eq!(
        app.send(req(Method::POST, "/v1/admin/apps/game/list").cookie(&admin))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game").header("x-country", "US"))
            .await
            .status(),
        StatusCode::OK
    );

    // Moderation: stage, delist, withdraw.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/apps/game/stage")
                .cookie(&admin)
                .json(json!({ "stage": "released" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let apps = body_json(
        app.send(req(Method::GET, "/v1/admin/apps?status=listed").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(apps[0]["stage"], "released");
    assert_eq!(apps[0]["releases"][0]["id"], id.as_str());
    assert_eq!(apps[0]["pending_releases"], 0);
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/releases/{id}/revoke"))
                .cookie(&admin)
                .json(json!({ "reason": "" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "a reason is required");
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/releases/{id}/revoke"))
                .cookie(&admin)
                .json(json!({ "reason": "Test withdrawal" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = app
        .send(
            req(Method::POST, "/v1/admin/apps/game/delist")
                .cookie(&admin)
                .json(json!({ "reason": "Test delisting" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Every action is in the audit log under the admin's name.
    let log = body_json(
        app.send(req(Method::GET, "/v1/admin/audit").cookie(&admin))
            .await,
    )
    .await;
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "app.delist",
            "release.revoke",
            "app.stage",
            "app.list",
            "release.approve",
            "admin.grant"
        ]
    );
    assert!(log[0]["actor"].as_str().unwrap().starts_with("Ada Admin ("));

    // Rights can be taken away again.
    admin::set_admin(&app.db, "Ada Admin", false, "test")
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn linux_updates_follow_the_installed_package_format(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-desktop-test-{}", Uuid::now_v7()));
    let (secret, _) = slop_api::desktop_updates::generate_updater_key().unwrap();
    let key = slop_api::desktop_updates::updater_key_from_base64(&secret).unwrap();
    let base = test_app(db).await;
    let mut config = base.config.clone();
    config.desktop = slop_api::config::DesktopConfig {
        dir: Some(dir.clone()),
        updater_key: Some(key.clone()),
        repo: Some("octo/slop".into()),
    };
    let router = router(AppState::new(base.db.clone(), config).unwrap()).unwrap();
    let app = TestApp { router, ..base };

    let file = dir.join("built.deb");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&file, b"pretend .deb").unwrap();
    let v = semver::Version::parse("0.1.30").unwrap();
    let published = slop_api::desktop_updates::publish(
        &app.db,
        &dir,
        &key,
        "linux-deb",
        &v,
        BUILD_COMMIT,
        &file,
    )
    .await
    .unwrap();
    assert_eq!(published.file_name, "Slop_0.1.30_amd64.deb");
    assert!(
        slop_api::desktop_updates::publish(
            &app.db,
            &dir,
            &key,
            "linux-deb",
            &v,
            BUILD_COMMIT,
            &file
        )
        .await
        .is_err()
    );
    assert!(
        slop_api::desktop_updates::publish(&app.db, &dir, &key, "beos", &v, BUILD_COMMIT, &file)
            .await
            .is_err()
    );

    let check = |q: &str| {
        req(
            Method::GET,
            format!("/v1/desktop/update/linux/x86_64/0.1.2{q}"),
        )
    };
    let deb = body_json(app.send(check("?bundle=deb")).await).await;
    assert_eq!(
        deb["url"],
        "http://localhost:8080/v1/desktop/files/Slop_0.1.30_amd64.deb"
    );
    // A .rpm or AppImage install isn't offered the .deb.
    assert_eq!(
        app.send(check("?bundle=rpm")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(check("?bundle=appimage")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.send(check("")).await.status(), StatusCode::NO_CONTENT);
    // Windows checks ignore the bundle type.
    assert_eq!(
        app.send(req(
            Method::GET,
            "/v1/desktop/update/windows/x86_64/0.1.2?bundle=nsis"
        ))
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let latest = body_json(
        app.send(req(Method::GET, "/v1/desktop/latest/linux-deb"))
            .await,
    )
    .await;
    assert_eq!(latest["version"], "0.1.30");
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Slop Pulse

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pulse_counts_players_and_playtime(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "pulse-game", "released", "").await;
    let (player_id, player) = app.user("Player", "US").await;

    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/pulse-game/launch"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let launch = || async {
        body_json(
            app.send(req(Method::POST, "/v1/apps/pulse-game/launch").cookie(&player))
                .await,
        )
        .await
    };
    let first = launch().await;
    assert_eq!(first["pulse_url"], "http://localhost:8080/v1/pulse");
    assert_eq!(first["heartbeat_seconds"], 60);
    let token = first["token"].as_str().unwrap().to_string();
    let pulse = |path: &str, token: &str| {
        req(Method::POST, format!("/v1/pulse/{path}")).json(json!({ "token": token }))
    };

    // Unknown tokens and heartbeats before start are refused.
    assert_eq!(
        app.send(pulse("start", "nope")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(pulse("heartbeat", &token)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(pulse("start", &token)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(pulse("start", &token)).await.status(),
        StatusCode::NO_CONTENT,
        "retries are fine"
    );

    // A second copy by the same player still counts as one player.
    let second = launch().await["token"].as_str().unwrap().to_string();
    app.send(pulse("start", &second)).await;
    let players = body_json(
        app.send(req(Method::GET, "/v1/apps/pulse-game/players").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(players["playing_now"], 1);

    // Heartbeats add playtime, but not when sent too often.
    assert_eq!(
        app.send(pulse("heartbeat", &token)).await.status(),
        StatusCode::NO_CONTENT
    );
    let playtime = || async {
        sqlx::query_scalar::<_, i64>(
            "select playtime_seconds from user_app_usage where user_id = $1",
        )
        .bind(player_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(playtime().await, 0);
    sqlx::query("update pulse_sessions set last_beat_at = now() - interval '61 seconds' where token_hash = sha256($1::bytea)")
        .bind(token.as_bytes())
        .execute(&app.db)
        .await
        .unwrap();
    app.send(pulse("heartbeat", &token)).await;
    assert!((61..=63).contains(&playtime().await));
    // A long gap is capped.
    sqlx::query("update pulse_sessions set last_beat_at = now() - interval '9 minutes' where token_hash = sha256($1::bytea)")
        .bind(token.as_bytes())
        .execute(&app.db)
        .await
        .unwrap();
    app.send(pulse("heartbeat", &token)).await;
    assert!(
        (241..=245).contains(&playtime().await),
        "{}",
        playtime().await
    );
    let days: i32 =
        sqlx::query_scalar("select days_launched from user_app_usage where user_id = $1")
            .bind(player_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(days, 1, "two launches on one day count as one day");

    // Counts over time.
    slop_api::pulse::sample(&app.db).await.unwrap();
    slop_api::pulse::sample(&app.db).await.unwrap();
    let rows: i64 = sqlx::query_scalar("select count(*) from pulse_counts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 1, "one bucket per app per 5 minutes");
    let players = body_json(
        app.send(req(Method::GET, "/v1/apps/pulse-game/players").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(players["peak_this_month"], 1);
    assert_eq!(players["daily_peaks"][0]["peak"], 1);
    assert_eq!(
        players["daily_peaks"][0]["day"].as_str().unwrap().len(),
        10,
        "YYYY-MM-DD"
    );

    // Ending both copies: nobody's playing; the token is used up.
    app.send(pulse("end", &token)).await;
    app.send(pulse("end", &second)).await;
    let players = body_json(
        app.send(req(Method::GET, "/v1/apps/pulse-game/players").header("x-country", "US"))
            .await,
    )
    .await;
    assert_eq!(players["playing_now"], 0);
    assert_eq!(
        app.send(pulse("heartbeat", &token)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A token that wasn't started in time can't be.
    let late = launch().await["token"].as_str().unwrap().to_string();
    sqlx::query("update pulse_sessions set issued_at = now() - interval '16 minutes' where started_at is null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(pulse("start", &late)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Raw sessions go after a day; the counts and playtime stay.
    sqlx::query("update pulse_sessions set issued_at = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        slop_api::pulse::delete_old_sessions(&app.db).await.unwrap(),
        3
    );
    assert!(playtime().await > 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_can_turn_off_playtime_sharing(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "pulse-game", "released", "").await;
    let (_, player) = app.user("Player", "US").await;
    assert_eq!(
        body_json(app.send(req(Method::GET, "/v1/me").cookie(&player)).await).await["share_playtime"],
        true
    );
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&player)
                .json(json!({ "share_playtime": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let launched = body_json(
        app.send(req(Method::POST, "/v1/apps/pulse-game/launch").cookie(&player))
            .await,
    )
    .await;
    assert!(launched["token"].is_null());
    let sessions: i64 = sqlx::query_scalar("select count(*) from pulse_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(sessions, 0);
}

// ---------------------------------------------------------------------------------------------
// Picking GitHub releases, the release watcher and automatic approval

async fn release_status(app: &TestApp, id: &str) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "select status::text, status_reason, reviewed_by from releases where id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn updates_are_approved_automatically_after_a_person_approves_one(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;

    // The first release waits for a person, and says so.
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    let (status, reason, _) = release_status(&app, &first).await;
    assert_eq!(status, "awaiting_review");
    assert!(reason.unwrap().contains("first release"));
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();

    // The next one passes every check, so Slop approves and signs it.
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    assert_eq!(
        release_status(&app, &second).await,
        ("approved".into(), None, Some("slop".into()))
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["auto_approved"], true);
    assert_eq!(mine[0]["source"], "maker");
    assert_eq!(mine[1]["auto_approved"], false);

    // After a withdrawal, a person looks again.
    admin::revoke(&app.db, second.parse().unwrap(), "rev", "broken build")
        .await
        .unwrap();
    let third = submit_and_fetch(&app, &maker, "1.2.0").await;
    let (status, reason, _) = release_status(&app, &third).await;
    assert_eq!(status, "awaiting_review");
    assert!(reason.unwrap().contains("withdrawn"));
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        third.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();

    // The maker can turn it off.
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/game/update-settings")
                .cookie(&maker)
                .json(json!({ "auto_update": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let settings = body_json(res).await;
    assert_eq!(settings["auto_update"], false);
    assert_eq!(settings["auto_approval_ready"], false);
    let fourth = submit_and_fetch(&app, &maker, "1.3.0").await;
    assert_eq!(release_status(&app, &fourth).await.0, "awaiting_review");

    // Settings are the maker's alone.
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/update-settings").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_pick_a_github_release_and_slop_watches_for_the_next(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = github_app(&app).await;

    // Pick: newest first, drafts left out, files guessed from their names.
    let picks = body_json(
        app.send(req(Method::GET, "/v1/apps/game/github-releases").cookie(&maker))
            .await,
    )
    .await;
    let picks = picks.as_array().unwrap();
    assert_eq!(picks.len(), 2);
    assert_eq!(picks[0]["tag"], "v1.1.0");
    assert_eq!(picks[0]["version"], "1.1.0");
    assert_eq!(picks[0]["submitted"], false);
    let assets = picks[0]["assets"].as_array().unwrap();
    assert_eq!(
        assets[0]["url"],
        "https://github.com/octo/game/releases/download/v1.1.0/game-1.1.0-windows.zip"
    );
    assert_eq!(assets[0]["platform"], "windows");
    assert_eq!(assets[0]["kind"], "archive");
    assert_eq!(assets[1]["kind"], "installer");
    assert!(assets[2]["platform"].is_null());

    // Submitting the older release: the version comes from the tag, the commit from GitHub.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .cookie(&maker)
                .json(json!({
                    "tag": "v1.0.0",
                    "files": [{
                        "platform": "windows", "kind": "archive", "exec": "game.exe",
                        "url": "https://github.com/octo/game/releases/download/v1.0.0/game-1.0.0-windows.zip"
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["version"], "1.0.0");
    assert_eq!(release["commit_sha"], "1".repeat(40));

    // Archives must be zips: the desktop app can't unpack anything else.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .cookie(&maker)
                .json(json!({
                    "tag": "v1.1.0",
                    "files": [{
                        "platform": "linux", "kind": "archive", "exec": "game",
                        "url": "https://github.com/octo/game/releases/download/v1.1.0/game-linux.tar.gz"
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains(".zip")
    );

    // The watcher sees v1.1.0 is newer and submits it from the 1.0.0 pattern.
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 1);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["version"], "1.1.0");
    assert_eq!(mine[0]["source"], "github_release");
    assert_eq!(mine[0]["commit_sha"], "b".repeat(40));
    assert_eq!(mine[0]["files"][0]["file_name"], "game-1.1.0-windows.zip");
    assert_eq!(mine[0]["files"][0]["exec"], "game.exe");

    // Each GitHub release is handled once.
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 0);
    let settings = body_json(
        app.send(req(Method::GET, "/v1/apps/game/update-settings").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(settings["watch_last_tag"], "v1.1.0");
    assert!(settings["watch_problem"].is_null());
    assert!(settings["watch_note"].is_null());

    // Turning watching off stops it.
    sqlx::query("delete from releases where version = '1.1.0'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set watch_releases = false, watch_last_tag = null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 0);
}
