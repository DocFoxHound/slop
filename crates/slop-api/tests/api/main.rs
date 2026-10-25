// SPDX-License-Identifier: AGPL-3.0-or-later
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
    manage, releases, router,
    tokens::pkce_challenge,
};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tower::ServiceExt;
use uuid::Uuid;

// Tests by area (functions listed in the test-coverage inventory). The tests below this harness
// predate the split and stay here.
mod accounts_tests;
mod admin_tests;
mod agents_tests;
mod messages_tests;
mod money_tests;
mod people_tests;
mod pictures_tests;
mod publishing_tests;
mod store_tests;

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

/// A repository's files: MIT-licensed, two npm packages, a checkout link. octo/game has only the
/// licence.
const SCANNED_SOURCE: &[(&str, &str)] = &[
    (
        "LICENSE",
        "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software. The above copyright notice and this permission notice shall be included in all copies.",
    ),
    (
        "package-lock.json",
        r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/lodash":{"version":"4.17.20"},"node_modules/evil-pkg":{"version":"1.0.0"}}}"#,
    ),
    (
        "src/shop.js",
        "export const buy = () => location.assign('https://checkout.stripe.com/c/pay');",
    ),
    (
        "src/net.js",
        "const ws = new WebSocket('wss://play.example.com');",
    ),
];

/// A repository archive as GitHub makes them.
fn source_archive(files: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for &(path, text) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(text.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, format!("octo-scanned-abc/{path}"), text.as_bytes())
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
    out
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
            // A maker's .sig beside a file, when a test has set one (maker keys).
            get(|axum::extract::Path((_, file)): axum::extract::Path<(String, String)>| async move {
                if file == "web.zip" {
                    return web_build().into_response();
                }
                if file.ends_with(".sig") {
                    let sig = MAKER_SIG.lock().unwrap().clone();
                    return if sig.is_empty() { StatusCode::NOT_FOUND.into_response() } else { sig.into_response() };
                }
                BUILD_BYTES.into_response()
            }),
        )
        // GitHub Actions' OIDC signing keys (a test key), and a repository for Slop Build setup.
        .route(
            "/.well-known/jwks",
            get(|| async {
                axum::Json(serde_json::from_str::<Value>(include_str!("../fixtures/oidc-test-jwks.json")).unwrap())
            }),
        )
        .route("/repos/octo/game", get(|| async { axum::Json(json!({ "default_branch": "trunk", "id": 1001, "owner": { "id": 42 } })) }))
        // The source archive Slop's source check reads (source_scan.rs), and OSV (dep_watch.rs).
        .route("/repos/octo/scanned/tarball/{sha}", get(|| async { source_archive(SCANNED_SOURCE) }))
        .route("/repos/octo/game/tarball/{sha}", get(|| async { source_archive(&SCANNED_SOURCE[..1]) }))
        .route(
            "/v1/querybatch",
            post(|axum::Json(body): axum::Json<Value>| async move {
                let results: Vec<Value> = body["queries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|q| match q["package"]["name"].as_str() {
                        Some("lodash") => json!({ "vulns": [{ "id": "GHSA-lodash" }] }),
                        Some("evil-pkg") => json!({ "vulns": [{ "id": "MAL-2026-1" }] }),
                        _ => json!({}),
                    })
                    .collect();
                axum::Json(json!({ "results": results }))
            }),
        )
        .route(
            "/v1/vulns/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
                axum::Json(json!({
                    "id": id,
                    "summary": format!("{id} summary"),
                    "affected": [{ "package": { "name": "lodash" }, "ranges": [{ "events": [{ "introduced": "0" }, { "fixed": "4.17.21" }] }] }]
                }))
            }),
        )
        // Repository checks (frozen.rs): a public repository, a private one, and Git's discovery
        // request for a repository on another host.
        .route("/repos/octo/open-tool", get(|| async { axum::Json(json!({ "private": false })) }))
        .route("/repos/octo/closed-tool", get(|| async { axum::Json(json!({ "private": true })) }))
        .route("/octo/game/info/refs", get(|| async { "001e# service=git-upload-pack\n" }))
        // Google and GitLab sign-in (auth/oauth.rs): the code is the token, and names the account.
        .route("/google/token", post(|body: String| async move { axum::Json(json!({ "access_token": form_code(&body) })) }))
        .route("/gitlab/token", post(|body: String| async move { axum::Json(json!({ "access_token": form_code(&body) })) }))
        // A private repository on a Gitea host (repo_tokens.rs): only the token "token good" reads
        // it; its release file redirects (with the token) to short-lived file storage elsewhere.
        .route(
            "/api/v1/repos/ann/secret-game",
            get(|headers: axum::http::HeaderMap| async move { gitea_auth(&headers, axum::Json(json!({ "private": true }))) }),
        )
        .route(
            "/api/v1/repos/ann/secret-game/contents",
            get(|headers: axum::http::HeaderMap| async move {
                gitea_auth(&headers, axum::Json(json!([{ "name": "README.md", "type": "file" }, { "name": "src", "type": "dir" }])))
            }),
        )
        .route(
            "/api/v1/repos/ann/secret-game/raw/README.md",
            get(|headers: axum::http::HeaderMap| async move {
                gitea_auth(&headers, format!("# Secret game\n\n{}\n", PROOF.lock().unwrap()))
            }),
        )
        .route(
            "/api/v1/repos/ann/secret-game/archive/{file}",
            get(|headers: axum::http::HeaderMap| async move { gitea_auth(&headers, source_archive(&SCANNED_SOURCE[..1])) }),
        )
        // The same repository on GitLab's API (the fake server is "localhost" for GitLab).
        .route(
            "/api/v4/projects/{id}/repository/archive.tar.gz",
            get(|axum::extract::Path(id): axum::extract::Path<String>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                if id != "octo/scanned" || q.get("sha").is_none_or(|s| s.len() != 40) {
                    return StatusCode::NOT_FOUND.into_response();
                }
                source_archive(SCANNED_SOURCE).into_response()
            }),
        )
        .route(
            "/ann/secret-game/releases/download/v1/game.zip",
            get(|headers: axum::http::HeaderMap| async move {
                let port = headers["host"].to_str().unwrap().rsplit(':').next().unwrap().to_string();
                gitea_auth(
                    &headers,
                    axum::response::Redirect::to(&format!("http://localhost:{port}/storage/game.zip?expires=soon")),
                )
            }),
        )
        .route(
            "/storage/game.zip",
            get(|headers: axum::http::HeaderMap| async move {
                assert!(headers.get("authorization").is_none(), "the token never leaves the repository's host");
                BUILD_BYTES
            }),
        )
        // Microsoft and Discord take the app's secret in the form.
        .route(
            "/microsoft/token",
            post(|body: String| async move {
                if !body.contains("client_secret=secret") || !body.contains("code_verifier=") {
                    return axum::Json(json!({ "error": "invalid_client" }));
                }
                axum::Json(json!({ "access_token": form_code(&body) }))
            }),
        )
        .route(
            "/discord/token",
            post(|body: String| async move {
                if !body.contains("client_secret=secret") {
                    return axum::Json(json!({ "error": "invalid_client" }));
                }
                axum::Json(json!({ "access_token": form_code(&body) }))
            }),
        )
        .route(
            "/microsoft/oidc/userinfo",
            get(|headers: axum::http::HeaderMap| async move {
                let token = bearer(&headers);
                axum::Json(json!({ "sub": format!("ms-{token}"), "name": "Mia Soft", "email": "anyone@example.com" }))
            }),
        )
        .route(
            "/discord/users/@me",
            get(|headers: axum::http::HeaderMap| async move {
                let token = bearer(&headers);
                axum::Json(json!({ "id": "80351110224678912", "username": token, "global_name": "Dee", "email": "dee@example.com", "verified": true }))
            }),
        )
        .route(
            "/google/userinfo",
            get(|headers: axum::http::HeaderMap| async move {
                let token = bearer(&headers);
                axum::Json(json!({ "sub": token, "name": format!("Google {token}"), "email": format!("{token}@example.com"), "email_verified": true }))
            }),
        )
        .route(
            "/gitlab/api/user",
            get(|headers: axum::http::HeaderMap| async move {
                let (id, username) = match bearer(&headers).as_str() {
                    "gl-ann" => (501, "ann"),
                    "gl-bob" => (502, "bob"),
                    _ => (599, "someone"),
                };
                axum::Json(json!({ "id": id, "username": username, "name": username, "email": format!("{username}@example.com"), "confirmed_at": "2026-01-01T00:00:00Z" }))
            }),
        )
        // GitLab projects (repo_owner.rs): ann's own, and a group's where ann (501) is an Owner.
        .route(
            "/gitlab/api/projects/{path}",
            get(|axum::extract::Path(path): axum::extract::Path<String>| async move {
                match path.as_str() {
                    "ann/game" => axum::Json(json!({ "id": 9, "namespace": { "kind": "user", "full_path": "ann" }, "owner": { "id": 501 } })).into_response(),
                    "studio/game" => axum::Json(json!({ "id": 10, "namespace": { "kind": "group", "full_path": "studio" } })).into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/gitlab/api/projects/{id}/members/all/{user}",
            get(|axum::extract::Path((id, user)): axum::extract::Path<(i64, i64)>| async move {
                if id == 10 && user == 501 {
                    axum::Json(json!({ "access_level": 50 })).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        // Push services (push.rs): one that takes pushes, one that says the subscription is gone.
        .route(
            "/push/ok",
            post(|headers: axum::http::HeaderMap| async move {
                PUSHES.lock().unwrap().push(
                    headers["authorization"].to_str().unwrap().to_string(),
                );
                StatusCode::CREATED
            }),
        )
        .route("/push/gone", post(|| async { StatusCode::GONE }))
        // The EU's VIES service (donations.rs): one valid German number.
        .route(
            "/vies",
            post(|axum::Json(b): axum::Json<Value>| async move {
                axum::Json(json!({ "valid": b["countryCode"] == "DE" && b["vatNumber"] == "123456789" }))
            }),
        )
        // Picture hosts for profile pictures (avatars.rs, pictures_tests.rs): GitHub's account
        // pictures at /avatars/u/<id>, and any other host's at /pics/<name>.
        .route("/avatars/{*path}", get(pictures_tests::serve_picture))
        .route("/pics/{*path}", get(pictures_tests::serve_picture))
        // GitHub's raw files (media.rs): whatever a test put in RAW_FILES.
        .route(
            "/raw/{*path}",
            get(|axum::extract::Path(path): axum::extract::Path<String>| async move {
                match RAW_FILES.lock().unwrap().get(&path) {
                    Some(bytes) => (StatusCode::OK, bytes.clone()).into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
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
        // Answers "not modified" to a request carrying its ETag, as GitHub does.
        .route(
            "/repos/octo/game/releases/latest",
            get(|headers: axum::http::HeaderMap| async move {
                let etag = "\"latest-v1.1.0\"";
                if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(etag) {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
                (
                    [("etag", etag)],
                    axum::Json(json!({
                        "tag_name": "v1.1.0", "draft": false, "prerelease": false,
                        "published_at": "2026-10-01T00:00:00Z", "html_url": "https://github.com/octo/game/releases/tag/v1.1.0",
                        "assets": [{ "name": "game-1.1.0-windows.zip", "size": 10 }]
                    })),
                )
                    .into_response()
            }),
        )
        // Slop's GitHub App: installation 77 gets a token, which one private repository needs.
        .route(
            "/app/installations/{id}/access_tokens",
            post(|axum::extract::Path(id): axum::extract::Path<i64>, headers: axum::http::HeaderMap| async move {
                let signed = headers.get("authorization").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("Bearer ey"));
                if id != 77 || !signed {
                    return StatusCode::NOT_FOUND.into_response();
                }
                (StatusCode::CREATED, axum::Json(json!({ "token": "ghs_install", "expires_at": "2099-01-01T00:00:00Z" }))).into_response()
            }),
        )
        .route(
            "/repos/octo/hooked/license",
            get(|headers: axum::http::HeaderMap| async move {
                if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer ghs_install") {
                    axum::Json(json!({ "license": { "spdx_id": "MIT", "name": "MIT License" } })).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        // Official teams (teams.rs): the "acme" organization has Slop's App installed.
        .route(
            "/orgs/{org}/installation",
            get(|axum::extract::Path(org): axum::extract::Path<String>| async move {
                if org == "acme" {
                    (StatusCode::OK, axum::Json(json!({ "id": 77 }))).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/user/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<i64>| async move {
                let login = match id { 42 => "octo", 43 => "bob", _ => "someone" };
                axum::Json(json!({ "id": id, "login": login }))
            }),
        )
        // Who owns a repository (repo_owner.rs): octo is account 42, bob 43; acme and globex are
        // organizations (only acme has Slop's App). Deleted repositories aren't there.
        .route(
            "/repos/{owner}/{repo}",
            get(|axum::extract::Path((owner, repo)): axum::extract::Path<(String, String)>| async move {
                if repo.contains("deleted") || repo.contains("gone") {
                    return StatusCode::NOT_FOUND.into_response();
                }
                let (id, kind) = match owner.as_str() {
                    "octo" => (42, "User"),
                    "bob" => (43, "User"),
                    "acme" => (900, "Organization"),
                    "globex" => (901, "Organization"),
                    _ => (7, "User"),
                };
                axum::Json(json!({ "private": false, "id": 5000 + id, "owner": { "login": owner, "id": id, "type": kind } })).into_response()
            }),
        )
        .route(
            "/orgs/{org}/memberships/{login}",
            get(|axum::extract::Path((org, login)): axum::extract::Path<(String, String)>| async move {
                if org == "acme" && login == "octo" {
                    axum::Json(json!({ "role": "admin", "state": "active" })).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/orgs/{org}",
            get(|| async { axum::Json(json!({ "login": "acme", "is_verified": false, "two_factor_requirement_enabled": true })) }),
        )
        .route(
            "/dns-query",
            get(|| async {
                let txt = DNS_TXT.lock().unwrap().clone();
                axum::Json(json!({ "Answer": txt.map(|t| vec![json!({ "data": format!("\"{t}\"") })]).unwrap_or_default() }))
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
                            "../../../slop-provenance/tests/fixtures/github-actions-provenance.sigstore.json"
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
    test_app_with(db, |_| {}).await
}

async fn test_app_with(db: PgPool, adjust: impl FnOnce(&mut Config)) -> TestApp {
    let github_base = fake_github().await;
    let signing_key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: String::new(),
        migrate_database_url: None,
        bad_link_feeds: vec![],
        public_url: "http://localhost:8080".parse().unwrap(),
        github: Some(GithubConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            oauth_base: github_base.parse().unwrap(),
            api_base: github_base.parse().unwrap(),
        }),
        google: None,
        gitlab: None,
        bitbucket: None,
        microsoft: None,
        discord: None,
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
            github_app: None,
            // Tests users have no GitHub accounts; repo_owner tests switch it on.
            require_repo_owner: false,
            repo_token_key: Some([9; 32]),
            osv_api_base: github_base.parse().unwrap(),
            // The fake server is also "localhost", standing in for GitLab (source_scan.rs).
            source_hosts: vec![("localhost".into(), slop_api::repo_tokens::Kind::Gitlab)],
        },
        rate_limits: slop_api::ratelimit::Limits::default(),
        account_limit_scale: 1,
        desktop: Default::default(),
        demos: Default::default(),
        media: slop_api::config::MediaConfig {
            github_raw_base: format!("{github_base}raw/").parse().unwrap(),
            // Profile pictures (avatars.rs) come from the fake GitHub on this machine.
            github_avatar_base: format!("{github_base}avatars/").parse().unwrap(),
            avatar_test_hosts: vec!["127.0.0.1".into()],
        },
        payments: Default::default(),
        email: None,
        push: None,
        doh_url: format!("{github_base}dns-query").parse().unwrap(),
        web_dir: None,
        client_ip_header: Some("x-test-client-ip".into()),
        simulation: false,
        ops: Default::default(),
        approvals_required: 1,
        field_key: Some([11; 32]),
        security_contact: Some("mailto:security@slop.example".into()),
        contact_email: Some("hello@slop.example".into()),
        source_url: Some("https://github.com/slop/slop".into()),
    };
    let mut config = config;
    adjust(&mut config);
    slop_api::legal::sync(&db).await.unwrap();
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
        // Test users have accepted every agreement in effect, unless a test takes it back.
        sqlx::query(
            "insert into legal_acceptances (user_id, kind, version)
             select $1, kind, version from legal_documents where kind in ('terms', 'publisher', 'people')",
        )
        .bind(id)
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
        "repo_url": format!("https://github.com/octo/{slug}"),
        "license_spdx": "MIT",
        "min_age": 12,
        "tags": ["roguelike", "retro"],
        "platforms": ["windows", "linux", "linux-deb"]
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
    // The Terms of Service are accepted at sign-up.
    assert_eq!(body_json(res).await["error"]["field"], "accept_terms");
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup")
                .cookie(&signup_cookie)
                .json(json!({
                    "display_name": "  Octo  ", "country_code": "us", "age_confirmed": true,
                    "accept_terms": true
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
    let accepted: Vec<String> =
        sqlx::query_scalar("select kind from legal_acceptances order by kind")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(accepted, ["terms"]);

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
async fn a_taken_store_address_comes_with_a_free_one(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let (_, other) = app.user("Other", "US").await;
    let check =
        |slug: &str, who: &str| req(Method::GET, format!("/v1/apps/{slug}/available")).cookie(who);

    // Signed in only, and only well-formed addresses.
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/herds/available"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let res = app.send(check("Bad_Address", &maker)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let free = body_json(app.send(check("angryherds", &maker)).await).await;
    assert_eq!(
        free,
        json!({"slug": "angryherds", "available": true, "suggestion": null})
    );

    for slug in ["angryherds", "angryherds-2"] {
        let res = app
            .send(
                req(Method::POST, "/v1/apps")
                    .cookie(&maker)
                    .json(new_app(slug)),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    // Taken: the next free one, whether asked about the address or a numbered copy of it.
    let taken = body_json(app.send(check("angryherds", &other)).await).await;
    assert_eq!(taken["available"], false);
    assert_eq!(taken["suggestion"], "angryherds-3");
    let taken = body_json(app.send(check("angryherds-2", &other)).await).await;
    assert_eq!(taken["suggestion"], "angryherds-3");

    // Creating it anyway says the same, so a maker's agent can pick the free one.
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&other).json({
            let mut again = new_app("angryherds");
            again["repo_url"] = json!("https://github.com/other/angryherds");
            again
        }))
        .await;
    let body = body_json(res).await;
    assert_eq!(body["error"]["field"], "slug");
    assert_eq!(
        body["error"]["message"],
        "that address is taken; angryherds-3 is free"
    );

    // A long address stays within 60 characters.
    let long = "a".repeat(60);
    app.send(
        req(Method::POST, "/v1/apps")
            .cookie(&maker)
            .json(new_app(&long)),
    )
    .await;
    let taken = body_json(app.send(check(&long, &other)).await).await;
    assert_eq!(taken["suggestion"], format!("{}-2", "a".repeat(58)));
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
        .send(req(Method::POST, "/v1/apps").cookie(&maker).json({
            let mut again = new_app("pixel-dungeon");
            again["repo_url"] = json!("https://github.com/octo/another-repo");
            again
        }))
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
                .json(json!({ "repo_url": "ftp://insecure.example/x" })),
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
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'game', 'Game', 'A game', 'game', '{stage}', $2, $3, 'MIT', '{{windows}}')"
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
            "platform": "windows",
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
                .json(release_body(app, version, "game-windows.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["status"], "fetching");
    assert!(releases::fetch_next(&app.state).await.unwrap());
    release["id"].as_str().unwrap().to_string()
}

/// Slop's source check reads the repository at the release commit, and the daily dependency
/// watch tells the maker about advisories published since (but not ones the build showed).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn source_checks_and_the_dependency_watch(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let app_id = Uuid::now_v7();
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'scanned', 'Scanned', 'A game', 'game', 'beta', $2, $3, 'MIT', '{windows}')",
    )
    .bind(app_id)
    .bind(maker_id)
    .bind(format!("{}octo/scanned", app.fake_base))
    .execute(&app.db)
    .await
    .unwrap();
    let release_id = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified, signed_manifest, source_checks)
         values ($1, $2, '1.0.0', repeat('c', 40), 'approved', $3, now(), true, '{}', $4)",
    )
    .bind(release_id)
    .bind(app_id)
    .bind(maker_id)
    .bind(json!({ "secrets": [], "vulnerabilities": [{ "id": "GHSA-lodash", "package": "lodash" }] }))
    .execute(&app.db)
    .await
    .unwrap();

    assert_eq!(
        slop_api::source_scan::run_once(&app.state).await.unwrap(),
        1
    );
    assert_eq!(
        slop_api::source_scan::run_once(&app.state).await.unwrap(),
        0,
        "checked once"
    );
    let scan: Value = sqlx::query_scalar("select repo_scan from releases where id = $1")
        .bind(release_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(scan["status"], "checked", "{scan}");
    assert_eq!(scan["licence"]["files"][0]["spdx"], "MIT");
    assert_eq!(scan["licence"]["problems"], json!([]));
    assert_eq!(scan["packages"], 2);
    assert_eq!(scan["monetization"][0]["rule"], "checkout_link");
    assert_eq!(scan["monetization"][0]["examples"][0], "src/shop.js:1");
    // It goes online, but its store page doesn't say so.
    assert_eq!(scan["network"][0]["examples"][0], "src/net.js:1");
    assert_eq!(scan["online_undeclared"], true);

    assert_eq!(slop_api::dep_watch::run_once(&app.state).await.unwrap(), 1);
    assert_eq!(
        slop_api::dep_watch::run_once(&app.state).await.unwrap(),
        0,
        "once a day"
    );
    let notices: Vec<(String, String)> = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and title like 'New security advisories%'",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].1.contains("evil-pkg 1.0.0 (MAL-2026-1"),
        "{}",
        notices[0].1
    );
    assert!(
        !notices[0].1.contains("lodash"),
        "the build already showed that one"
    );
    let signal: String = sqlx::query_scalar("select kind from abuse_signals where subject_id = $1")
        .bind(app_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(signal, "malicious_dependency");

    // The maker sees both on the Releases page.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/scanned/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["repo_scan"]["status"], "checked");
    let advisories = mine[0]["advisories"].as_array().unwrap();
    assert_eq!(advisories.len(), 2);
    assert_eq!(advisories[0]["vuln_id"], "MAL-2026-1", "malicious first");
    let lodash = advisories
        .iter()
        .find(|a| a["package"] == "lodash")
        .unwrap();
    assert_eq!(lodash["fixed"], "4.17.21");
    assert_eq!(lodash["known_at_build"], true);

    // An update whose source check finds something the approved release didn't have waits for
    // a person instead of being approved automatically.
    let update = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, repo_scan)
         values ($1, $2, '1.1.0', repeat('d', 40), 'awaiting_review', $3, $4)",
    )
    .bind(update)
    .bind(app_id)
    .bind(maker_id)
    .bind(&scan)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        slop_api::source_scan::new_concerns(&app.db, update)
            .await
            .unwrap(),
        None,
        "same as before"
    );
    let mut with_loot = scan.clone();
    with_loot["monetization"].as_array_mut().unwrap().push(
        json!({ "rule": "loot_box", "label": "Loot boxes or gacha", "count": 1, "examples": [] }),
    );
    sqlx::query("update releases set repo_scan = $2 where id = $1")
        .bind(update)
        .bind(&with_loot)
        .execute(&app.db)
        .await
        .unwrap();
    let why = slop_api::source_scan::new_concerns(&app.db, update)
        .await
        .unwrap()
        .unwrap();
    assert!(why.contains("loot boxes"), "{why}");
    sqlx::query("update releases set repo_scan = '{\"status\": \"failed\"}' where id = $1")
        .bind(update)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(
        slop_api::source_scan::new_concerns(&app.db, update)
            .await
            .unwrap()
            .is_some()
    );

    // The maker says what it connects to; players see it on the store page.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/scanned")
                .cookie(&maker)
                .json(json!({ "online": ["own_servers", "player_servers"], "online_hosts": ["Play.Example.com:7777"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/scanned").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(page["online"], json!(["own_servers", "player_servers"]));
    assert_eq!(page["online_hosts"], json!(["play.example.com:7777"]));
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/scanned")
                .cookie(&maker)
                .json(json!({ "online_hosts": ["http://evil"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // GitLab: the same repository through GitLab's archive API.
    sqlx::query("update apps set repo_url = $2 where id = $1")
        .bind(app_id)
        .bind(format!(
            "{}octo/scanned",
            app.fake_base.replace("127.0.0.1", "localhost")
        ))
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update releases set repo_scanned_at = null where id = $1")
        .bind(release_id)
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::source_scan::run_once(&app.state).await.unwrap();
    let scan: Value = sqlx::query_scalar("select repo_scan from releases where id = $1")
        .bind(release_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(scan["status"], "checked", "{scan}");
    assert_eq!(scan["licence"]["files"][0]["spdx"], "MIT", "{scan}");
    assert!(scan.get("online_undeclared").is_none(), "declared now");

    // Repositories Slop can't read are recorded as not checked.
    sqlx::query("update apps set repo_url = 'https://bitbucket.org/octo/scanned' where id = $1")
        .bind(app_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update releases set repo_scanned_at = null where id = $1")
        .bind(release_id)
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::source_scan::run_once(&app.state).await.unwrap();
    let status: String =
        sqlx::query_scalar("select repo_scan ->> 'status' from releases where id = $1")
            .bind(release_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "not_checked");
}

/// A file over GitHub's 2 GiB limit, released in numbered parts: Slop fetches and hashes each,
/// and the manifest lists them for the desktop app to download and join.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn release_files_in_parts(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let base = format!("{}octo/game/releases/download/v1", app.fake_base);
    let body = |parts: Vec<String>| {
        let mut b = release_body(&app, "1.0.0", "game-windows.zip");
        b["files"][0]["parts"] = json!(parts);
        b
    };
    for bad in [
        vec![format!("{base}/game-windows.zip.001")],
        vec![
            format!("{base}/game-windows.zip.001"),
            format!("{base}/game-windows.zip.003"),
        ],
        vec![
            format!("{base}/game-windows.zip.001"),
            format!("{base}/other.zip.002"),
        ],
    ] {
        let res = app
            .send(
                req(Method::POST, "/v1/apps/game/releases")
                    .cookie(&maker)
                    .json(body(bad)),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body(vec![
                    format!("{base}/game-windows.zip.001"),
                    format!("{base}/game-windows.zip.002"),
                ])),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());

    // The fake host serves the same bytes for each part, so the joined file is two copies.
    let joined = [BUILD_BYTES, BUILD_BYTES].concat();
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["status"], "awaiting_review", "{mine}");
    assert_eq!(mine[0]["files"][0]["size_bytes"], joined.len());
    assert_eq!(
        mine[0]["files"][0]["sha256"],
        hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&joined))
    );

    admin::approve(&app.db, Some(&app.signing_key), lax(), id, "rev")
        .await
        .unwrap();
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    let signed: slop_manifest::Signed = serde_json::from_value(
        body_json(
            app.send(req(Method::GET, "/v1/apps/game/manifest").header("x-country", "US"))
                .await,
        )
        .await,
    )
    .unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    let file = &manifest.files[0];
    assert_eq!(file.size, joined.len() as u64);
    assert_eq!(file.parts.len(), 2);
    let part_sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(BUILD_BYTES));
    for (i, part) in file.parts.iter().enumerate() {
        assert_eq!(part.size, BUILD_BYTES.len() as u64);
        assert_eq!(part.sha256, part_sha);
        assert!(part.url.ends_with(&format!(".{:03}", i + 1)));
    }

    // A link for each part; one past the end isn't there.
    let link = |part: usize| {
        app.send(
            req(
                Method::GET,
                format!("/v1/apps/game/releases/{id}/download/windows?part={part}"),
            )
            .header("x-country", "US"),
        )
    };
    let res = link(2).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        body_json(res).await["url"]
            .as_str()
            .unwrap()
            .ends_with("game-windows.zip.002")
    );
    assert_eq!(link(3).await.status(), StatusCode::NOT_FOUND);
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
    assert_eq!(mine[0]["files"][0]["file_name"], "game-windows.zip");
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
    let file = manifest.file_for(slop_manifest::Platform::Windows).unwrap();
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
        json!({ "platform": "windows", "kind": "archive", "url": "https://evil.example/x.zip", "exec": "game" }),
        json!({ "platform": "windows", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.zip", app.fake_base) }),
        json!({ "platform": "windows", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.zip", app.fake_base), "exec": "../../bin/sh" }),
        json!({ "platform": "windows", "kind": "apk", "url": format!("{}octo/game/releases/download/v1/x.apk", app.fake_base) }),
        // Wrong file type for the platform, and a platform the store page doesn't list.
        json!({ "platform": "windows", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.tar.gz", app.fake_base), "exec": "g" }),
        json!({ "platform": "windows", "kind": "installer", "url": format!("{}octo/game/releases/download/v1/setup.exe", app.fake_base) }),
        json!({ "platform": "linux", "kind": "archive", "url": format!("{}octo/game/releases/download/v1/x.tar.gz", app.fake_base), "exec": "g" }),
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
        "repository_id": "1001",
        "repository_owner_id": "42",
    });
    for (k, v) in overrides.as_object().unwrap() {
        claims[k] = v.clone();
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key".into());
    let key =
        jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("../fixtures/oidc-test-key.pem"))
            .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

async fn github_app(app: &TestApp) -> (Uuid, String) {
    let (maker_id, cookie) = app.user("Maker", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'game', 'Game', 'A game', 'game', 'beta', $2, 'https://github.com/octo/game', 'MIT',
                 '{windows,linux,linux-deb}')",
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
            { "platform": "linux", "file_name": "game-build.12-linux.tar.gz", "exec": "game" },
            { "platform": "linux-deb", "file_name": "game_build.12_amd64.deb" },
            // The store page doesn't list .rpm, so this one is left out.
            { "platform": "linux-rpm", "file_name": "game-build.12.x86_64.rpm" },
            { "platform": "windows", "file_name": "game-build.12-windows.zip", "exec": "game.exe", "args": ["--fullscreen"] }
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
    // The store page lists Windows and Linux (.tar.gz and .deb): a release without them all is
    // refused, saying what's missing.
    let mut partial = ci_release();
    partial["files"] = json!([partial["files"][3].clone()]);
    let res = submit_ci(&app, &github_token(&app, json!({})), partial).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Linux and Linux .deb")
    );
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
        &"https://github.com/octo/game/releases/download/slop-build-12/game-build.12-linux.tar.gz"
    ));
    let deb = release["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["platform"] == "linux-deb")
        .unwrap();
    assert_eq!(deb["kind"], "installer");
    assert!(deb["exec"].is_null());
    let windows = release["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["platform"] == "windows")
        .unwrap();
    assert_eq!(
        windows["args"],
        json!(["--fullscreen"]),
        "from the maker's slop.toml"
    );
    assert_eq!(release["files"].as_array().unwrap().len(), 3, "no .rpm");
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
    std::fs::write(
        dir.join("guide.html"),
        "<!doctype html><title>How to publish</title>",
    )
    .unwrap();
    std::fs::write(dir.join("publishing.md"), "# Publishing → Slop").unwrap();

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
    // The guide comes with its text already in the page, and as UTF-8 Markdown, for tools that
    // don't run JavaScript.
    assert!(
        text(app.send(req(Method::GET, "/publish/guide")).await)
            .await
            .contains("How to publish")
    );
    let md = app.send(req(Method::GET, "/publishing.md")).await;
    assert_eq!(md.headers()["content-type"], "text/markdown; charset=utf-8");
    assert!(text(md).await.contains("Publishing → Slop"));
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
        environment: Some("production".into()),
        ..Default::default()
    };
    let router = router(AppState::new(base.db.clone(), config).unwrap()).unwrap();
    let app = TestApp { router, ..base };

    let ci = |overrides: Value| {
        let mut claims = json!({
            "repository": "octo/slop",
            "job_workflow_ref": "octo/slop/.github/workflows/pipeline.yml@refs/heads/main",
            "environment": "production",
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

    // Only the pipeline on main in Slop's repository, in the approved environment, may upload.
    for bad in [
        json!({ "repository": "mallory/slop", "job_workflow_ref": "mallory/slop/.github/workflows/pipeline.yml@refs/heads/main" }),
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/other.yml@refs/heads/main" }),
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/pipeline.yml@refs/heads/feature" }),
        // The old desktop workflow's channels.
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/desktop.yml@refs/heads/main" }),
        json!({ "job_workflow_ref": "octo/slop/.github/workflows/desktop.yml@refs/tags/production" }),
        // Not approved (SLOP_DESKTOP_ENVIRONMENT).
        json!({ "environment": null }),
        json!({ "environment": "staging" }),
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
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a reason is required"
    );
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
                .json(json!({ "rule": "misleading", "reason": "Test delisting" })),
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
            "moderation.delist_app",
            "staff_action.request",
            "release.revoke",
            "staff_action.request",
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
        ..Default::default()
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
    // A second session running alongside adds nothing: playtime follows the clock.
    sqlx::query("update pulse_sessions set started_at = now() - interval '2 minutes', last_beat_at = now() - interval '61 seconds' where token_hash = sha256($1::bytea)")
        .bind(second.as_bytes())
        .execute(&app.db)
        .await
        .unwrap();
    app.send(pulse("heartbeat", &second)).await;
    assert!(
        (61..=63).contains(&playtime().await),
        "{}",
        playtime().await
    );
    // A long gap is capped.
    sqlx::query("update pulse_sessions set last_beat_at = now() - interval '9 minutes' where token_hash = sha256($1::bytea)")
        .bind(token.as_bytes())
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update user_app_usage set credited_until = now() - interval '9 minutes' where user_id = $1")
        .bind(player_id)
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
    sqlx::query("update apps set platforms = '{windows}'")
        .execute(&app.db)
        .await
        .unwrap();

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
    assert!(assets[1]["platform"].is_null(), "an .exe isn't guessed");
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

    // Each platform has its file type, and only the store page's platforms are accepted.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .cookie(&maker)
                .json(json!({
                    "tag": "v1.1.0",
                    "files": [{
                        "platform": "windows", "kind": "archive", "exec": "game.exe",
                        "url": "https://github.com/octo/game/releases/download/v1.1.0/game-windows.tar.gz"
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("must be a .zip")
    );
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
            .contains("doesn't list Linux")
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

    // The next check sends the ETag, and GitHub's "not modified" means nothing to do, even
    // with the release gone from Slop.
    sqlx::query("delete from releases where version = '1.1.0'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set watch_checked_at = null, watch_last_tag = null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 0);
    // Checked a moment ago and recently updated: not due again yet.
    sqlx::query("update apps set watch_etag = null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 0);
    // Without the ETag, and due, it's submitted again.
    sqlx::query("update apps set watch_checked_at = null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::updates::watch_once(&app.state).await.unwrap(), 1);

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

/// The version in the manifest players are given.
async fn served_version(app: &TestApp, cookie: &str) -> String {
    let res = app
        .send(req(Method::GET, "/v1/apps/game/manifest").cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let signed: slop_manifest::Signed = serde_json::from_value(body_json(res).await).unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    manifest.version
}

async fn request_change(app: &TestApp, cookie: &str, body: Value) -> Response {
    app.send(
        req(Method::POST, "/v1/apps/game/changes")
            .cookie(cookie)
            .json(body),
    )
    .await
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_choose_the_release_and_moves_are_reviewed(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    assert_eq!(release_status(&app, &second).await.0, "approved");
    assert_eq!(served_version(&app, &maker).await, "1.1.0");

    let manage = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(manage["serving"]["version"], "1.1.0");
    assert_eq!(manage["held"], false);
    assert_eq!(manage["approved"].as_array().unwrap().len(), 2);
    assert_eq!(manage["repo_locked"], true);

    // Rolling back to a release that already passed its checks goes through automatically.
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "release", "release_id": first, "note": "1.1 crashes" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let change = body_json(res).await;
    assert_eq!(change["status"], "approved");
    assert_eq!(change["automatic"], true);
    assert_eq!(served_version(&app, &maker).await, "1.0.0");
    // A new approved release doesn't go out while the app is held.
    let third = submit_and_fetch(&app, &maker, "1.2.0").await;
    assert_eq!(release_status(&app, &third).await.0, "approved");
    assert_eq!(served_version(&app, &maker).await, "1.0.0");
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "release", "release_id": first }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "already held there");

    // Without automatic approval, a person decides.
    app.send(
        req(Method::PUT, "/v1/apps/game/update-settings")
            .cookie(&maker)
            .json(json!({ "auto_update": false })),
    )
    .await;
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "release", "release_id": null }),
    )
    .await;
    let change = body_json(res).await;
    assert_eq!(change["status"], "pending");
    assert!(change["status_reason"].as_str().unwrap().contains("person"));
    assert_eq!(served_version(&app, &maker).await, "1.0.0");

    let (admin_id, admin_cookie) = app.user("Ada Admin", "US").await;
    admin::set_admin(&app.db, &admin_id.to_string(), true, "test")
        .await
        .unwrap();
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(queue["changes"][0]["id"], change["id"]);
    assert_eq!(queue["changes"][0]["players_get"], "1.0.0");
    let res = app
        .send(
            req(
                Method::POST,
                format!(
                    "/v1/admin/changes/{}/approve",
                    change["id"].as_str().unwrap()
                ),
            )
            .cookie(&admin_cookie),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(served_version(&app, &maker).await, "1.2.0");

    // Moving the repository: not from the store page editor, and always by a person.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let elsewhere = "https://github.com/octo/game-two".to_string();
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .cookie(&maker)
                .json(json!({ "repo_url": elsewhere })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .cookie(&maker)
                .json(json!({ "repo_url": "https://github.com/Octo/game.git", "title": "Game!" })),
        )
        .await;
    let status = res.status();
    assert_eq!(
        status,
        StatusCode::OK,
        "the same repository is fine: {}",
        body_json(res).await
    );

    app.send(
        req(Method::PUT, "/v1/apps/game/update-settings")
            .cookie(&maker)
            .json(json!({ "auto_update": true })),
    )
    .await;
    let change = body_json(
        request_change(
            &app,
            &maker,
            json!({ "kind": "repo", "repo_url": elsewhere }),
        )
        .await,
    )
    .await;
    assert_eq!(change["status"], "pending");
    let id: Uuid = change["id"].as_str().unwrap().parse().unwrap();
    assert!(manage::reject_change(&app.db, id, "rev", "").await.is_err());
    manage::reject_change(&app.db, id, "rev", "which repo is real?")
        .await
        .unwrap();
    let change = body_json(
        request_change(
            &app,
            &maker,
            json!({ "kind": "repo", "repo_url": elsewhere }),
        )
        .await,
    )
    .await;
    manage::approve_change(
        &app.db,
        change["id"].as_str().unwrap().parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let repo: String = sqlx::query_scalar("select repo_url from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(repo, elsewhere);
    let settings = body_json(
        app.send(req(Method::GET, "/v1/apps/game/update-settings").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(settings["auto_approval_ready"], false);
    assert!(
        settings["auto_approval_note"]
            .as_str()
            .unwrap()
            .contains("moved")
    );

    // The history shows each decision; a pending one can be cancelled.
    let pending = body_json(
        request_change(
            &app,
            &maker,
            json!({ "kind": "repo", "repo_url": "https://github.com/octo/game-three" }),
        )
        .await,
    )
    .await;
    let res = app
        .send(
            req(
                Method::POST,
                format!(
                    "/v1/apps/game/changes/{}/cancel",
                    pending["id"].as_str().unwrap()
                ),
            )
            .cookie(&maker),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let manage = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    let statuses: Vec<&str> = manage["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        ["cancelled", "approved", "rejected", "approved", "approved"]
    );

    // Only the maker sees it.
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request_change(
            &app,
            &stranger,
            json!({ "kind": "release", "release_id": null })
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

fn signed_webhook(event: &str, body: Value, secret: &str) -> Req {
    use hmac::{KeyInit, Mac};
    let bytes = serde_json::to_vec(&body).unwrap();
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(&bytes);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    req(Method::POST, "/v1/github/webhook")
        .header("x-github-event", event.to_string())
        .header("x-hub-signature-256", signature)
        .json(body)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_github_app_gets_webhooks_and_its_own_tokens(db: PgPool) {
    let app = test_app_with(db, |config| {
        config.releases.github_app = Some(slop_api::config::GithubAppConfig {
            app_id: "123".into(),
            slug: "slop-test".into(),
            private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                "../fixtures/github-app-test-key.pem"
            ))
            .unwrap(),
            webhook_secret: "hook-secret".into(),
        });
    })
    .await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    // The maker's linked GitHub account owns octo/hooked: only an owner's lookup uses Slop's
    // App to read a private repository.
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', '42', $1)",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let license = |cookie: String| {
        let app = &app;
        async move {
            body_json(
                app.send(
                    req(
                        Method::GET,
                        "/v1/repos/license?url=https://github.com/octo/hooked",
                    )
                    .cookie(&cookie),
                )
                .await,
            )
            .await
        }
    };
    // A private repository Slop can't see yet.
    assert!(license(maker.clone()).await["spdx"].is_null());

    // A webhook with the wrong signature is refused.
    let install = json!({
        "action": "created",
        "installation": { "id": 77 },
        "repositories": [{ "full_name": "octo/Hooked" }]
    });
    let res = app
        .send(signed_webhook("installation", install.clone(), "wrong"))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // The maker installs the App: requests about that repository now use its token.
    let res = app
        .send(signed_webhook("installation", install, "hook-secret"))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(license(maker.clone()).await["spdx"], "MIT");

    // The store page shows it's installed, with the install link for other repositories.
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms, watch_checked_at)
         values ($1, 'hooked', 'Hooked', 'A game', 'game', 'beta', $2, 'https://github.com/octo/hooked', 'MIT', '{windows}', now())",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let settings = body_json(
        app.send(req(Method::GET, "/v1/apps/hooked/update-settings").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(settings["github_app"]["installed"], true);
    assert_eq!(
        settings["github_app"]["install_url"],
        "https://github.com/apps/slop-test/installations/new"
    );

    // A published release wakes the watcher for that repository's apps.
    let res = app
        .send(signed_webhook(
            "release",
            json!({
                "action": "published",
                "installation": { "id": 77 },
                "repository": { "full_name": "octo/hooked" },
                "release": { "tag_name": "v2.0.0" }
            }),
            "hook-secret",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let checked: Option<time::OffsetDateTime> =
        sqlx::query_scalar("select watch_checked_at from apps where slug = 'hooked'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(checked.is_none());

    // Removing the repository from the installation goes back to the shared token.
    let res = app
        .send(signed_webhook(
            "installation_repositories",
            json!({
                "action": "removed",
                "installation": { "id": 77 },
                "repositories_removed": [{ "full_name": "octo/hooked" }]
            }),
            "hook-secret",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(license(maker.clone()).await["spdx"].is_null());
    let settings = body_json(
        app.send(req(Method::GET, "/v1/apps/hooked/update-settings").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(settings["github_app"]["installed"], false);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn trending_puts_the_most_played_first(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    listed_with(&app, maker, "quiet", "released", "updated_at = now()").await;
    listed_with(
        &app,
        maker,
        "busy",
        "released",
        "updated_at = now() - interval '1 day'",
    )
    .await;
    listed_with(
        &app,
        maker,
        "old-hit",
        "released",
        "updated_at = now() - interval '2 days'",
    )
    .await;
    for (slug, players, ago) in [("busy", 40, "1 hour"), ("old-hit", 900, "30 days")] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into pulse_counts (app_id, bucket, players)
             select id, now() - interval '{ago}', $2 from apps where slug = $1"
        )))
        .bind(slug)
        .bind(players)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // This week's players first; a hit from last month counts for nothing now.
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?sort=trending")).await,
        ["busy", "quiet", "old-hit"]
    );
}

// ---------------------------------------------------------------------------------------------
// Store-page pictures and videos, and similar apps

/// Files the fake GitHub serves from /raw/<owner>/<repo>/<commit>/<path>.
/// The TXT record the fake DNS answers with (official team checks).
static DNS_TXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

static RAW_FILES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
> = std::sync::LazyLock::new(Default::default);

fn put_raw(commit: &str, path: &str, bytes: Vec<u8>) {
    RAW_FILES
        .lock()
        .unwrap()
        .insert(format!("octo/game/{commit}/.slop/{path}"), bytes);
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_pixel(w, h, image::Rgb([30, 160, 90]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A draft on octo/game whose repository has a thumbnail, two screenshots and a video at `commit`.
async fn app_with_pictures(app: &TestApp, maker: Uuid, slug: &str, commit: &str) -> Uuid {
    listed_with(
        app,
        maker,
        slug,
        "coming_soon",
        "status = 'draft', repo_url = 'https://github.com/octo/game'",
    )
    .await;
    put_raw(commit, "thumb.png", png(920, 430));
    put_raw(commit, "shots/a.png", png(640, 360));
    put_raw(commit, "shots/b.png", png(1280, 720));
    put_raw(
        commit,
        "media.json",
        json!({
            "thumbnail": "thumb.png",
            "gallery": [
                { "image": "shots/a.png", "caption": "Level one" },
                { "youtube": "https://youtu.be/dQw4w9WgXcQ" },
                { "image": "./shots/b.png" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn make_admin(app: &TestApp, name: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    admin::set_admin(&app.db, &id.to_string(), true, "test")
        .await
        .unwrap();
    (id, cookie)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pictures_come_from_the_repository_and_wait_for_review(db: PgPool) {
    let app = test_app(db).await;
    let (maker, cookie) = app.user("Maker", "US").await;
    let (_, other) = app.user("Other", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Admin").await;
    // "Check" reads the default branch's newest commit, which the fake GitHub says is b…b.
    let head = "b".repeat(40);
    app_with_pictures(&app, maker, "pics", &head).await;

    let res = app
        .send(req(Method::POST, "/v1/apps/pics/media/check").cookie(&other))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .send(req(Method::POST, "/v1/apps/pics/media/check").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["result"], "pending");
    assert_eq!(body["live"], Value::Null);
    let pending = &body["pending"];
    let set_id = pending["id"].as_str().unwrap().to_string();
    assert_eq!(
        pending["thumbnail"]["url"],
        format!("{}raw/octo/game/{head}/.slop/thumb.png", app.fake_base)
    );
    assert_eq!(pending["thumbnail"]["width"], 920);
    let kinds: Vec<_> = pending["gallery"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["image", "video", "image"]);
    assert_eq!(pending["gallery"][2]["path"], "shots/b.png");
    assert_eq!(pending["gallery"][1]["poster"], Value::Null);

    // Nothing shows until it's reviewed, and the app can't be listed meanwhile.
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/pics").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(page["thumbnail_url"], Value::Null);
    assert_eq!(page["gallery"], json!([]));
    let res = app
        .send(req(Method::POST, "/v1/admin/apps/pics/list").cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(queue["media"][0]["id"], set_id.as_str());
    assert_eq!(queue["ready_to_list"][0]["pictures_pending"], true);

    // Checking again finds nothing new.
    let body = body_json(
        app.send(req(Method::POST, "/v1/apps/pics/media/check").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(body["result"], "unchanged");
    assert_eq!(body["pending"]["id"], set_id.as_str());

    // Turned down: the maker sees why, and the same pictures aren't queued again.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/media/{set_id}/reject"))
                .cookie(&admin_cookie)
                .json(json!({ "reason": "The second screenshot is from another game." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let body = body_json(
        app.send(req(Method::POST, "/v1/apps/pics/media/check").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(body["result"], "problem");
    assert!(body["problem"].as_str().unwrap().contains("another game"));
    assert_eq!(
        body["rejected"]["status_reason"],
        "The second screenshot is from another game."
    );

    // A release at a new commit with different pictures: a new set to review.
    let next = "c".repeat(40);
    let app_id: Uuid = sqlx::query_scalar("select id from apps where slug = 'pics'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    app_with_pictures_files(&next);
    let outcome = slop_api::media::refresh(
        &app.state,
        app_id,
        "https://github.com/octo/game",
        Some(&next),
        "release",
    )
    .await
    .unwrap();
    let slop_api::media::Outcome::Pending(new_set) = outcome else {
        panic!("expected new pictures, got {outcome:?}");
    };
    let res = app
        .send(req(Method::POST, format!("/v1/admin/media/{new_set}/approve")).cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/pics").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(
        page["thumbnail_url"],
        format!("{}raw/octo/game/{next}/.slop/thumb.png", app.fake_base)
    );
    let order = |page: &Value| -> Vec<String> {
        page["gallery"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i["youtube_id"].as_str().map_or_else(
                    || {
                        i["url"]
                            .as_str()
                            .unwrap()
                            .rsplit('/')
                            .next()
                            .unwrap()
                            .to_string()
                    },
                    String::from,
                )
            })
            .collect()
    };
    assert_eq!(order(&page), ["a.png", "b.png"]);

    // The maker puts the gallery in order on the website; that isn't reviewed.
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/pics/media/order")
                .cookie(&cookie)
                .json(json!({ "keys": ["image:shots/b.png", "image:shots/a.png"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["live"]["gallery"][0]["path"],
        "shots/b.png"
    );
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/pics").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(order(&page), ["b.png", "a.png"]);

    let res = app
        .send(req(Method::POST, "/v1/admin/apps/pics/list").cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let listed = list_slugs(&app, req(Method::GET, "/v1/apps")).await;
    assert_eq!(listed, ["pics"]);
}

/// At `commit`: the thumbnail and two screenshots, no video.
fn app_with_pictures_files(commit: &str) {
    put_raw(commit, "thumb.png", png(920, 430));
    put_raw(commit, "shots/a.png", png(640, 360));
    put_raw(commit, "shots/b.png", png(640, 360));
    put_raw(
        commit,
        "media.json",
        json!({ "thumbnail": "thumb.png", "gallery": [{ "image": "shots/a.png" }, { "image": "shots/b.png" }] })
            .to_string()
            .into_bytes(),
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bad_picture_lists_are_explained(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    let app_id = app_with_pictures(&app, maker, "oops", &"d".repeat(40)).await;
    let check = |commit: String, manifest: Value| {
        put_raw(&commit, "media.json", manifest.to_string().into_bytes());
        let state = app.state.clone();
        async move {
            slop_api::media::refresh(
                &state,
                app_id,
                "https://github.com/octo/game",
                Some(&commit),
                "maker",
            )
            .await
            .unwrap_err()
            .0
        }
    };
    let e = "e".repeat(40);
    put_raw(&e, "small.png", png(300, 100));
    put_raw(&e, "big.png", png(640, 360));
    for (manifest, expect) in [
        (
            json!({ "gallery": [{ "image": "../../etc/passwd.png" }] }),
            "isn't a usable path",
        ),
        (
            json!({ "gallery": [{ "image": "missing.png" }] }),
            "isn't in the repository",
        ),
        (json!({ "thumbnail": "small.png" }), "at least 460×215"),
        (
            json!({ "gallery": [{ "youtube": "https://vimeo.com/1" }] }),
            "isn't a YouTube video link",
        ),
        (
            json!({ "gallery": [{ "image": "big.png" }, { "image": "big.png" }] }),
            "twice",
        ),
        (json!({ "galery": [] }), "isn't valid"),
        (
            json!({ "gallery": (0..21).map(|_| json!({ "image": "big.png" })).collect::<Vec<_>>() }),
            "the most is 20",
        ),
    ] {
        let problem = check(e.clone(), manifest.clone()).await;
        assert!(problem.contains(expect), "{manifest}: {problem}");
    }
    let stored: Option<String> = sqlx::query_scalar("select media_problem from apps where id = $1")
        .bind(app_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(stored.unwrap().contains("the most is 20"));
    // No .slop/media.json at all: nothing changes, and that's not a problem.
    let outcome = slop_api::media::refresh(
        &app.state,
        app_id,
        "https://github.com/octo/game",
        Some(&"f".repeat(40)),
        "maker",
    )
    .await
    .unwrap();
    assert_eq!(outcome, slop_api::media::Outcome::NoManifest);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ai_reviewers_use_admin_tokens(db: PgPool) {
    let app = test_app(db).await;
    let (maker, maker_cookie) = app.user("Maker", "US").await;
    let (admin_id, admin_cookie) = make_admin(&app, "Admin").await;
    app_with_pictures(&app, maker, "robot", &"b".repeat(40)).await;
    app.send(req(Method::POST, "/v1/apps/robot/media/check").cookie(&maker_cookie))
        .await;

    // Only admins, signed in themselves, make tokens.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&maker_cookie)
                .json(json!({ "name": "Claude", "scope": "review" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let mut tokens = vec![];
    for scope in ["read", "review"] {
        let res = app
            .send(
                req(Method::POST, "/v1/admin/tokens")
                    .cookie(&admin_cookie)
                    .json(json!({ "name": format!("Claude {scope}"), "scope": scope, "days": 30 })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        let body = body_json(res).await;
        assert!(body["token"].as_str().unwrap().starts_with("slopadm_"));
        tokens.push(format!("Bearer {}", body["token"].as_str().unwrap()));
    }
    let (read, review) = (tokens[0].clone(), tokens[1].clone());
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .header("authorization", review.clone())
                .json(json!({ "name": "child", "scope": "review" })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::NOT_FOUND,
        "tokens can't make tokens"
    );
    let res = app
        .send(req(Method::GET, "/v1/admin/review").header("authorization", "Bearer slopadm_nope"))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // The feed: everything waiting, with the app and the requests that decide it.
    let res = app
        .send(req(Method::GET, "/v1/admin/review").header("authorization", read.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let feed = body_json(res).await;
    assert_eq!(feed["can_decide"], false);
    let item = feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "media")
        .unwrap()
        .clone();
    assert_eq!(item["app"]["slug"], "robot");
    assert_eq!(item["app"]["description_md"], "");
    assert_eq!(
        item["detail"]["pictures"]["gallery"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let set_id = item["id"].as_str().unwrap().to_string();
    assert_eq!(
        item["approve"]["path"],
        format!("/v1/admin/media/{set_id}/approve")
    );
    let guide = app
        .send(req(Method::GET, "/v1/admin/review/guide").header("authorization", read.clone()))
        .await;
    assert_eq!(guide.status(), StatusCode::OK);
    let text = String::from_utf8(
        guide
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        text.contains("relative to\nhttp://localhost:8080")
            || text.contains("relative to http://localhost:8080"),
        "{text}"
    );
    assert!(!text.contains("{origin}"));
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/review/guide"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // A read token can leave notes but not decide.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/media/{set_id}/approve"))
                .header("authorization", read.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/review/media/{set_id}/notes"),
            )
            .header("authorization", read.clone())
            .json(json!({ "verdict": "unsure", "note": "Can't watch the video." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    let note = &queue["media"][0]["notes"][0];
    assert_eq!(note["verdict"], "unsure", "{queue}");
    assert!(
        note["author"]
            .as_str()
            .unwrap()
            .starts_with("AI reviewer \"Claude read\" for Admin")
    );

    // A review token decides, and the audit log says who.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/media/{set_id}/approve"))
                .header("authorization", review.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'media.approve'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        actor.starts_with("AI reviewer \"Claude review\""),
        "{actor}"
    );
    let feed = body_json(
        app.send(
            req(Method::GET, "/v1/admin/review?kind=listing")
                .header("authorization", review.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(
        feed["items"][0]["approve"]["path"],
        "/v1/admin/apps/robot/list"
    );

    // Revoked, or its admin no longer an admin: the token stops working.
    let list = body_json(
        app.send(req(Method::GET, "/v1/admin/tokens").cookie(&admin_cookie))
            .await,
    )
    .await;
    let read_id = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["scope"] == "read")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(req(Method::POST, format!("/v1/admin/tokens/{read_id}/revoke")).cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/review").header("authorization", read))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    admin::set_admin(&app.db, &admin_id.to_string(), false, "test")
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/review").header("authorization", review))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn similar_apps_mix_tags_popularity_and_discovery(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    let tagged = [
        ("dungeon", vec!["roguelike", "pixel-art", "retro"]),
        ("twin", vec!["roguelike", "pixel-art", "retro"]),
        ("cousin", vec!["roguelike", "cozy"]),
        ("stranger", vec!["racing"]),
        ("hit", vec!["racing"]),
        ("gem", vec![]),
        ("zombie", vec!["roguelike", "pixel-art", "retro"]),
    ];
    for (slug, tags) in &tagged {
        listed_with(&app, maker, slug, "released", "").await;
        for tag in tags {
            sqlx::query(
                "insert into app_tags (app_id, tag_id)
                 select a.id, t.id from apps a, tags t where a.slug = $1 and t.slug = $2",
            )
            .bind(slug)
            .bind(tag)
            .execute(&app.db)
            .await
            .unwrap();
        }
    }
    // A dead project is never recommended, however well it matches.
    sqlx::query(
        "update apps set stage = 'beta', last_commit_at = now() - interval '200 days' where slug = 'zombie'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into pulse_counts (app_id, bucket, players) select id, now() - interval '1 hour', 500 from apps where slug = 'hit'",
    )
    .execute(&app.db)
    .await
    .unwrap();

    let res = app.send(req(Method::GET, "/v1/apps/dungeon/similar")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    let slugs = |k: &str| -> Vec<String> {
        body[k]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(slugs("like_this"), ["twin", "cousin"]);
    assert_eq!(slugs("popular"), ["hit"]);
    let mut discover = slugs("discover");
    discover.sort();
    assert_eq!(discover, ["gem", "stranger"]);

    let res = app.send(req(Method::GET, "/v1/apps/nope/similar")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------------------------
// Ratings, For you, and the simulation

async fn tag_app(app: &TestApp, slug: &str, tags: &[&str]) {
    for tag in tags {
        sqlx::query(
            "insert into app_tags (app_id, tag_id) select a.id, t.id from apps a, tags t where a.slug = $1 and t.slug = $2",
        )
        .bind(slug)
        .bind(tag)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_players_vote_and_ratings_add_up(db: PgPool) {
    let app = test_app(db).await;
    let (maker, maker_cookie) = app.user("Maker", "US").await;
    listed_with(&app, maker, "voted", "released", "").await;
    listed_with(&app, maker, "early", "beta", "").await;
    let (player, cookie) = app.user("Player", "US").await;
    let vote = |cookie: String, slug: &'static str, v: Value| {
        let r = req(Method::POST, format!("/v1/apps/{slug}/vote"))
            .cookie(&cookie)
            .json(json!({ "vote": v }));
        async { app.send(r).await }
    };

    // Not launched yet: can't vote; the page says why.
    let res = vote(cookie.clone(), "voted", json!("not_slop")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/voted/ratings").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(ratings["mine"]["can_vote"], false);
    assert!(
        ratings["mine"]["reason"]
            .as_str()
            .unwrap()
            .contains("Launch it")
    );

    for slug in ["voted", "early"] {
        let res = app
            .send(req(Method::POST, format!("/v1/apps/{slug}/launch")).cookie(&cookie))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    // Voting opens at release; makers can't vote on their own apps.
    assert_eq!(
        vote(cookie.clone(), "early", json!("slop")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        vote(maker_cookie.clone(), "voted", json!("slop"))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        vote(cookie.clone(), "voted", json!("meh")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let res = vote(cookie.clone(), "voted", json!("not_slop")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let r = body_json(res).await;
    assert_eq!(r["not_slop"], 1);
    assert_eq!(r["percent"], Value::Null, "below the minimum");
    assert_eq!(r["mine"]["vote"], "not_slop");
    // Changed, then withdrawn: three events, no current vote.
    vote(cookie.clone(), "voted", json!("slop")).await;
    let r = body_json(vote(cookie.clone(), "voted", Value::Null).await).await;
    assert_eq!(
        (r["not_slop"].as_i64(), r["slop"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(r["mine"]["vote"], Value::Null);
    let events: Vec<(i16, Option<i16>)> = sqlx::query_as(
        "select vote, prev_vote from vote_events where user_id = $1 order by created_at, id",
    )
    .bind(player)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(events, [(1, None), (-1, Some(1)), (0, Some(-1))]);
    assert_eq!(r["daily"][0]["changed"], 1);
    assert_eq!(r["daily"][0]["withdrawn"], 1);

    // Five voters: a percentage and a label, weighted by playtime.
    for (i, (v, hours)) in [
        ("not_slop", 10),
        ("not_slop", 10),
        ("not_slop", 0),
        ("slop", 0),
        ("slop", 0),
    ]
    .into_iter()
    .enumerate()
    {
        let (id, c) = app.user(&format!("Voter {i}"), "US").await;
        app.send(req(Method::POST, "/v1/apps/voted/launch").cookie(&c))
            .await;
        sqlx::query("update user_app_usage set playtime_seconds = $3 where user_id = $1 and app_id = (select id from apps where slug = $2)")
            .bind(id)
            .bind("voted")
            .bind(hours * 3600_i64)
            .execute(&app.db)
            .await
            .unwrap();
        assert_eq!(vote(c, "voted", json!(v)).await.status(), StatusCode::OK);
    }
    let r = body_json(app.send(req(Method::GET, "/v1/apps/voted/ratings")).await).await;
    assert_eq!(r["percent"], 60, "3 of 5");
    // Two votes weigh 1 (10 hours); three weigh ≈0.29 (one launch day): (2 + 0.29) / (2 + 0.87).
    assert_eq!(r["percent_weighted"], 80);
    assert_eq!(r["label"], "Very Not Slop");
    let months = r["monthly"].as_array().unwrap();
    assert_eq!(months.last().unwrap()["not_slop"], 3);
    assert_eq!(months.last().unwrap()["percent_weighted"], 80);
    let listed = body_json(app.send(req(Method::GET, "/v1/apps?sort=title")).await).await;
    let tile = listed["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["slug"] == "voted")
        .unwrap()
        .clone();
    assert_eq!(
        (
            tile["vote_count"].as_i64(),
            tile["not_slop_percent"].as_i64()
        ),
        (Some(5), Some(80))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn for_you_follows_activity_and_can_be_switched_off(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    for (slug, tags) in [
        ("rogue-a", &["roguelike", "pixel-art"][..]),
        ("rogue-b", &["roguelike", "dark"][..]),
        ("rogue-c", &["roguelike", "retro"][..]),
        ("rogue-d", &["roguelike", "pixel-art", "retro"][..]),
        ("cozy-farm", &["cozy", "simulation"][..]),
        ("racer", &["racing", "chaotic"][..]),
        ("dev-kit", &["developer-tools", "automation"][..]),
        ("notes", &["note-taking", "productivity"][..]),
        ("adult", &["roguelike", "dark"][..]),
    ] {
        listed_with(&app, maker, slug, "released", "").await;
        tag_app(&app, slug, tags).await;
    }
    sqlx::query("update apps set min_age = 18 where slug = 'adult'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set kind = 'app' where slug in ('dev-kit', 'notes')")
        .execute(&app.db)
        .await
        .unwrap();
    let (_, cookie) = app.user("Fan", "US").await;
    for slug in ["rogue-a", "rogue-b"] {
        app.send(req(Method::PUT, format!("/v1/me/library/{slug}")).cookie(&cookie))
            .await;
    }

    let res = app
        .send(req(Method::GET, "/v1/me/for-you").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let feed = body_json(res).await;
    assert_eq!(feed["personalized"], true);
    assert_eq!(feed["based_on"][0], "Roguelike");
    let items = feed["items"].as_array().unwrap();
    let slugs: Vec<&str> = items.iter().map(|i| i["slug"].as_str().unwrap()).collect();
    assert!(
        !slugs.contains(&"rogue-a") && !slugs.contains(&"rogue-b"),
        "not what they have: {slugs:?}"
    );
    // The three roguelikes they don't have come first (US accounts see 18+ apps too).
    let mut top: Vec<&str> = slugs[..3].to_vec();
    top.sort();
    assert_eq!(
        top,
        ["adult", "rogue-c", "rogue-d"],
        "best matches first: {slugs:?}"
    );
    assert!(
        items[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Because you like Roguelike")
    );
    assert_eq!(
        items[3]["pick"], "discover",
        "every fourth card is a discovery"
    );
    assert!(
        items[3]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Something different")
    );

    // Switched off: the same feed as everyone.
    app.send(
        req(Method::PATCH, "/v1/me")
            .cookie(&cookie)
            .json(json!({ "personalize": false })),
    )
    .await;
    let feed = body_json(
        app.send(req(Method::GET, "/v1/me/for-you").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(feed["personalized"], false);
    assert!(
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["pick"] == "popular")
    );
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&cookie)).await).await;
    assert_eq!(me["personalize"], false);

    // UK accounts start in the under-18 view: not personal by default, and no 18+ apps.
    let (_, uk) = app.user("UK fan", "GB").await;
    app.send(req(Method::PUT, "/v1/me/library/rogue-a").cookie(&uk))
        .await;
    let feed = body_json(
        app.send(req(Method::GET, "/v1/me/for-you").cookie(&uk))
            .await,
    )
    .await;
    assert_eq!(feed["personalized"], false);
    assert!(
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["slug"] != "adult")
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/for-you")).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_simulation_scores_the_real_code_against_its_truth(db: PgPool) {
    let app = test_app(db).await;
    let made = slop_api::sim::generate(
        &app.db,
        slop_api::sim::Settings {
            apps: 160,
            users: 400,
            days: 75,
            seed: 11,
        },
    )
    .await
    .unwrap();
    assert!(made.vote_events > 300, "{made:?}");
    let r = slop_api::sim::report(&app.db, &app.config, 300, 5)
        .await
        .unwrap();
    // Never recommended.
    assert_eq!(
        (r.shown_dead, r.shown_in_library_or_played, r.shown_own_apps),
        (0, 0, 0)
    );
    assert_eq!((r.under_18_personal, r.under_18_shown_18_plus), (0, 0));
    // Better than popular, which is better than random; a quarter is discovery.
    assert!(r.for_you.hit_rate > r.popular_baseline.hit_rate, "{r:#?}");
    assert!(
        r.popular_baseline.hit_rate > r.random_baseline.hit_rate,
        "{r:#?}"
    );
    assert!((0.2..=0.3).contains(&r.discovery_share), "{r:#?}");
    assert!(r.discovery_fit < r.match_fit);
    // 👎 and 👍 move the feed.
    assert!(
        r.dislike_roguelike_share_after < r.dislike_roguelike_share_before,
        "{r:#?}"
    );
    assert!(r.like_tool_share_after > r.like_tool_share_before, "{r:#?}");
    assert!(r.gamer_with_tools_tool_share > 0.05, "{r:#?}");
    // The numbers add up.
    assert!(r.ratings_checked > 20);
    assert_eq!(r.ratings_matching, r.ratings_checked);
    assert_eq!(r.monthly_matching_now, r.ratings_checked);
    assert!(r.rating_truth_correlation > 0.4, "{r:#?}");
    assert_eq!(r.players_matching, r.players_checked);

    // People: creatives, jobs in every state, payments through the real code.
    let p = &r.people;
    assert!(made.people.jobs > 40, "{made:?}");
    // Profiles moderation hid (sim_moderation.rs) aren't live.
    let hidden: i64 =
        sqlx::query_scalar("select count(*) from creatives where hidden_reason is not null")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (p.live_profiles + hidden) as usize,
        slop_api::sim_people::CREATIVES.len()
    );
    assert_eq!(
        p.portfolios_waiting_for_review as usize,
        slop_api::sim_people::WAITING.len()
    );
    for status in [
        "requested",
        "offered",
        "in_progress",
        "completed",
        "declined",
    ] {
        assert!(p.jobs_by_status.contains_key(status), "{p:#?}");
    }
    assert_eq!(p.fees_not_5_percent, 0);
    assert_eq!(p.credit_rule_violations, 0, "{p:#?}");
    assert!(p.credited_jobs > 5, "{p:#?}");
    assert!(p.completed_automatically > 0, "{p:#?}");
    assert_eq!((p.reviews_not_completed, p.reviews_not_by_client), (0, 0));
    assert_eq!(p.overall_rating_untouched, p.badge_apps);
    // Mediation and conversations, through the real code.
    assert_eq!(p.conversations_failing_verification, 0, "{p:#?}");
    assert!(p.conversations_verified > 0, "{p:#?}");
    assert_eq!(p.final_files_before_payment, 0, "{p:#?}");
    assert_eq!(p.hidden_reviews_counted, 0, "{p:#?}");
    assert!(p.messages > 0 && p.inquiries > 0, "{p:#?}");
    // Too few reviews per creative at this size to score ratings; the sandbox report does.
    assert!(p.reviews > 30, "{p:#?}");
    // Reports, decisions and appeals, through the real code.
    let m = &r.moderation;
    assert_eq!(m.actioned_without_decision, 0, "{m:#?}");
    for status in ["open", "actioned", "dismissed"] {
        assert!(
            m.reports_by_status
                .iter()
                .any(|(s, n)| s == status && *n > 0),
            "{m:#?}"
        );
    }
    assert!(!m.decisions_by_action.is_empty(), "{m:#?}");
    // Donations: 5% of each, every one recorded as a payment.
    let d = &r.donations;
    assert!(d.donations > 5, "{d:#?}");
    assert_eq!(
        (d.fees_not_5_percent, d.paid_without_payment),
        (0, 0),
        "{d:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// People: creatives, jobs, payments, credits (DESIGN §15)

fn profile(handle: &str, trader: bool) -> Value {
    json!({
        "handle": handle,
        "name": "Vera Okafor",
        "kind": "individual",
        "headline": "Pixel art and tiny animations",
        "skills": ["pixel-art", "music"],
        "ai_use": "none",
        "trader": trader,
        "taking_work": true,
        "turnaround": "1–2 weeks",
        "accent": "grape",
        "sections": ["services", "portfolio"],
        "links": [{ "label": "", "url": "https://www.artstation.com/vera" }],
        "services": [
            { "name": "Sprite sheet", "description": "One character, 4 directions", "price_minor": 12000, "currency": "usd", "days": 7, "skill": "pixel-art" }
        ],
    })
}

/// A creative whose profile is live (portfolio approved) and who can be paid (simulated).
async fn live_creative(app: &TestApp, handle: &str, country: &str, trader: bool) -> (Uuid, String) {
    let (id, cookie) = app.user(handle, country).await;
    let mut p = profile(handle, trader);
    p["confirm_adult"] = json!(true);
    let res = app
        .send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let set = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         values ($1, $2, 'https://github.com/x/y', repeat('c', 40), '[]', 'f', 'approved', 'sim')",
    )
    .bind(set)
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update creatives set portfolio_set_id = $2 where user_id = $1")
        .bind(id)
        .bind(set)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/me/creative/stripe").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    (id, cookie)
}

fn sprite_offer(payment: &str) -> Value {
    json!({
        "title": "Knight sprite sheet",
        "scope_md": "4 directions × 8 frames.",
        "price_minor": 12001,
        "currency": "usd",
        "payment": payment,
        "due_date": (OffsetDateTime::now_utc().date() + time::Duration::days(14)).to_string(),
        "revisions": 1,
        "rights": "transfer",
        "open_source": false,
        "credit": true,
        "ai_use": "none"
    })
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_go_live_with_a_reviewed_portfolio(db: PgPool) {
    let app = test_app(db).await;
    let (composer_id, cookie) = app.user("Composer", "US").await;
    // Their GitHub account (42) owns octo/game, the portfolio repository.
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', '42', $1)",
    )
    .bind(composer_id)
    .execute(&app.db)
    .await
    .unwrap();
    let (_, other) = app.user("Visitor", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Admin").await;

    // Adults only, confirmed once.
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative")
                .cookie(&cookie)
                .json(profile("loopsmith", true)),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let mut p = profile("loopsmith", true);
    p["confirm_adult"] = json!(true);
    let res = app
        .send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let me = body_json(res).await;
    assert_eq!(me["live"], false);
    assert_eq!(
        me["sections"],
        json!(["services", "portfolio", "reviews", "worked_on", "about"])
    );
    assert_eq!(me["links"][0]["label"], "www.artstation.com");
    assert_eq!(me["services"][0]["id"].as_str().unwrap().len(), 8);
    assert_eq!(me["payments"], "off");
    // Handles are unique; bad links and unknown skills are refused.
    let (_, rival) = app.user("Rival", "US").await;
    let mut p = profile("loopsmith", false);
    p["confirm_adult"] = json!(true);
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative")
                .cookie(&rival)
                .json(p.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    p["handle"] = json!("rival");
    p["links"] = json!([{ "label": "x", "url": "javascript:alert(1)" }]);
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative")
                .cookie(&rival)
                .json(p.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Not live yet: only its owner sees it.
    assert_eq!(
        app.send(req(Method::GET, "/v1/people/loopsmith").cookie(&other))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let own = body_json(
        app.send(req(Method::GET, "/v1/people/loopsmith").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(own["status"], "waiting");

    // The portfolio comes from the repository: pictures, a video and a track.
    let head = "b".repeat(40);
    put_raw(&head, "portfolio/avatar.png", png(256, 256));
    put_raw(&head, "portfolio/knight.png", png(64, 64));
    put_raw(
        &head,
        "portfolio/theme.ogg",
        b"OggS\0\x02rest of a track".to_vec(),
    );
    put_raw(
        &head,
        "portfolio/fake.mp3",
        b"<html>not audio</html>".to_vec(),
    );
    put_raw(
        &head,
        "portfolio.json",
        json!({
            "avatar": "portfolio/avatar.png",
            "works": [
                { "image": "portfolio/knight.png", "title": "Knight", "skill": "pixel-art" },
                { "audio": "portfolio/fake.mp3", "title": "Theme" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let res = app
        .send(
            req(Method::POST, "/v1/me/creative/portfolio/check")
                .cookie(&cookie)
                .json(json!({ "repo_url": "https://github.com/octo/game" })),
        )
        .await;
    let body = body_json(res).await;
    assert_eq!(body["result"], "problem");
    assert!(
        body["problem"].as_str().unwrap().contains("isn't an MP3"),
        "{body}"
    );

    put_raw(
        &head,
        "portfolio.json",
        json!({
            "avatar": "portfolio/avatar.png",
            "works": [
                { "image": "portfolio/knight.png", "title": "Knight", "skill": "pixel-art" },
                { "audio": "portfolio/theme.ogg", "poster": "portfolio/knight.png", "title": "Main theme", "skill": "music" },
                { "youtube": "https://youtu.be/dQw4w9WgXcQ", "title": "Reel" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let body = body_json(
        app.send(
            req(Method::POST, "/v1/me/creative/portfolio/check")
                .cookie(&cookie)
                .json(json!({})),
        )
        .await,
    )
    .await;
    assert_eq!(body["result"], "pending", "{body}");
    let set = body["pending"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["pending"]["works"][1]["audio"]["format"], "ogg");

    // Reviewed like store pictures, by people or AI reviewers.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(queue["portfolios"][0]["set"]["id"], set.as_str());
    let feed = body_json(
        app.send(req(Method::GET, "/v1/admin/review?kind=portfolio").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(feed["items"][0]["creative"]["handle"], "loopsmith");
    assert_eq!(
        feed["items"][0]["approve"]["path"],
        format!("/v1/admin/portfolio/{set}/approve")
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/portfolio/{set}/approve")).cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Live: in the directory and on its own page, in the creative's order.
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative/portfolio/order")
                .cookie(&cookie)
                .json(json!({ "keys": ["audio:portfolio/theme.ogg"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let dir = body_json(app.send(req(Method::GET, "/v1/people?skill=music")).await).await;
    assert_eq!(dir["people"][0]["handle"], "loopsmith");
    assert_eq!(
        dir["people"][0]["can_hire"], false,
        "payments are off on this server"
    );
    assert_eq!(dir["people"][0]["from_price"]["minor"], 12000);
    let none = body_json(app.send(req(Method::GET, "/v1/people?skill=voice")).await).await;
    assert_eq!(none["people"], json!([]));
    let page = body_json(
        app.send(req(Method::GET, "/v1/people/loopsmith").cookie(&other))
            .await,
    )
    .await;
    assert_eq!(page["status"], "live");
    assert_eq!(page["is_me"], false);
    assert_eq!(page["accent"], "grape");
    assert_eq!(page["works"][0]["kind"], "audio");
    assert_eq!(
        page["works"][0]["audio_url"],
        format!(
            "{}raw/octo/game/{head}/.slop/portfolio/theme.ogg",
            app.fake_base
        )
    );
    assert_eq!(page["works"][2]["youtube_id"], "dQw4w9WgXcQ");
    assert_eq!(page["links"][0]["domain"], "artstation.com");
    assert_eq!(page["rating"]["percent"], Value::Null);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hiring_runs_from_request_to_credit(db: PgPool) {
    // A long walk-through: more writes than one client may make in a minute.
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 1000;
    })
    .await;
    let (creative, creative_cookie) = live_creative(&app, "vera", "US", true).await;
    // A consumer in Germany hiring a trader: the withdrawal right applies.
    let (client, client_cookie) = app.user("Tavern Dev", "DE").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Mediator").await;
    listed_with(&app, client, "tiny-tavern", "released", "").await;
    listed_with(&app, creative, "veras-own", "released", "").await;
    let post =
        |path: String, cookie: &str, body: Value| req(Method::POST, path).cookie(cookie).json(body);

    // The request: the client's own project, adults only, and the exact-wording warning.
    let hire = |slug: &str, ack: bool, inquiry: bool| {
        json!({ "app_slug": slug, "skill": "pixel-art", "request_md": "A knight, 4 directions, 32×32.",
                "confirm_adult": true, "exact_wording_ack": ack, "inquiry": inquiry })
    };
    let res = app
        .send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("tiny-tavern", false, true),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "the exact-wording box is required"
    );
    let res = app
        .send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("veras-own", true, true),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "not their project"
    );
    let res = app
        .send(post(
            "/v1/people/vera/hire".into(),
            &creative_cookie,
            hire("veras-own", true, true),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "can't hire yourself");
    let job = body_json(
        app.send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("tiny-tavern", true, true),
        ))
        .await,
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    let url = |step: &str| format!("/v1/jobs/{id}/{step}");
    assert_eq!(job["inquiry"], true);
    assert!(
        job["conversation"]["requested_at"].is_string(),
        "an inquiry asks for a conversation"
    );
    assert!(
        !job["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("message"))
    );
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/jobs/{id}")).cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Conversations: the creative opens them; both acknowledge they're recorded.
    assert_eq!(
        app.send(post(url("conversation/open"), &client_cookie, json!({})))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(post(
            url("messages"),
            &client_cookie,
            json!({ "body": "hello?" })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/jobs").cookie(&creative_cookie))
            .await,
    )
    .await;
    assert_eq!(mine["as_creative"][0]["conversation_requested"], true);
    let job = body_json(
        app.send(post(url("conversation/open"), &creative_cookie, json!({})))
            .await,
    )
    .await;
    assert_eq!(job["conversation"]["open"], true);
    assert!(
        job["conversation"]["notice"]
            .as_str()
            .unwrap()
            .contains("recorded")
    );
    let res = app
        .send(post(
            url("messages"),
            &creative_cookie,
            json!({ "body": "Which engine?" }),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "must acknowledge the recording"
    );
    app.send(post(
        url("messages"),
        &creative_cookie,
        json!({ "body": "Which engine?", "recording_ack": true }),
    ))
    .await;
    app.send(post(
        url("messages"),
        &client_cookie,
        json!({ "body": "Godot, transparent PNGs.", "recording_ack": true }),
    ))
    .await;
    assert_eq!(
        app.send(post(
            url("messages"),
            &stranger,
            json!({ "body": "hi", "recording_ack": true })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    // Either side can report the other's message; a stranger can't see it to report it.
    let job = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id}")).cookie(&client_cookie))
            .await,
    )
    .await;
    let first = job["conversation"]["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let report = |cookie: &str| {
        req(Method::POST, "/v1/reports").cookie(cookie).json(json!({
            "kind": "message", "subject": first, "reason": "harassment",
            "details": "Rude.", "good_faith": true
        }))
    };
    assert_eq!(
        app.send(report(&stranger)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(report(&creative_cookie)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(report(&client_cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let quoted: String =
        sqlx::query_scalar("select quoted from reports where subject_kind = 'message'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        quoted, "Which engine?",
        "the report keeps the text as it was"
    );
    // Each side hears about the other's steps and messages; messages merge while unread.
    let inbox = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&creative_cookie))
            .await,
    )
    .await;
    let titles: Vec<&str> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert!(
        titles
            .iter()
            .any(|t| t.contains("would like to talk about a job")),
        "{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.starts_with("New messages from")),
        "{titles:?}"
    );
    let job = body_json(
        app.send(post(url("conversation/close"), &creative_cookie, json!({})))
            .await,
    )
    .await;
    assert_eq!(job["conversation"]["messages"].as_array().unwrap().len(), 2);
    assert!(job["conversation"]["text_deleted_after"].is_string());
    // Each side downloads its own copy; the fingerprints chain.
    let res = app
        .send(req(Method::GET, url("conversation")).cookie(&client_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let copy = body_json(res).await;
    assert_eq!(copy["messages"][1]["body"], "Godot, transparent PNGs.");

    // The offer, with a sign-off; accepting asks for the exact-wording box again.
    let mut o = sprite_offer("split");
    o["sign_off"] = json!(true);
    app.send(post(url("offer"), &creative_cookie, o)).await;
    let job = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id}")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert!(
        job["summary"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("exactly as written"))
    );
    assert!(
        job["agreement"]["text"]
            .as_str()
            .unwrap()
            .contains("nothing more and nothing less")
    );
    assert_eq!(job["withdrawal_days"], 14);
    let res = app
        .send(post(
            url("accept"),
            &client_cookie,
            json!({ "offer_count": 1 }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let job = body_json(
        app.send(post(
            url("accept"),
            &client_cookie,
            json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["status"], "awaiting_payment");
    let paid = body_json(app.send(post(url("pay"), &client_cookie, json!({}))).await).await;
    assert_eq!(paid["job"]["status"], "in_progress");

    // Sign-off first; a change while waiting is new work with its own price and date.
    assert_eq!(
        app.send(post(
            url("deliver"),
            &creative_cookie,
            json!({ "note": "x" })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let job = body_json(
        app.send(post(
            url("sign-off/send"),
            &creative_cookie,
            json!({ "links": ["https://drive.example/sketch.png"] }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["status"], "awaiting_sign_off");
    let job = body_json(
        app.send(post(
            url("change/request"),
            &client_cookie,
            json!({ "note": "Could the knight also run?" }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["changes"][0]["status"], "requested");
    assert_eq!(
        app.send(post(
            url("change/request"),
            &client_cookie,
            json!({ "note": "And jump?" })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let due = (OffsetDateTime::now_utc().date() + time::Duration::days(30)).to_string();
    let job = body_json(app.send(post(url("change/offer"), &creative_cookie, json!({
        "title": "Add a running animation", "scope_md": "8 frames, 4 directions.", "extra_price_minor": 4000, "new_due_date": due
    }))).await).await;
    assert_eq!(job["changes"][0]["status"], "offered");
    assert_eq!(
        app.send(post(
            url("change/accept"),
            &client_cookie,
            json!({ "number": 1 })
        ))
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let job = body_json(
        app.send(post(
            url("change/accept"),
            &client_cookie,
            json!({ "number": 1, "exact_terms_ack": true }),
        ))
        .await,
    )
    .await;
    assert_eq!(
        job["status"], "awaiting_payment",
        "the change's price is due now"
    );
    assert_eq!(job["due_date"], due);
    assert!(
        job["changes"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("# Change to Agreement 1")
    );
    assert_eq!(job["changes"][0]["sha256"].as_str().unwrap().len(), 64);
    let paid = body_json(app.send(post(url("pay"), &client_cookie, json!({}))).await).await;
    assert_eq!(paid["job"]["status"], "in_progress");
    app.send(post(
        url("sign-off/send"),
        &creative_cookie,
        json!({ "links": ["https://drive.example/sketch2.png"] }),
    ))
    .await;
    let job = body_json(
        app.send(post(url("sign-off"), &client_cookie, json!({})))
            .await,
    )
    .await;
    assert!(job["signed_off_at"].is_string());

    // The delivery is a preview while money is owed; paying it accepts, then the final files.
    let job = body_json(
        app.send(post(
            url("deliver"),
            &creative_cookie,
            json!({ "links": ["https://drive.example/preview.zip"] }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["status"], "delivered");
    assert!(
        job["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "delivered_preview")
    );
    assert_eq!(
        app.send(post(url("accept-delivery"), &client_cookie, json!({})))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let paid = body_json(app.send(post(url("pay"), &client_cookie, json!({}))).await).await;
    assert_eq!(paid["job"]["status"], "final_files");
    assert_eq!(paid["job"]["credited"], false, "not until the final files");
    let job = body_json(
        app.send(post(
            url("final-files"),
            &creative_cookie,
            json!({ "links": ["https://drive.example/final.zip"] }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["credited"], true);
    assert_eq!(job["paid_minor"], 12001 + 4000);
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tiny-tavern")).await).await;
    assert_eq!(page["made_with_people"], true);
    assert_eq!(page["credits"][0]["handle"], "vera");

    // The record has it all, for both sides.
    let res = app
        .send(req(Method::GET, url("record")).cookie(&creative_cookie))
        .await;
    let record =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(record.contains("## Service Agreement"));
    assert!(record.contains("# Change to Agreement 1"));
    assert!(record.contains("Godot, transparent PNGs."));

    // A problem: the copy of the conversation is checked against the fingerprints.
    let job = body_json(
        app.send(post(
            url("problem"),
            &client_cookie,
            json!({ "statement": "One frame is missing from the run cycle." }),
        ))
        .await,
    )
    .await;
    assert_eq!(job["problem"]["status"], "open");
    let checked = body_json(
        app.send(post(
            url("evidence"),
            &client_cookie,
            json!({ "copy": copy.clone() }),
        ))
        .await,
    )
    .await;
    assert_eq!(
        (
            checked["verified_messages"].as_i64(),
            checked["unverified_messages"].as_i64()
        ),
        (Some(2), Some(0))
    );
    let mut forged = copy.clone();
    forged["messages"][1]["body"] = json!("Unity, and also a free boss sprite.");
    let checked = body_json(
        app.send(post(
            url("evidence"),
            &creative_cookie,
            json!({ "copy": forged }),
        ))
        .await,
    )
    .await;
    assert_eq!(checked["unverified_messages"], 1);
    // Mediators read the record (audited) and suggest; only the two sides can agree.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    let problem = queue["problems"][0]["id"].as_str().unwrap().to_string();
    let res = app
        .send(req(Method::GET, format!("/v1/admin/jobs/{id}/record")).cookie(&admin_cookie))
        .await;
    let record =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(record.contains("2 messages match Slop's fingerprints, 0 don't"));
    let res = app
        .send(post(
            format!("/v1/admin/problems/{problem}/suggest"),
            &admin_cookie,
            json!({ "suggestion": "A $20 refund for the missing frame." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let job = body_json(
        app.send(post(
            url("problem/propose"),
            &creative_cookie,
            json!({ "kind": "refund", "amount_minor": 2000, "note": "Sorry, $20 back." }),
        ))
        .await,
    )
    .await;
    let proposal = job["problem"]["proposals"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        app.send(post(
            url(&format!("problem/proposals/{proposal}/accept")),
            &creative_cookie,
            json!({})
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let job = body_json(
        app.send(post(
            url(&format!("problem/proposals/{proposal}/accept")),
            &client_cookie,
            json!({}),
        ))
        .await,
    )
    .await;
    assert_eq!(job["problem"]["status"], "resolved");
    assert_eq!(
        job["refunded_minor"], 2000,
        "simulated here; Stripe refunds come from the creative"
    );

    // A 👎 review, disputed: the mediator decides on the rules, with a reason.
    app.send(
        req(Method::PUT, url("review"))
            .cookie(&client_cookie)
            .json(json!({ "vote": "slop", "body": "Redo it free or this stays." })),
    )
    .await;
    let res = app
        .send(post(
            format!("/v1/people/reviews/{id}/dispute"),
            &stranger,
            json!({ "reason": "extortion", "statement": "x" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .send(post(
            format!("/v1/people/reviews/{id}/dispute"),
            &creative_cookie,
            json!({ "reason": "extortion", "statement": "The review is a threat for free work." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let job = body_json(
        app.send(
            req(Method::PUT, url("review/dispute-answer"))
                .cookie(&client_cookie)
                .json(json!({ "statement": "It's my honest opinion." })),
        )
        .await,
    )
    .await;
    assert_eq!(job["review"]["dispute"]["status"], "pending");
    let feed = body_json(
        app.send(req(Method::GET, "/v1/admin/review?kind=review_dispute").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(feed["items"][0]["id"], id.as_str());
    let res = app
        .send(post(
            format!("/v1/admin/review-disputes/{id}/decide"),
            &admin_cookie,
            json!({ "outcome": "remove", "reason": "" }),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a reason is required"
    );
    let res = app.send(post(format!("/v1/admin/review-disputes/{id}/decide"), &admin_cookie, json!({ "outcome": "remove", "reason": "The review demands free work in exchange for removing it." }))).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let page = body_json(app.send(req(Method::GET, "/v1/people/vera")).await).await;
    assert_eq!(page["reviews"], json!([]));
    assert_eq!(page["rating"]["slop"], 0);

    // Thirty days after the conversation closed, Slop forgets the text; copies still verify.
    let n = slop_api::people::jobs::forget_conversations(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(31),
    )
    .await
    .unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        app.send(req(Method::GET, url("conversation")).cookie(&client_cookie))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let kept: slop_api::people::jobs::ConversationCopy = serde_json::from_value(copy).unwrap();
    let job_id: Uuid = id.parse().unwrap();
    assert_eq!(
        slop_api::people::jobs::verify_copy(&app.db, job_id, &kept)
            .await
            .unwrap(),
        (2, 0)
    );

    // Second job: nothing owed on delivery, left 7 days: completes on its own.
    let job2 = body_json(app.send(post("/v1/people/vera/hire".into(), &client_cookie, json!({ "app_slug": "tiny-tavern", "skill": "music", "request_md": "A tavern tune.", "client_is_business": true, "exact_wording_ack": true }))).await).await;
    let id2 = job2["id"].as_str().unwrap().to_string();
    app.send(post(
        format!("/v1/jobs/{id2}/offer"),
        &creative_cookie,
        sprite_offer("upfront"),
    ))
    .await;
    let job2 = body_json(
        app.send(post(
            format!("/v1/jobs/{id2}/accept"),
            &client_cookie,
            json!({ "offer_count": 1, "exact_terms_ack": true }),
        ))
        .await,
    )
    .await;
    assert_eq!(
        job2["withdrawal"],
        Value::Null,
        "hiring as a business: no consumer withdrawal"
    );
    app.send(post(
        format!("/v1/jobs/{id2}/pay"),
        &client_cookie,
        json!({}),
    ))
    .await;
    app.send(post(
        format!("/v1/jobs/{id2}/deliver"),
        &creative_cookie,
        json!({ "note": "Attached." }),
    ))
    .await;
    let n = slop_api::people::jobs::auto_complete(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(8),
    )
    .await
    .unwrap();
    assert_eq!(n, 1);
    let job2 = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id2}")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert_eq!(
        (
            job2["status"].as_str(),
            job2["completed_automatically"].as_bool()
        ),
        (Some("completed"), Some(true))
    );

    // Third: everything on delivery, never paid. Auto-completed, still owed: no new hires.
    let job3 = body_json(
        app.send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("tiny-tavern", true, false),
        ))
        .await,
    )
    .await;
    let id3 = job3["id"].as_str().unwrap().to_string();
    app.send(post(
        format!("/v1/jobs/{id3}/offer"),
        &creative_cookie,
        sprite_offer("on_delivery"),
    ))
    .await;
    let job3 = body_json(
        app.send(post(
            format!("/v1/jobs/{id3}/accept"),
            &client_cookie,
            json!({ "offer_count": 1, "exact_terms_ack": true }),
        ))
        .await,
    )
    .await;
    assert_eq!(
        job3["status"], "in_progress",
        "nothing to pay until delivery"
    );
    app.send(post(
        format!("/v1/jobs/{id3}/deliver"),
        &creative_cookie,
        json!({ "links": ["https://drive.example/p.png"] }),
    ))
    .await;
    slop_api::people::jobs::auto_complete(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(8),
    )
    .await
    .unwrap();
    let res = app
        .send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("tiny-tavern", true, false),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("you still owe")
    );
    let job3 = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id3}")).cookie(&creative_cookie))
            .await,
    )
    .await;
    assert_eq!(job3["client_record"]["owing"], 1);
    // Paying it unlocks the final files and hiring again.
    let paid = body_json(
        app.send(post(
            format!("/v1/jobs/{id3}/pay"),
            &client_cookie,
            json!({}),
        ))
        .await,
    )
    .await;
    assert_eq!(paid["job"]["status"], "final_files");

    // Fourth: withdrawn by the consumer within 14 days.
    let job4 = body_json(
        app.send(post(
            "/v1/people/vera/hire".into(),
            &client_cookie,
            hire("tiny-tavern", true, false),
        ))
        .await,
    )
    .await;
    let id4 = job4["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{job4}"))
        .to_string();
    app.send(post(
        format!("/v1/jobs/{id4}/offer"),
        &creative_cookie,
        sprite_offer("on_delivery"),
    ))
    .await;
    app.send(post(
        format!("/v1/jobs/{id4}/accept"),
        &client_cookie,
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    ))
    .await;
    let job4 = body_json(
        app.send(post(
            format!("/v1/jobs/{id4}/withdraw"),
            &client_cookie,
            json!({}),
        ))
        .await,
    )
    .await;
    assert_eq!(job4["status"], "withdrawn", "{job4}");
}

/// A fake Stripe: accounts, account links and Checkout sessions on connected accounts.
async fn fake_stripe() -> String {
    use std::sync::{Arc, Mutex};
    type Sessions = Arc<Mutex<std::collections::HashMap<String, Value>>>;
    let sessions: Sessions = Arc::default();
    let create = {
        let sessions = sessions.clone();
        move |headers: axum::http::HeaderMap, body: String| {
            let sessions = sessions.clone();
            async move {
                assert_eq!(headers["authorization"], "Bearer sk_test_slop");
                assert_eq!(headers["stripe-account"], "acct_vera");
                let form: std::collections::HashMap<String, String> =
                    url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                let id = format!("cs_{}", sessions.lock().unwrap().len() + 1);
                let metadata: serde_json::Map<String, Value> = form
                    .iter()
                    .filter_map(|(k, v)| {
                        k.strip_prefix("metadata[")
                            .map(|k| (k.trim_end_matches(']').to_string(), json!(v)))
                    })
                    .collect();
                let session = json!({
                    "id": id, "status": "open", "payment_status": "unpaid",
                    "url": format!("https://checkout.stripe.example/{id}"),
                    "amount_total": form.iter()
                        .filter(|(k, _)| k.starts_with("line_items[") && k.ends_with("[price_data][unit_amount]"))
                        .map(|(_, v)| v.parse::<i64>().unwrap())
                        .sum::<i64>(),
                    "currency": form["line_items[0][price_data][currency]"],
                    "application_fee": form.get("payment_intent_data[application_fee_amount]").map(|f| f.parse::<i64>().unwrap()),
                    "metadata": metadata,
                    // What Slop sent, for tests to check.
                    "form": form,
                });
                sessions.lock().unwrap().insert(id, session.clone());
                axum::Json(session)
            }
        }
    };
    let get_session = {
        let sessions = sessions.clone();
        move |axum::extract::Path(id): axum::extract::Path<String>| {
            let sessions = sessions.clone();
            async move {
                let s = sessions.lock().unwrap().get(&id).cloned().unwrap();
                let mut s = s.clone();
                s["payment_intent"] = json!({ "latest_charge": format!("ch_{id}"), "application_fee_amount": s["application_fee"] });
                axum::Json(s)
            }
        }
    };
    let pay = {
        let sessions = sessions.clone();
        move |axum::extract::Path(id): axum::extract::Path<String>| {
            let sessions = sessions.clone();
            async move {
                let mut all = sessions.lock().unwrap();
                let s = all.get_mut(&id).unwrap();
                s["status"] = json!("complete");
                s["payment_status"] = json!("paid");
                StatusCode::OK
            }
        }
    };
    let app = Router::new()
        .route("/v1/accounts", post(|| async { axum::Json(json!({ "id": "acct_vera" })) }))
        .route(
            "/v1/accounts/{id}",
            get(|| async { axum::Json(json!({ "id": "acct_vera", "charges_enabled": true, "details_submitted": true })) }),
        )
        .route(
            "/v1/account_links",
            post(|body: String| async move {
                assert!(body.contains("account=acct_vera"), "{body}");
                axum::Json(json!({ "url": "https://connect.stripe.example/setup/acct_vera" }))
            }),
        )
        .route("/v1/checkout/sessions", post(create))
        .route("/v1/checkout/sessions/{id}", get(get_session))
        .route("/test/pay/{id}", post(pay))
        .route(
            "/v1/charges/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
                axum::Json(json!({ "id": id, "amount_refunded": 2000 }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

fn stripe_signed(body: &Value, secret: &str) -> (String, String) {
    use hmac::{Hmac, KeyInit, Mac};
    let body = body.to_string();
    let t = OffsetDateTime::now_utc().unix_timestamp();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    (
        body,
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes())),
    )
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_payments_are_direct_charges_confirmed_by_signed_webhooks(db: PgPool) {
    let stripe_base = fake_stripe().await;
    let app = test_app_with(db, |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: "whsec_platform, whsec_slop".into(),
            api_base: stripe_base.parse().unwrap(),
        })
    })
    .await;
    let (creative, cookie) = app.user("vera", "US").await;
    let mut p = profile("vera", false);
    p["confirm_adult"] = json!(true);
    app.send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         values ($1, $2, 'https://github.com/x/y', repeat('c', 40), '[]', 'f', 'approved', 'sim')",
    )
    .bind(Uuid::nil()).bind(creative).execute(&app.db).await.unwrap();
    sqlx::query("update creatives set portfolio_set_id = $1 where user_id = $2")
        .bind(Uuid::nil())
        .bind(creative)
        .execute(&app.db)
        .await
        .unwrap();

    // "Get paid" sends the creative to Stripe's own sign-up.
    let on = body_json(
        app.send(req(Method::POST, "/v1/me/creative/stripe").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(on["url"], "https://connect.stripe.example/setup/acct_vera");
    assert_eq!(on["status"], "pending");
    let back = body_json(
        app.send(req(Method::POST, "/v1/me/creative/stripe/refresh").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(back["status"], "active");

    let (client, client_cookie) = app.user("Client", "US").await;
    listed_with(&app, client, "tavern", "beta", "").await;
    let job = body_json(app.send(req(Method::POST, "/v1/people/vera/hire").cookie(&client_cookie).json(json!({ "app_slug": "tavern", "skill": "pixel-art", "request_md": "Knight", "confirm_adult": true, "exact_wording_ack": true }))).await).await;
    let id = job["id"].as_str().unwrap().to_string();
    app.send(
        req(Method::POST, format!("/v1/jobs/{id}/offer"))
            .cookie(&cookie)
            .json(sprite_offer("upfront")),
    )
    .await;
    app.send(
        req(Method::POST, format!("/v1/jobs/{id}/accept"))
            .cookie(&client_cookie)
            .json(json!({ "offer_count": 1, "exact_terms_ack": true })),
    )
    .await;

    // Pay: a Checkout session on the creative's account, with Slop's 5% fee; pressing again
    // reuses it.
    let paid = body_json(
        app.send(req(Method::POST, format!("/v1/jobs/{id}/pay")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert_eq!(paid["url"], "https://checkout.stripe.example/cs_1");
    assert_eq!(paid["job"]["status"], "awaiting_payment");
    let again = body_json(
        app.send(req(Method::POST, format!("/v1/jobs/{id}/pay")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert_eq!(again["url"], "https://checkout.stripe.example/cs_1");

    // The client pays on Stripe; the webhook must be signed.
    reqwest::Client::new()
        .post(format!("{stripe_base}test/pay/cs_1"))
        .send()
        .await
        .unwrap();
    let event = json!({ "id": "evt_1", "type": "checkout.session.completed", "account": "acct_vera", "data": { "object": { "id": "cs_1" } } });
    let (body, sig) = stripe_signed(&event, "whsec_wrong");
    let hook = |body: String, sig: String| {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/stripe/webhook")
            .header(header::HOST, "localhost:8080")
            .header("stripe-signature", sig)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let res = app.router.clone().oneshot(hook(body, sig)).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // Each endpoint has its own secret; either one signs.
    let (body, sig) = stripe_signed(&event, "whsec_slop");
    let res = app
        .router
        .clone()
        .oneshot(hook(body.clone(), sig.clone()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app.router.clone().oneshot(hook(body, sig)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "the same event again is fine");
    let job = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id}")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert_eq!(job["status"], "in_progress");
    let rows: Vec<(String, i32, i32, String, bool)> = sqlx::query_as(
        "select stripe_object_id, amount_minor, slop_fee_minor, stripe_account_id, simulated from payments",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [(
            "ch_cs_1".to_string(),
            12001,
            600,
            "acct_vera".to_string(),
            false
        )]
    );

    // The creative refunds part of it from their Stripe dashboard; Slop records it.
    let event = json!({ "id": "evt_2", "type": "charge.refunded", "account": "acct_vera", "data": { "object": { "id": "ch_cs_1" } } });
    let (body, sig) = stripe_signed(&event, "whsec_slop");
    let res = app.router.clone().oneshot(hook(body, sig)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let job = body_json(
        app.send(req(Method::GET, format!("/v1/jobs/{id}")).cookie(&client_cookie))
            .await,
    )
    .await;
    assert_eq!(job["refunded_minor"], 2000);
    assert!(
        job["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "refunded")
    );
}

// ---------------------------------------------------------------------------------------------
// Reports, moderation decisions, appeals, closing accounts and terms (DESIGN §4, §16)

fn report_app(slug: &str, reason: &str) -> Value {
    json!({ "kind": "app", "subject": slug, "reason": reason, "details": "It mines crypto.", "good_faith": true })
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reports_lead_to_decisions_with_reasons_and_appeals(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, reporter) = app.user("Rita Reporter", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "miner", "released", "").await;
    approved_release(&app, "miner", maker_id).await;

    // Reasons depend on what's reported.
    let reasons = body_json(
        app.send(req(Method::GET, "/v1/reports/reasons?kind=review"))
            .await,
    )
    .await;
    let codes: Vec<&str> = reasons
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"fake_review") && !codes.contains(&"malware"));

    // A report is acknowledged with a reference and a deadline: serious ones within a day.
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&reporter)
                .json(report_app("miner", "malware")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let ack = body_json(res).await;
    let reference = ack["reference"].as_str().unwrap().to_string();
    assert!(reference.starts_with("R-"));
    let due = OffsetDateTime::parse(ack["due_at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(due - OffsetDateTime::now_utc() <= time::Duration::hours(24));
    // Once per reporter while it's open; never about your own; a good-faith statement each time.
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&reporter)
                .json(report_app("miner", "scam")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&maker)
                .json(report_app("miner", "malware")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let mut no_statement = report_app("miner", "malware");
    no_statement["good_faith"] = json!(false);
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&other)
                .json(no_statement),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "good_faith");
    // Signed out: a name and email, except for child abuse reports.
    let res = app
        .send(req(Method::POST, "/v1/reports").json(report_app("miner", "broken")))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "name");
    let mut signed_out = report_app("miner", "broken");
    signed_out["name"] = json!("Sam");
    signed_out["email"] = json!("sam@example.com");
    assert_eq!(
        app.send(req(Method::POST, "/v1/reports").json(signed_out))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/reports").json(report_app("miner", "child_abuse")))
            .await
            .status(),
        StatusCode::OK
    );
    let preserved: i64 = sqlx::query_scalar(
        "select count(*) from reports where preserve_until > now() + interval '360 days'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(preserved, 1, "child abuse reports are preserved for a year");
    // A copyright report needs what a DMCA notice needs.
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&other)
                .json(report_app("miner", "copyright")),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "copyright");

    // The queue: urgent first, with who it's about.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    let reports = queue["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[0]["reason"], "child_abuse");
    assert_eq!(reports[0]["priority"], 0);
    assert_eq!(reports[0]["reporter"], "No name <no email>, signed out");
    assert_eq!(reports[1]["reason"], "malware");
    assert_eq!(reports[1]["account"]["name"], "Mia Maker");
    assert_eq!(reports[1]["same_subject"], 2);
    let report_id = reports[1]["id"].as_str().unwrap().to_string();
    // Only admins can act.
    let act = |cookie: &str, body: Value| {
        req(Method::POST, format!("/v1/admin/reports/{report_id}/act"))
            .cookie(cookie)
            .json(body)
    };
    let decision = json!({
        "action": "delist_app", "rule": "malware",
        "facts": "Version 1.2 starts a crypto miner in the background.",
        "outcome": "We took the app out of the store."
    });
    assert_eq!(
        app.send(act(&reporter, decision.clone())).await.status(),
        StatusCode::NOT_FOUND
    );
    let mut bad = decision.clone();
    bad["action"] = json!("hide_review");
    assert_eq!(
        app.send(act(&admin, bad)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(act(&admin, decision.clone())).await;
    assert_eq!(res.status(), StatusCode::OK);
    // Every open report about the app closes with it; the app leaves the store.
    let open: i64 = sqlx::query_scalar("select count(*) from reports where status = 'open'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/miner").cookie(&other))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // The reporter sees the outcome.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/reports").cookie(&reporter))
            .await,
    )
    .await;
    assert_eq!(mine[0]["reference"], reference.as_str());
    assert_eq!(mine[0]["status"], "actioned");
    assert_eq!(mine[0]["outcome"], "We took the app out of the store.");

    // The maker is told, with a statement of reasons.
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&maker)).await).await;
    assert_eq!(me["new_decisions"], 1);
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    let d = &decisions[0];
    assert_eq!(d["action_label"], "Taken out of the store");
    assert_eq!(d["rule_title"], "Malware and data theft");
    assert_eq!(d["automated"], false);
    assert_eq!(d["can_appeal"], true);
    assert_eq!(d["new"], true);
    assert!(
        d.get("decided_by").is_none(),
        "the moderator isn't named to the user"
    );
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&maker)).await).await;
    assert_eq!(me["new_decisions"], 0);
    // Others can't see or appeal it.
    let id = d["id"].as_str().unwrap().to_string();
    let appeal = |cookie: &str| {
        req(Method::POST, format!("/v1/me/decisions/{id}/appeal"))
            .cookie(cookie)
            .json(json!({ "appeal": "That's a pathfinding worker thread, not a miner." }))
    };
    assert_eq!(
        app.send(appeal(&reporter)).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(appeal(&maker)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["appeal_status"], "pending");
    assert_eq!(
        app.send(appeal(&maker)).await.status(),
        StatusCode::CONFLICT,
        "once"
    );

    // A person reverses it, and the app is back.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(
        queue["appeals"][0]["appeal"],
        "That's a pathfinding worker thread, not a miner."
    );
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/decisions/{id}/appeal"))
                .cookie(&admin)
                .json(json!({ "reverse": true, "reason": "Checked the source: it's a worker thread." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/miner").cookie(&other))
            .await
            .status(),
        StatusCode::OK
    );
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(decisions[0]["appeal_status"], "reversed");
    assert!(decisions[0]["reversed_at"].is_string());
    assert_eq!(decisions[0]["new"], true, "the answer is news too");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn suspended_accounts_can_read_and_appeal_but_nothing_else(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, reporter) = app.user("Rita Reporter", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "scammy", "released", "").await;
    app.send(
        req(Method::POST, "/v1/reports")
            .cookie(&reporter)
            .json(report_app("scammy", "scam")),
    )
    .await;
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    let report_id = queue["reports"][0]["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{report_id}/act"))
                .cookie(&admin)
                .json(json!({
                    "action": "suspend_account", "rule": "scam", "law": "FTC Act §5",
                    "facts": "Asked players for card details.", "outcome": "We suspended the maker."
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&maker)).await).await;
    assert_eq!(me["suspended"], true);
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("another")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "suspended");
    let res = app
        .send(req(Method::GET, "/v1/me/export").cookie(&maker))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "a suspended account can still download its data"
    );
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(decisions[0]["subject_kind"], "account");
    assert_eq!(decisions[0]["law"], "FTC Act §5");
    let id = decisions[0]["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{id}/appeal"))
                .cookie(&maker)
                .json(json!({ "appeal": "Someone else had my password." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    app.send(
        req(Method::POST, format!("/v1/admin/decisions/{id}/appeal"))
            .cookie(&admin)
            .json(json!({ "reverse": true, "reason": "Account was taken over; restored." })),
    )
    .await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("another")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_an_account_bans_the_sign_in_and_keeps_ratings_honest(db: PgPool) {
    let app = test_app(db).await;
    // Sign up through GitHub, so the sign-in can be tried again afterwards.
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
    let signup_cookie = set_cookie(&res, "slop_signup").unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/signup").cookie(&signup_cookie).json(json!({
                "display_name": "Octo", "country_code": "US", "age_confirmed": true, "accept_terms": true
            })),
        )
        .await;
    let cookie = set_cookie(&res, "slop_session").unwrap();
    let user_id: Uuid = sqlx::query_scalar("select id from users")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "liked", "released", "").await;
    listed_with(&app, user_id, "octos-game", "released", "").await;
    let liked: Uuid = sqlx::query_scalar("select id from apps where slug = 'liked'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into votes (user_id, app_id, vote, weight) values ($1, $2, 1, 2)")
        .bind(user_id)
        .bind(liked)
        .execute(&app.db)
        .await
        .unwrap();

    // The data export has everything linked to the account.
    let res = app
        .send(req(Method::GET, "/v1/me/export").cookie(&cookie))
        .await;
    assert!(
        res.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    let export = body_json(res).await;
    assert_eq!(export["account"]["email"], "octo@example.com");
    assert_eq!(export["sign_ins"][0]["provider"], "github");
    assert_eq!(export["votes"][0]["app"], "liked");
    assert_eq!(export["apps"][0]["slug"], "octos-game");
    assert!(export["app_releases"].is_array());
    assert_eq!(export["agreements_accepted"][0]["document"], "terms");
    assert!(export["sessions"][0].get("token_hash").is_none());

    // Typing the confirmation is required; an open People job would block it.
    let close = |confirm: &str| {
        req(Method::POST, "/v1/me/close")
            .cookie(&cookie)
            .json(json!({ "confirm": confirm }))
    };
    assert_eq!(
        app.send(close("yes")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(close("Close my account")).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Signed out everywhere, and the sign-in is refused from now on.
    assert_eq!(
        app.send(req(Method::GET, "/v1/me").cookie(&cookie))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
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
    assert_eq!(location(&res), "/signin?error=closed");
    assert!(set_cookie(&res, "slop_session").is_none());

    // Personal data gone; the vote still counts, linked to nobody; the app out of the store.
    let (email, closed): (Option<String>, bool) =
        sqlx::query_as("select email, closed_at is not null from users where id = $1")
            .bind(user_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((email, closed), (None, true));
    let votes: Vec<(Uuid, f32)> =
        sqlx::query_as("select user_id, weight from votes where app_id = $1")
            .bind(liked)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(votes.len(), 1);
    assert_ne!(votes[0].0, user_id);
    let status: String =
        sqlx::query_scalar("select status::text from apps where slug = 'octos-game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "delisted");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agreements_wait_for_the_version_in_effect(db: PgPool) {
    let app = test_app(db).await;
    let (id, maker) = app.user("Mia Maker", "US").await;
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'publisher'")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();

    // A first app needs the Publisher Agreement.
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("first")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(err["error"]["code"], "terms_required");
    assert_eq!(err["error"]["field"], "publisher");
    let doc = body_json(app.send(req(Method::GET, "/v1/legal/publisher")).await).await;
    assert_eq!(doc["title"], "Publisher Agreement");
    assert!(
        doc["body_md"]
            .as_str()
            .unwrap()
            .contains("Draft for legal review")
    );
    let version = doc["version"].as_str().unwrap().to_string();
    let accept = |kind: &str, version: &str| {
        req(Method::POST, "/v1/me/legal/accept")
            .cookie(&maker)
            .json(json!({ "kind": kind, "version": version }))
    };
    assert_eq!(
        app.send(accept("privacy", &version)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "policies are read, not accepted"
    );
    assert_eq!(
        app.send(accept("publisher", "1999-01-01")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(accept("publisher", &version)).await.status(),
        StatusCode::NO_CONTENT
    );
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("first")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);

    // A change is announced 15 days ahead: nothing waits for it until then, but it's shown.
    sqlx::query(
        "insert into legal_documents (kind, version, title, summary, body_md, sha256, effective_at)
         values ('publisher', '2099-01-01', 'Publisher Agreement', 'Clearer AI rules.', 'New words.', 'x', now() + interval '15 days')",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let notices = body_json(
        app.send(req(Method::GET, "/v1/me/legal").cookie(&maker))
            .await,
    )
    .await;
    let n = notices
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "publisher")
        .unwrap();
    assert_eq!(n["version"], "2099-01-01");
    assert_eq!(n["in_effect"], false);
    assert_eq!(n["action"], "accept");
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("second")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    // Once it's in effect, publishing waits for it.
    sqlx::query("update legal_documents set effective_at = now() where version = '2099-01-01'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("third")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        app.send(accept("publisher", "2099-01-01")).await.status(),
        StatusCode::NO_CONTENT
    );
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&maker)
                .json(new_app("third")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);

    // A new version of a file takes effect 15 days after it's published; the first at once.
    sqlx::query("update legal_documents set version = '2000-01-01' where kind = 'privacy'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::legal::sync(&app.db).await.unwrap();
    let wait: f64 = sqlx::query_scalar(
        "select extract(epoch from effective_at - published_at)::float8 from legal_documents
         where kind = 'privacy' and version <> '2000-01-01'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(wait, 15.0 * 86400.0);
    let notices = body_json(
        app.send(req(Method::GET, "/v1/me/legal").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        notices
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "privacy" && n["action"] == "read")
    );
}

// ---------------------------------------------------------------------------------------------
// Interested and notifications (DESIGN §5, §6)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn interested_players_hear_when_an_app_moves_on(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "Slop <n@slop.test>".into(),
        })
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (quiet_id, quiet) = app.user("Quinn Quiet", "US").await;
    for (id, email) in [(fan_id, "fay@example.com"), (quiet_id, "quinn@example.com")] {
        sqlx::query("update users set email = $2 where id = $1")
            .bind(id)
            .bind(email)
            .execute(&app.db)
            .await
            .unwrap();
    }
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "later", "coming_soon", "").await;
    listed_with(&app, maker_id, "done", "released", "").await;

    let mark = |cookie: &str, slug: &str| {
        req(Method::PUT, format!("/v1/me/interested/{slug}")).cookie(cookie)
    };
    assert_eq!(
        app.send(mark(&fan, "soon")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(mark(&quiet, "soon")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(mark(&fan, "soon")).await.status(),
        StatusCode::NO_CONTENT,
        "idempotent"
    );
    assert_eq!(
        app.send(mark(&maker, "soon")).await.status(),
        StatusCode::CONFLICT,
        "not your own"
    );
    assert_eq!(
        app.send(mark(&fan, "done")).await.status(),
        StatusCode::CONFLICT,
        "released: library instead"
    );

    // Counts are public; "Most anticipated" sorts by them.
    let slugs = list_slugs(
        &app,
        req(Method::GET, "/v1/apps?sort=anticipated&stage=coming_soon"),
    )
    .await;
    assert_eq!(slugs, ["soon", "later"]);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/soon").cookie(&fan))
            .await,
    )
    .await;
    assert_eq!(page["interested"], 2);
    assert_eq!(page["is_interested"], true);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/interested").cookie(&fan))
            .await,
    )
    .await;
    assert_eq!(mine[0]["slug"], "soon");
    // Signed out, "for you" upcoming is the most anticipated.
    let upcoming = body_json(app.send(req(Method::GET, "/v1/upcoming/for-you")).await).await;
    assert_eq!(upcoming[0]["slug"], "soon");
    assert_eq!(upcoming[0]["reason"], "2 interested");

    // Quinn doesn't want these by email.
    let prefs = body_json(
        app.send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&quiet)
                .json(json!({ "interested": false })),
        )
        .await,
    )
    .await;
    assert_eq!(prefs["categories"][1]["category"], "interested");
    assert_eq!(prefs["categories"][1]["email"], false);
    assert_eq!(
        app.send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&quiet)
                .json(json!({ "spam": true }))
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Moving to Beta tells everyone Interested.
    let soon: Uuid = sqlx::query_scalar("select id from apps where slug = 'soon'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '0.1.0', repeat('a', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(soon)
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    admin::set_stage(&app.db, "soon", slop_api::catalog::AppStage::Beta, "test")
        .await
        .unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&fan)).await).await;
    assert_eq!(me["unread_notifications"], 1);
    let inbox = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&fan))
            .await,
    )
    .await;
    assert_eq!(
        inbox["items"][0]["title"],
        "soon is in beta: you can try it now"
    );
    assert_eq!(inbox["items"][0]["link"], "/app/soon");

    // Emails go only to those who want them.
    let queued = slop_api::notify::queue_emails(&app.db, &app.config)
        .await
        .unwrap();
    assert_eq!(queued, 1);
    let to: Vec<String> = sqlx::query_scalar("select to_address from email_outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(to, ["fay@example.com"]);
    let body: String = sqlx::query_scalar("select body from email_outbox")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(body.contains("http://localhost:8080/app/soon"), "{body}");
    assert!(body.contains("/account#notifications"));

    // Reading clears the count.
    let res = app
        .send(
            req(Method::POST, "/v1/me/notifications/read")
                .cookie(&fan)
                .json(json!({})),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&fan)).await).await;
    assert_eq!(me["unread_notifications"], 0);
}

// ---------------------------------------------------------------------------------------------
// Donations (DESIGN §12, PAYMENTS §5–6)

async fn donations_on(app: &TestApp, maker: &str, slug: &str, servers: bool) {
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(maker)
                .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let res = app
        .send(
            req(Method::PUT, format!("/v1/apps/{slug}/donations/settings"))
                .cookie(maker)
                .json(json!({
                    "donations_on": true, "donation_suggested_minor": 300, "donation_currency": "usd",
                    "runs_servers": servers,
                    "servers_statement": if servers { Some("Pays for two game servers and the leaderboard.") } else { None }
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
}

/// A donation to a mod or community build is shared: 30% to the original's maker first, as its
/// own donation, then the rest to the branch's author. Slop never splits one payment.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donations_to_mods_are_shared_with_the_original(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 1000;
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (modder_id, modder) = app.user("Bo Modder", "US").await;
    let (_, donor) = app.user("Dee Donor", "US").await;
    listed_with(
        &app,
        maker_id,
        "arena",
        "released",
        "repo_url = 'https://github.com/mia/arena'",
    )
    .await;
    donations_on(&app, &maker, "arena", false).await;
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/branches").cookie(&modder).json(json!({
            "slug": "arena-dragons", "title": "Dragons", "summary": "Adds dragons.",
            "repo_url": "https://github.com/bo/arena-dragons", "license_spdx": "MIT", "kind": "addon", "note": "Three dragons."
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let mod_id: Uuid = sqlx::query_scalar("select id from apps where slug = 'arena-dragons'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('b', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(mod_id)
    .bind(modder_id)
    .execute(&app.db)
    .await
    .unwrap();
    admin::list_app(&app.db, "arena-dragons", "test")
        .await
        .unwrap();
    donations_on(&app, &modder, "arena-dragons", false).await;

    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena-dragons/donate").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(info["shared"]["slug"], "arena");
    assert_eq!(info["shared"]["percent"], 30);
    let start = |slug: &str, body: Value| {
        let mut body = body;
        body["confirm_adult"] = json!(true);
        req(Method::POST, format!("/v1/apps/{slug}/donate"))
            .cookie(&donor)
            .json(body)
    };

    // The branch's payment needs the original's share paid first.
    let res = app
        .send(start("arena-dragons", json!({ "amount_minor": 700 })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "share_donation");
    assert_eq!(slop_api::donations::share_for(700), 300);
    let res = app
        .send(start(
            "arena",
            json!({ "amount_minor": 300, "share_for": "arena-dragons", "rest_minor": 700 }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let share = body_json(res).await;
    assert_eq!(share["done"], true);
    let share_id = share["id"].as_str().unwrap().to_string();
    let res = app
        .send(start(
            "arena-dragons",
            json!({ "amount_minor": 800, "share_donation": share_id }),
        ))
        .await;
    assert_eq!(
        body_json(res).await["error"]["field"],
        "amount_minor",
        "the share must be 30%"
    );
    let res = app
        .send(start(
            "arena-dragons",
            json!({ "amount_minor": 700, "share_donation": share_id }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(start(
            "arena-dragons",
            json!({ "amount_minor": 700, "share_donation": share_id }),
        ))
        .await;
    assert_eq!(
        body_json(res).await["error"]["field"],
        "share_donation",
        "each share once"
    );

    let paid: Vec<(String, i32)> = sqlx::query_as(
        "select a.slug, d.donation_minor from donations d join apps a on a.id = d.app_id
         where d.status = 'paid' order by d.created_at",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        paid,
        [
            ("arena".to_string(), 300),
            ("arena-dragons".to_string(), 700)
        ]
    );

    // The original's maker donating to the mod: nothing to share with themselves.
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena-dragons/donate").cookie(&maker))
            .await,
    )
    .await;
    assert!(info["shared"].is_null());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donations_go_to_the_maker_and_give_nothing_back(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 1000;
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "n@slop.test".into(),
        });
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (donor_id, donor) = app.user("Dee Donor", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    sqlx::query("update users set email = 'dee@example.com' where id = $1")
        .bind(donor_id)
        .execute(&app.db)
        .await
        .unwrap();
    listed_with(&app, maker_id, "arena", "released", "").await;

    // Enabling: the tax notice first (PAY-9), then the maker's own account.
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&maker)
                .json(json!({ "tax_notice_ack": false })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "tax_notice_ack");
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(info["on"], false);
    donations_on(&app, &maker, "arena", true).await;
    let before = body_json(
        app.send(req(Method::GET, "/v1/apps/arena").cookie(&other))
            .await,
    )
    .await;

    // The box: presets around the suggestion, each with the total first (PAY-10).
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(info["on"], true);
    let middle = &info["presets"][1];
    assert_eq!(middle["donation"], 300);
    assert_eq!(middle["total"], 340);
    assert_eq!(middle["slop_fee"], 15);
    assert_eq!(middle["maker_receives"], 285);
    assert!(
        info["servers"]["example_consent"]
            .as_str()
            .unwrap()
            .contains("every month until I cancel")
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine["cant"], "It's your own app.");

    // Donating: 18+, then done (simulated: no Stripe on this server).
    let donate = |cookie: &str, body: Value| {
        req(Method::POST, "/v1/apps/arena/donate")
            .cookie(cookie)
            .json(body)
    };
    let res = app
        .send(donate(&donor, json!({ "amount_minor": 300 })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "confirm_adult");
    let res = app
        .send(donate(&donor, json!({ "amount_minor": 300, "confirm_adult": true, "share_name": true, "message": "Love the arena!" })))
        .await;
    let started = body_json(res).await;
    assert_eq!(started["done"], true, "{started}");
    assert_eq!(
        app.send(donate(
            &maker,
            json!({ "amount_minor": 300, "confirm_adult": true })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(donate(&donor, json!({ "amount_minor": 50 })))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Recorded as a direct charge on the maker's account, with exactly 5% of the donation.
    let (total, fee_minor, kind): (i32, i32, String) =
        sqlx::query_as("select amount_minor, slop_fee_minor, kind from payments")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((total, fee_minor, kind.as_str()), (340, 15, "donation"));

    // PAY-5: nothing about the app looks different to anyone because of a donation.
    let after = body_json(
        app.send(req(Method::GET, "/v1/apps/arena").cookie(&other))
            .await,
    )
    .await;
    assert_eq!(before, after);
    let as_donor = body_json(
        app.send(req(Method::GET, "/v1/apps/arena").cookie(&donor))
            .await,
    )
    .await;
    assert!(
        !as_donor.to_string().to_lowercase().contains("donat"),
        "{as_donor}"
    );

    // The donor has the receipt; the maker sees the shared name and message, and totals.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/donations").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(mine["donations"][0]["status"], "paid");
    assert_eq!(mine["donations"][0]["total_minor"], 340);
    let summary = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donations").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(summary["currency_totals"][0]["donated_minor"], 300);
    assert_eq!(summary["shared"][0]["name"], "Dee Donor");
    assert_eq!(summary["shared"][0]["message"], "Love the arena!");
    assert_eq!(
        summary["months"].as_array().unwrap().len(),
        0,
        "fewer than 10: no breakdown"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donations").cookie(&donor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let inbox = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(inbox["items"][0]["title"], "Dee Donor donated to arena");

    // The maker refunds a mistake (PAY-3).
    let id = summary["recent"][0]["id"].as_str().unwrap();
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/donations/{id}/refund")).cookie(&donor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/donations/{id}/refund")).cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/donations").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(mine["donations"][0]["status"], "refunded");

    // A server donation: the exact monthly terms, confirmed with an unticked box and saved.
    let monthly = json!({ "amount_minor": 500, "monthly": true });
    assert_eq!(
        body_json(app.send(donate(&donor, monthly.clone())).await).await["error"]["field"],
        "consent"
    );
    let mut ok = monthly.clone();
    ok["consent"] = json!(true);
    assert_eq!(
        body_json(app.send(donate(&donor, ok.clone())).await).await["done"],
        true
    );
    assert_eq!(
        app.send(donate(&donor, ok.clone())).await.status(),
        StatusCode::CONFLICT,
        "one per app"
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/donations").cookie(&donor))
            .await,
    )
    .await;
    let sub = &mine["server_donations"][0];
    assert_eq!(sub["status"], "active");
    assert!(
        sub["consent_text"]
            .as_str()
            .unwrap()
            .contains("$5.46 every month until I cancel"),
        "{sub}"
    );
    // A month on: renewed (simulated), and reminded first (PAY-16), by email too.
    sqlx::query("update server_donations set next_charge_at = now() - interval '1 minute'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::donations::hourly(&app.state).await.unwrap();
    let months: i64 = sqlx::query_scalar(
        "select count(*) from donations where kind = 'monthly' and status = 'paid'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(months, 2);
    let reminders: i64 =
        sqlx::query_scalar("select count(*) from email_outbox where subject like '%renews on%'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(reminders, 1);
    // Account closing waits for it.
    let res = app
        .send(
            req(Method::POST, "/v1/me/close")
                .cookie(&maker)
                .json(json!({ "confirm": "close my account" })),
        )
        .await;
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("servers")
    );

    // Shutting the servers down cancels every server donation and tells the donors (PAY-17).
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(body_json(res).await["cancelled"], 1);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/donations").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(
        mine["server_donations"][0]["cancelled_by"],
        "servers_shut_down"
    );
    let told: i64 =
        sqlx::query_scalar("select count(*) from email_outbox where subject like '%has ended%'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(told, 1);
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await,
    )
    .await;
    assert!(info["servers"].is_null());

    // Where a country's setting is off, the donor's amount is the total (PAY-12).
    sqlx::query("update country_policies set donor_covers_fees = false where country_code = 'US'")
        .execute(&app.db)
        .await
        .unwrap();
    let q = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate/quote?amount_minor=300").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(
        (q["total"].as_i64(), q["maker_receives"].as_i64()),
        (Some(300), Some(300 - 15 - 39))
    );

    // Fee statements for the maker.
    let statements = body_json(
        app.send(req(Method::GET, "/v1/me/fee-statements").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(statements["statements"][0]["kind"], "donation");
    assert!(
        statements["statements"][0]["slop_fee_minor"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert_eq!(
        statements["vat"]["kind"], "none",
        "no VAT on Slop's fee in the US"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_donations_are_direct_charges_named_as_donations(db: PgPool) {
    let stripe_base = fake_stripe().await;
    let app = test_app_with(db, |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: "whsec_slop".into(),
            api_base: stripe_base.parse().unwrap(),
        })
    })
    .await;
    let (maker_id, maker) = app.user("Vera", "US").await;
    let (_, donor) = app.user("Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let on = body_json(
        app.send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&maker)
                .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
        )
        .await,
    )
    .await;
    assert_eq!(on["status"], "active", "{on}");
    donations_on(&app, &maker, "arena", false).await;
    let started = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/donate")
                .cookie(&donor)
                .json(json!({ "amount_minor": 300, "confirm_adult": true })),
        )
        .await,
    )
    .await;
    let url = started["url"].as_str().unwrap();
    let session = url.rsplit('/').next().unwrap();
    // The Checkout session Slop asked for: the donor's total, Slop's 5% of the donation, "donation".
    let s: Value = reqwest::Client::new()
        .get(format!("{stripe_base}v1/checkout/sessions/{session}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["form"]["line_items[0][price_data][unit_amount]"], "340");
    assert_eq!(
        s["form"]["payment_intent_data[application_fee_amount]"],
        "15"
    );
    assert_eq!(
        s["form"]["payment_intent_data[description]"],
        "Donation to arena"
    );
    assert!(
        s["form"]
            .get("payment_intent_data[transfer_data][destination]")
            .is_none(),
        "PAY-1: never transfer_data"
    );
    // Paid on Stripe, confirmed by a signed webhook; the state comes from Stripe's API.
    reqwest::Client::new()
        .post(format!("{stripe_base}test/pay/{session}"))
        .send()
        .await
        .unwrap();
    let event = json!({
        "id": "evt_don_1", "type": "checkout.session.completed", "account": "acct_vera",
        "data": { "object": { "id": session, "metadata": { "donation_id": started["id"] } } }
    });
    let (body, sig) = stripe_signed(&event, "whsec_slop");
    let res = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/stripe/webhook")
                .header("stripe-signature", sig)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let (status, object): (String, String) =
        sqlx::query_as("select status, stripe_object_id from donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), object.as_str()),
        ("paid", format!("ch_{session}").as_str())
    );
}

// ---------------------------------------------------------------------------------------------
// Written reviews and profiles (DESIGN §7)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviews_go_with_votes_and_profiles_follow_the_privacy_choice(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    listed_with(&app, maker, "tower", "released", "").await;
    let (us_id, us) = app.user("Uma US", "US").await;
    let (uk_id, uk) = app.user("Kit UK", "GB").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (_, reporter) = app.user("Rita Reporter", "US").await;
    for cookie in [&us, &uk] {
        app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(cookie))
            .await;
    }
    let review = |cookie: &str, body: &str| {
        req(Method::PUT, "/v1/apps/tower/review")
            .cookie(cookie)
            .json(json!({ "body": body }))
    };
    // A review goes with a vote.
    assert_eq!(
        app.send(review(&us, "Great tower defence.")).await.status(),
        StatusCode::CONFLICT
    );
    for (cookie, v) in [(&us, "not_slop"), (&uk, "slop")] {
        let res = app
            .send(
                req(Method::POST, "/v1/apps/tower/vote")
                    .cookie(cookie)
                    .json(json!({ "vote": v })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    assert_eq!(
        app.send(review(&us, "Great tower defence.")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(review(&uk, "Too grindy after level 5."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(review(&us, "  ")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // On the app page: public profiles by name; the UK under-18 default is private (PRIV-6).
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(
        (page["not_slop"].as_i64(), page["slop"].as_i64()),
        (Some(1), Some(1)),
        "{page}"
    );
    let by = |name: &str| {
        page["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["body"].as_str().unwrap().contains(name))
            .unwrap()
            .clone()
    };
    let uma = by("Great");
    assert_eq!(uma["author"], "Uma US");
    assert_eq!(uma["author_id"], us_id.to_string());
    let kit = by("grindy");
    assert_eq!(kit["author"], "A player");
    assert!(kit["author_id"].is_null());
    let only_slop = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/reviews?vote=slop"))
            .await,
    )
    .await;
    assert_eq!(only_slop["reviews"].as_array().unwrap().len(), 1);
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/ratings").cookie(&us))
            .await,
    )
    .await;
    assert_eq!(ratings["mine"]["review"]["body"], "Great tower defence.");

    // Profiles: votes and reviews when public.
    let p = body_json(
        app.send(req(Method::GET, format!("/v1/users/{us_id}")))
            .await,
    )
    .await;
    assert_eq!(p["public"], true);
    assert_eq!(p["votes"][0]["slug"], "tower");
    assert_eq!(p["votes"][0]["my_vote"], "not_slop");
    assert_eq!(p["votes"][0]["review"], "Great tower defence.");
    let p = body_json(
        app.send(req(Method::GET, format!("/v1/users/{uk_id}")))
            .await,
    )
    .await;
    assert_eq!(p["public"], false);
    assert_eq!(p["votes"].as_array().unwrap().len(), 0);
    let own = body_json(
        app.send(req(Method::GET, format!("/v1/users/{uk_id}")).cookie(&uk))
            .await,
    )
    .await;
    assert_eq!(
        own["votes"].as_array().unwrap().len(),
        1,
        "you always see your own"
    );
    app.send(
        req(Method::PATCH, "/v1/me")
            .cookie(&uk)
            .json(json!({ "profile_public": true })),
    )
    .await;
    let p = body_json(
        app.send(req(Method::GET, format!("/v1/users/{uk_id}")))
            .await,
    )
    .await;
    assert_eq!(p["public"], true);

    // Withdrawing the vote takes the review off the page.
    app.send(
        req(Method::POST, "/v1/apps/tower/vote")
            .cookie(&uk)
            .json(json!({ "vote": null })),
    )
    .await;
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(page["reviews"].as_array().unwrap().len(), 1);

    // Reported and hidden by a decision, never for being negative (REV-2).
    let id = uma["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&reporter)
                .json(json!({
                    "kind": "app_review", "subject": id, "reason": "private_info",
                    "details": "Contains someone's phone number.", "good_faith": true
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(
        queue["reports"][0]["link"],
        format!("/app/tower#review-{id}")
    );
    let report_id = queue["reports"][0]["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{report_id}/act")).cookie(&admin).json(json!({
                "action": "hide_review", "rule": "private_info", "facts": "The review includes a phone number.",
                "outcome": "Hidden."
            })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(page["reviews"].as_array().unwrap().len(), 0);
    let ratings = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/ratings").cookie(&us))
            .await,
    )
    .await;
    assert_eq!(ratings["mine"]["review"]["hidden"], true);
}

// ---------------------------------------------------------------------------------------------
// Branches and community builds (DESIGN §9)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn branches_are_community_builds_on_the_original_page(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (author_id, author) = app.user("Bo Brancher", "US").await;
    listed_with(
        &app,
        maker,
        "castle",
        "released",
        "platforms = '{windows}', repo_url = 'https://github.com/mia/castle', min_age = 12,
         content_labels = '{violence}', online = '{player_servers}'",
    )
    .await;
    let branch =
        |slug: &str, repo: &str| {
            req(Method::POST, "/v1/apps/castle/branches").cookie(&author).json(json!({
            "slug": slug, "title": "Castle (Linux port)", "summary": "Runs natively on Linux.",
            "repo_url": repo, "license_spdx": "MIT", "kind": "port",
            "note": "Adds a Linux build and fixes paths.", "platforms": ["linux", "linux-deb"]
        }))
        };
    let res = app
        .send(branch("castle-linux", "https://github.com/mia/castle"))
        .await;
    assert_eq!(
        body_json(res).await["error"]["field"],
        "repo_url",
        "a fork, not the original"
    );
    let res = app
        .send(branch("castle-linux", "https://github.com/bo/castle"))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    // It's asked what a new app is asked; left out, the original's answers carry over, and it
    // can't be rated below the original.
    let own = body_json(
        app.send(req(Method::GET, "/v1/apps/castle-linux").cookie(&author))
            .await,
    )
    .await;
    assert_eq!(own["min_age"], 12);
    assert_eq!(own["content_labels"], json!(["violence"]));
    assert_eq!(own["online"], json!(["player_servers"]));
    assert_eq!(own["sandbox_ready"], false);
    for (field, value) in [("min_age", json!(7)), ("content_labels", json!(["horror"]))] {
        let res = app
            .send(
                req(Method::PATCH, "/v1/apps/castle-linux")
                    .cookie(&author)
                    .json(json!({ field: value })),
            )
            .await;
        assert_eq!(
            body_json(res).await["error"]["field"],
            field,
            "not below the original"
        );
    }
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/castle-linux")
                .cookie(&author)
                .json(json!({ "min_age": 16, "content_labels": ["violence", "horror"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "adding is fine");
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/branches").cookie(&author).json(json!({
            "slug": "castle-kids", "title": "Castle for kids", "summary": "Gentler.",
            "repo_url": "https://github.com/bo/castle-kids", "license_spdx": "MIT", "kind": "mod",
            "note": "No monsters.", "platforms": ["windows"], "min_age": 3, "content_labels": []
        })))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "min_age");
    assert_eq!(
        app.send(branch("castle-linux", "https://github.com/bo/castle2"))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app
        .send(req(Method::POST, "/v1/apps/castle-linux/branches").cookie(&author).json(json!({
            "slug": "x", "title": "x", "summary": "x", "repo_url": "https://github.com/z/z", "license_spdx": "MIT",
            "kind": "fix", "note": "", "platforms": ["windows"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "branch the original");

    // A draft until a release is approved and it's listed, like any app.
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = 'castle-linux'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('b', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(author_id)
    .execute(&app.db)
    .await
    .unwrap();
    admin::list_app(&app.db, "castle-linux", "test")
        .await
        .unwrap();

    // Not in the store on its own; in the Community list; on the original's page, filling the gap.
    let store = list_slugs(&app, req(Method::GET, "/v1/apps")).await;
    assert_eq!(store, ["castle"]);
    let community = list_slugs(&app, req(Method::GET, "/v1/apps?community=true")).await;
    assert_eq!(community, ["castle-linux"]);
    let page = body_json(app.send(req(Method::GET, "/v1/apps/castle")).await).await;
    assert_eq!(page["community"][0]["slug"], "castle-linux");
    assert_eq!(page["community"][0]["author"], "Bo Brancher");
    assert_eq!(page["community"][0]["adds"], json!(["linux", "linux-deb"]));
    let own = body_json(app.send(req(Method::GET, "/v1/apps/castle-linux")).await).await;
    assert_eq!(own["branch"]["parent_slug"], "castle");
    assert_eq!(own["branch"]["kind_label"], "Port");
    assert_eq!(own["branch_of"], "castle");
    assert_eq!(own["maker"]["display_name"], "Bo Brancher");
}

/// Mods (Content Policy, "Code from outside Slop"): anyone publishes one for any app, it's
/// checked and reviewed like an app, and it's listed only on the app's page, never on its own.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn mods_are_listed_on_their_game_page(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (_, author) = app.user("Bo Modder", "US").await;
    listed_with(
        &app,
        maker,
        "castle",
        "released",
        "platforms = '{windows}', repo_url = 'https://github.com/mia/castle'",
    )
    .await;
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/branches").cookie(&author).json(json!({
            "slug": "castle-dragons", "title": "Dragons", "summary": "Adds dragons to Castle.",
            "repo_url": "https://github.com/bo/castle-dragons", "license_spdx": "MIT", "kind": "addon",
            "note": "Three new dragons and a lair."
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/castle-dragons").cookie(&author))
            .await,
    )
    .await;
    assert_eq!(
        page["platforms"],
        json!(["any"]),
        "one .zip for every platform"
    );
    assert_eq!(page["branch"]["kind_label"], "Mod");

    // Its release is one .zip with nothing to start.
    let release = |file: Value| {
        req(Method::POST, "/v1/apps/castle-dragons/releases")
            .cookie(&author)
            .json(json!({ "version": "1.0.0", "commit_sha": "a".repeat(40), "files": [file] }))
    };
    let url = "https://github.com/bo/castle-dragons/releases/download/v1.0.0/dragons.zip";
    let res = app
        .send(release(
            json!({ "platform": "any", "kind": "archive", "url": url, "exec": "dragons.exe" }),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "no program to start"
    );
    let res = app
        .send(release(
            json!({ "platform": "windows", "kind": "archive", "url": url, "exec": "x.exe" }),
        ))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "mods aren't per platform"
    );
    let res = app
        .send(release(
            json!({ "platform": "any", "kind": "archive", "url": url }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release_id: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/castle-dragons")
                .cookie(&author)
                .json(json!({ "platforms": ["windows"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Once approved and listed: on the game's page, not in the store or the Community shelf.
    // Fetched and scanned (the fake host isn't involved here), then approved: the signed
    // manifest names the game, so the Slop app adds it only to that game.
    sqlx::query(
        "update release_files set size_bytes = 10, sha256 = repeat('a', 64), scan_result = 'passed'
         where release_id = $1",
    )
    .bind(release_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update releases set status = 'awaiting_review' where id = $1")
        .bind(release_id)
        .execute(&app.db)
        .await
        .unwrap();
    admin::approve(&app.db, Some(&app.signing_key), lax(), release_id, "rev")
        .await
        .unwrap();
    admin::list_app(&app.db, "castle-dragons", "test")
        .await
        .unwrap();
    let signed: slop_manifest::Signed = serde_json::from_value(
        body_json(
            app.send(req(Method::GET, "/v1/apps/castle-dragons/manifest"))
                .await,
        )
        .await,
    )
    .unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    let castle_id: Uuid = sqlx::query_scalar("select id from apps where slug = 'castle'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        manifest.mod_for,
        Some(slop_manifest::ModFor {
            app_id: castle_id.to_string(),
            app_slug: "castle".into(),
            app_title: "castle".into(),
        })
    );
    assert!(manifest.files[0].launch.is_none());
    assert_eq!(manifest.build_of, None);

    // A mod with code that goes online is flagged, and suggests sandbox mode (DESIGN §9).
    sqlx::query(
        "update releases set repo_scan = '{\"status\": \"checked\", \"mod_code\": true, \"mod_online\": true}'
         where id = $1",
    )
    .bind(release_id)
    .execute(&app.db)
    .await
    .unwrap();
    let page = body_json(app.send(req(Method::GET, "/v1/apps/castle")).await).await;
    let listed = &page["community"][0];
    assert_eq!(
        (&listed["slug"], &listed["mod_code"], &listed["mod_online"]),
        (&json!("castle-dragons"), &json!(true), &json!(true))
    );
    let dragons = body_json(app.send(req(Method::GET, "/v1/apps/castle-dragons")).await).await;
    assert_eq!(dragons["sandbox_suggested"], true);

    // The game's own mods folder is signed into its builds' manifests; community builds take
    // the original's.
    sqlx::query(
        "update apps set mods_folder = '{\"base\": \"game\", \"path\": \"mods\", \"layout\": \"folder\"}'
         where slug = 'castle'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    // A community build's manifest names its original instead, so the Slop app can offer it
    // on the original's Play button.
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/branches").cookie(&author).json(json!({
            "slug": "castle-hd", "title": "Castle HD", "summary": "Sharper.",
            "repo_url": "https://github.com/bo/castle-hd", "license_spdx": "MIT", "kind": "mod",
            "note": "Bigger textures.", "platforms": ["windows"]
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = app
        .send(
            req(Method::POST, "/v1/apps/castle-hd/releases")
                .cookie(&author)
                .json(
                    json!({ "version": "1.0.0", "commit_sha": "c".repeat(40), "files": [{
                    "platform": "windows", "kind": "archive", "exec": "castle.exe",
                    "url": "https://github.com/bo/castle-hd/releases/download/v1.0.0/castle-hd.zip"
                }] }),
                ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let hd_release: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    sqlx::query(
        "update release_files set size_bytes = 10, sha256 = repeat('a', 64), scan_result = 'passed'
         where release_id = $1",
    )
    .bind(hd_release)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update releases set status = 'awaiting_review' where id = $1")
        .bind(hd_release)
        .execute(&app.db)
        .await
        .unwrap();
    admin::approve(&app.db, Some(&app.signing_key), lax(), hd_release, "rev")
        .await
        .unwrap();
    let signed: slop_manifest::Signed = serde_json::from_value(
        sqlx::query_scalar("select signed_manifest from releases where id = $1")
            .bind(hd_release)
            .fetch_one(&app.db)
            .await
            .unwrap(),
    )
    .unwrap();
    let hd: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert_eq!(hd.mod_for, None);
    assert_eq!(hd.build_of.map(|o| o.app_slug).as_deref(), Some("castle"));
    assert_eq!(
        hd.mods_folder,
        Some(slop_manifest::ModsFolder {
            base: slop_manifest::ModsBase::Game,
            path: "mods".into(),
            layout: slop_manifest::ModsLayout::Folder,
        })
    );
    let patch = |body: Value| {
        req(Method::PATCH, "/v1/apps/castle-hd")
            .cookie(&author)
            .json(body)
    };
    let res = app
        .send(patch(
            json!({ "mods_folder": { "base": "game", "path": "../escape", "layout": "flat" } }),
        ))
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "mods_folder");
    let res = app.send(patch(json!({ "mods_folder": null }))).await;
    assert_eq!(body_json(res).await["mods_folder"], Value::Null, "cleared");
    let res = app
        .send(patch(
            json!({ "mods_folder": { "base": "saves", "path": "Mods", "layout": "flat" } }),
        ))
        .await;
    assert_eq!(body_json(res).await["mods_folder"]["base"], "saves");

    // The library's update check lists it under the installed game.
    let updates = body_json(
        app.send(req(Method::GET, "/v1/library/updates?apps=castle"))
            .await,
    )
    .await;
    assert_eq!(updates[0]["mods"][0]["slug"], "castle-dragons", "{updates}");
    assert_eq!(updates[0]["mods"][0]["author"], "Bo Modder");
    assert_eq!(updates[0]["mods"][0]["code"], true);
    assert_eq!(updates[0]["mods"][0]["online"], true);
    assert_eq!(updates[0]["builds"], json!([]));
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps")).await,
        ["castle"]
    );
    assert!(
        list_slugs(&app, req(Method::GET, "/v1/apps?community=true"))
            .await
            .is_empty()
    );
    let page = body_json(app.send(req(Method::GET, "/v1/apps/castle")).await).await;
    assert_eq!(page["community"][0]["slug"], "castle-dragons");
    assert_eq!(page["community"][0]["branch_kind"], "addon");
    assert_eq!(page["community"][0]["adds"], json!([]));
}

/// Sandbox mode (DESIGN §10): makers declare an app works in it, it's suggested for apps that go
/// online, and players report apps that don't work in it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sandbox_mode_declarations_suggestions_and_reports(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker_id,
        "castle",
        "released",
        "online = '{player_servers}'",
    )
    .await;
    listed_with(&app, maker_id, "quiet", "released", "").await;
    listed_with(
        &app,
        maker_id,
        "hidden",
        "released",
        "visibility = 'private'",
    )
    .await;
    let page = |slug: &str, cookie: Option<&str>| {
        let mut r = req(Method::GET, format!("/v1/apps/{slug}"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        let app = &app;
        async move { body_json(app.send(r).await).await }
    };

    let castle = page("castle", None).await;
    assert_eq!(castle["sandbox_ready"], false);
    assert_eq!(castle["sandbox_suggested"], true, "it plays online");
    assert_eq!(castle["sandbox_trouble"], false);
    assert_eq!(castle["sandbox_reported"], false);
    assert_eq!(page("quiet", None).await["sandbox_suggested"], false);

    // The maker's declaration.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/quiet")
                .cookie(&maker)
                .json(json!({ "sandbox_ready": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(page("quiet", None).await["sandbox_ready"], true);
    assert_eq!(page("quiet", None).await["sandbox_mismatch"], false);

    // Its newest release's check decides: saves outside SLOP_SAVE_DIR undo the declaration for
    // players, and undeclared networking suggests sandbox mode.
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, signed_manifest, repo_scan)
         select $1, a.id, '1.0.0', repeat('a', 40), 'approved', a.maker_id, now(), '{}'::jsonb,
                '{\"status\": \"checked\", \"save_folder\": \"elsewhere\", \"online_undeclared\": true}'::jsonb
         from apps a where a.slug = 'quiet'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let quiet = page("quiet", None).await;
    assert_eq!(quiet["sandbox_ready"], true, "still what the maker said");
    assert_eq!(quiet["sandbox_mismatch"], true);
    assert_eq!(quiet["sandbox_suggested"], true);

    // Players' reports: three mark it, each player once, and they can take it back.
    let report = |method: Method, slug: &str, cookie: Option<&str>| {
        let mut r = req(method, format!("/v1/apps/{slug}/sandbox-report"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        let app = &app;
        async move { app.send(r).await.status() }
    };
    assert_eq!(
        report(Method::PUT, "quiet", None).await,
        StatusCode::UNAUTHORIZED
    );
    let mut players = Vec::new();
    // Only players who've started it can report it.
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        report(Method::PUT, "quiet", Some(&stranger)).await,
        StatusCode::CONFLICT
    );
    for name in ["Ann", "Bo", "Cy"] {
        let (id, cookie) = app.user(name, "US").await;
        sqlx::query(
            "insert into user_app_usage (user_id, app_id, days_launched) select $1, id, 1 from apps where slug = 'quiet'",
        )
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
        assert_eq!(
            report(Method::PUT, "quiet", Some(&cookie)).await,
            StatusCode::NO_CONTENT
        );
        players.push(cookie);
    }
    assert_eq!(
        report(Method::PUT, "quiet", Some(&players[0])).await,
        StatusCode::NO_CONTENT,
        "again is fine"
    );
    let quiet = page("quiet", Some(&players[0])).await;
    assert_eq!(quiet["sandbox_trouble"], true);
    assert_eq!(quiet["sandbox_reported"], true);
    assert_eq!(page("quiet", Some(&maker)).await["sandbox_reported"], false);
    assert_eq!(
        report(Method::DELETE, "quiet", Some(&players[0])).await,
        StatusCode::NO_CONTENT
    );
    let quiet = page("quiet", Some(&players[0])).await;
    assert_eq!(quiet["sandbox_trouble"], false, "two reports left");
    assert_eq!(quiet["sandbox_reported"], false);
    for slug in ["hidden", "nope"] {
        assert_eq!(
            report(Method::PUT, slug, Some(&players[1])).await,
            StatusCode::NOT_FOUND
        );
    }

    // The library's update check carries the same, for the Slop app's switch and prompt.
    let updates = body_json(
        app.send(req(Method::GET, "/v1/library/updates?apps=castle,quiet"))
            .await,
    )
    .await;
    assert_eq!(updates[0]["slug"], "castle");
    assert_eq!(updates[0]["sandbox_suggested"], true);
    assert_eq!(updates[1]["sandbox_ready"], true);
    assert_eq!(updates[1]["sandbox_trouble"], false);
}

// ---------------------------------------------------------------------------------------------
// Private Slop (DESIGN §13)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn private_slop_is_seen_only_by_who_it_is_shared_with(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let mut body = new_app("secret-tool");
    body["visibility"] = json!("private");
    body["repo_url"] = json!("https://github.com/octo/private-tool");
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let detail = body_json(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(detail["private"], true);

    // Sharing: by profile link; not visible until it has an approved release.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret-tool/shares")
                .cookie(&owner)
                .json(json!({ "user": format!("http://localhost:8080/u/{friend_id}") })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await[0]["name"], "Finn Friend");
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/apps/secret-tool/shares")
                .cookie(&friend)
                .json(json!({ "user": owner_id.to_string() }))
        )
        .await
        .status(),
        StatusCode::NOT_FOUND,
        "only the owner shares"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&friend))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = 'secret-tool'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('c', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&friend))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/private").cookie(&friend))
            .await,
    )
    .await;
    assert_eq!(mine["shared"][0]["slug"], "secret-tool");
    assert_eq!(mine["shared"][0]["owner"], "Olive Owner");
    let inbox = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&friend))
            .await,
    )
    .await;
    assert_eq!(
        inbox["items"][0]["title"],
        "Olive Owner shared Pixel Dungeon Delver with you"
    );

    // Never in the store; no listing, donations or voting.
    assert!(
        list_slugs(&app, req(Method::GET, "/v1/apps").cookie(&friend))
            .await
            .is_empty()
    );
    assert!(
        admin::list_app(&app.db, "secret-tool", "test")
            .await
            .is_err()
    );
    let res = app
        .send(req(Method::GET, "/v1/apps/secret-tool/donate").cookie(&friend))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "no donations");
    app.send(req(Method::POST, "/v1/apps/secret-tool/launch").cookie(&friend))
        .await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret-tool/vote")
                .cookie(&friend)
                .json(json!({ "vote": "not_slop" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // Unsharing takes it away.
    let res = app
        .send(
            req(
                Method::DELETE,
                format!("/v1/apps/secret-tool/shares/{friend_id}"),
            )
            .cookie(&owner),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&friend))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// Teams (DESIGN §14)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn player_teams_share_picks_nights_and_private_slop(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let create =
        |cookie: &str, body: Value| req(Method::POST, "/v1/teams").cookie(cookie).json(body);
    let res = app
        .send(create(
            &owner,
            json!({ "slug": "night-owls", "name": "Night Owls", "kind": "player" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    // Invite-only by default: not visible or joinable for others.
    assert_eq!(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/night-owls/join").cookie(&stranger))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let res = app
        .send(
            req(Method::POST, "/v1/teams/night-owls/invites")
                .cookie(&owner)
                .json(json!({ "user": format!("/u/{pal_id}") })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/teams").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(mine["invites"][0]["slug"], "night-owls");
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(page["invited"], true);
    assert_eq!(
        page["members"].as_array().unwrap().len(),
        0,
        "invitees don't see members yet"
    );
    app.send(req(Method::POST, "/v1/teams/night-owls/invites/accept").cookie(&pal))
        .await;

    // Picks and a game night (the others hear about it).
    listed_with(&app, owner_id, "co-op-caves", "released", "").await;
    let res = app
        .send(
            req(Method::POST, "/v1/teams/night-owls/picks")
                .cookie(&pal)
                .json(json!({ "app_slug": "co-op-caves", "note": "Great with 4" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let starts = (OffsetDateTime::now_utc() + time::Duration::days(2))
        .format(&Rfc3339)
        .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/teams/night-owls/events").cookie(&pal).json(json!({ "title": "Cave night", "starts_at": starts, "app_slug": "co-op-caves" })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(page["picks"][0]["slug"], "co-op-caves");
    assert_eq!(page["picks"][0]["note"], "Great with 4");
    assert_eq!(page["events"][0]["title"], "Cave night");
    assert_eq!(page["members"].as_array().unwrap().len(), 2);
    let inbox = body_json(
        app.send(req(Method::GET, "/v1/me/notifications").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(inbox["items"][0]["title"], "Night Owls: Cave night");

    // Private Slop shared with the team: members see it, others don't.
    let mut body = new_app("team-tool");
    body["visibility"] = json!("private");
    app.send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = 'team-tool'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('d', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/team-tool/shares")
                .cookie(&owner)
                .json(json!({ "team": "night-owls" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await[0]["team"], "night-owls");
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/team-tool").cookie(&pal))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/team-tool").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(page["shared"][0]["slug"], "team-tool");

    // Leaving: the last owner hands over first; the last one out ends the team.
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/night-owls/leave").cookie(&owner))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    app.send(
        req(
            Method::PUT,
            format!("/v1/teams/night-owls/members/{pal_id}"),
        )
        .cookie(&owner)
        .json(json!({ "role": "owner" })),
    )
    .await;
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/night-owls/leave").cookie(&owner))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/team-tool").cookie(&pal))
            .await
            .status(),
        StatusCode::OK
    );
    app.send(req(Method::POST, "/v1/teams/night-owls/leave").cookie(&pal))
        .await;
    let left: i64 = sqlx::query_scalar("select count(*) from teams")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn official_teams_are_verified_through_their_github_organization(db: PgPool) {
    let app = test_app_with(db, |config| {
        config.releases.github_app = Some(slop_api::config::GithubAppConfig {
            app_id: "123".into(),
            slug: "slop-test".into(),
            private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                "../fixtures/github-app-test-key.pem"
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
    listed_with(
        &app,
        owner_id,
        "rocket-run",
        "released",
        "repo_url = 'https://github.com/acme/rocket-run'",
    )
    .await;
    listed_with(
        &app,
        owner_id,
        "side-thing",
        "released",
        "repo_url = 'https://github.com/ada/side-thing'",
    )
    .await;

    // Owner and 2FA pass; the domain doesn't yet (no GitHub badge, no DNS record).
    let res = body_json(
        app.send(req(Method::POST, "/v1/teams/acme/verify").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(res["verified"], false, "{res}");
    let check = |name: &str| {
        res["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["check"] == name)
            .unwrap()["ok"]
            .clone()
    };
    assert_eq!(check("owner"), true);
    assert_eq!(check("two_factor"), true);
    assert_eq!(check("domain"), false);
    // Not verified: not listed, no badge on apps.
    let teams = body_json(app.send(req(Method::GET, "/v1/teams")).await).await;
    assert!(teams.as_array().unwrap().is_empty());

    // The DNS record proves the domain.
    app.send(
        req(Method::PATCH, "/v1/teams/acme")
            .cookie(&owner)
            .json(json!({ "dns_domain": "acme.example" })),
    )
    .await;
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/acme").cookie(&owner))
            .await,
    )
    .await;
    *DNS_TXT.lock().unwrap() = Some(page["dns"]["value"].as_str().unwrap().to_string());
    let res = body_json(
        app.send(req(Method::POST, "/v1/teams/acme/verify").cookie(&owner))
            .await,
    )
    .await;
    *DNS_TXT.lock().unwrap() = None;
    assert_eq!(res["verified"], true, "{res}");

    // Verified: listed, its organization's apps shown, and badged on their pages.
    let page = body_json(app.send(req(Method::GET, "/v1/teams/acme")).await).await;
    assert_eq!(page["verified"], true);
    let apps: Vec<&str> = page["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        apps,
        ["rocket-run"],
        "only apps from the organization's repositories"
    );
    let detail = body_json(app.send(req(Method::GET, "/v1/apps/rocket-run")).await).await;
    assert_eq!(detail["official_team"]["name"], "Acme Games");
    let other = body_json(app.send(req(Method::GET, "/v1/apps/side-thing")).await).await;
    assert!(other["official_team"].is_null());

    // Never Private Slop, never picks.
    let mut body = new_app("secret");
    body["visibility"] = json!("private");
    app.send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret/shares")
                .cookie(&owner)
                .json(json!({ "team": "acme" })),
        )
        .await;
    assert_eq!(body_json(res).await["error"]["field"], "team");
}

// ---------------------------------------------------------------------------------------------
// Search filters, frozen apps, going public, Pulse in community builds, donation safeguards, VAT

/// Weighted 👍 votes from new players, enough for a rating.
async fn votes_for(app: &TestApp, slug: &str, up: usize, down: usize) {
    for (i, vote) in std::iter::repeat_n(1i16, up)
        .chain(std::iter::repeat_n(-1i16, down))
        .enumerate()
    {
        let (voter, _) = app.user(&format!("Voter {slug} {i}"), "US").await;
        sqlx::query(
            "insert into votes (user_id, app_id, vote, weight) select $1, id, $2, 1 from apps where slug = $3",
        )
        .bind(voter)
        .bind(vote)
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn search_finds_what_runs_on_a_platform_including_community_builds(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (author, _) = app.user("Bo Brancher", "US").await;
    listed_with(&app, maker, "castle", "released", "platforms = '{windows}'").await;
    listed_with(
        &app,
        maker,
        "tower",
        "released",
        "platforms = '{linux,linux-deb}'",
    )
    .await;
    listed_with(&app, maker, "moat", "beta", "platforms = '{windows}'").await;
    listed_with(
        &app,
        author,
        "castle-linux",
        "released",
        "platforms = '{linux,linux-deb}', parent_app_id = (select id from apps where slug = 'castle'), branch_kind = 'port'",
    )
    .await;
    let mut linux = list_slugs(&app, req(Method::GET, "/v1/apps?platform=linux")).await;
    linux.sort();
    assert_eq!(
        linux,
        ["castle", "tower"],
        "castle runs on Linux through its port"
    );
    let mut windows = list_slugs(&app, req(Method::GET, "/v1/apps?platform=windows")).await;
    windows.sort();
    assert_eq!(windows, ["castle", "moat"]);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps?platform=amiga"))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // A delisted port doesn't count.
    sqlx::query("update apps set status = 'delisted' where slug = 'castle-linux'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?platform=linux")).await,
        ["tower"]
    );

    // Upcoming or released, % Not Slop, and the sorts.
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?upcoming=true")).await,
        ["moat"]
    );
    votes_for(&app, "castle", 9, 1).await;
    votes_for(&app, "tower", 3, 3).await;
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?min_not_slop=80")).await,
        ["castle"]
    );
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?sort=rating")).await[..2],
        ["castle", "tower"]
    );
    let (reviewer, _) = app.user("Rita Reviewer", "US").await;
    sqlx::query(
        "insert into app_reviews (id, user_id, app_id, body) select $1, $2, id, 'Great.' from apps where slug = 'moat'",
    )
    .bind(Uuid::now_v7())
    .bind(reviewer)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?sort=reviewed")).await[0],
        "moat"
    );
    // Popular this month: most added to libraries since the month began.
    sqlx::query(
        "insert into library_entries (user_id, app_id) select $1, id from apps where slug = 'tower'",
    )
    .bind(reviewer)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps?sort=popular")).await[0],
        "tower"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn apps_freeze_when_slop_cannot_read_the_repository(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    let repo = format!("{}octo/game", app.fake_base);

    // Readable: nothing happens.
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 1);
    assert_eq!(
        slop_api::frozen::run_once(&app.state).await.unwrap(),
        0,
        "checked daily"
    );
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(overview["frozen"], Value::Null);

    // The repository goes away: frozen, the maker is told, and no new releases.
    sqlx::query("update apps set repo_url = $1, repo_checked_at = null")
        .bind(format!("{}octo/gone", app.fake_base))
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::frozen::run_once(&app.state).await.unwrap();
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        overview["frozen"]["reason"]
            .as_str()
            .unwrap()
            .contains("can't be read")
    );
    let notices: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and category = 'apps'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(notices, 1);
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "2.0.0", "game-windows.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps")).await,
        ["game"],
        "still in the store during the grace period"
    );

    // 14 days later it leaves the store.
    sqlx::query("update apps set frozen_at = now() - interval '15 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::frozen::end_grace(&app.state).await.unwrap(), 1);
    assert!(
        list_slugs(&app, req(Method::GET, "/v1/apps"))
            .await
            .is_empty()
    );

    // Access back: "Check now" unfreezes it and puts it back in the store.
    sqlx::query("update apps set repo_url = $1")
        .bind(&repo)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/apps/game/repo-check").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, Value::Null);
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps")).await,
        ["game"]
    );
    let (_, other) = app.user("Otto Other", "US").await;
    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/game/repo-check").cookie(&other))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn private_slop_goes_public_after_a_fresh_review(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    let (admin_id, admin_cookie) = app.user("Ada Admin", "US").await;
    admin::set_admin(&app.db, &admin_id.to_string(), true, "test")
        .await
        .unwrap();
    let mut body = new_app("secret-tool");
    body["visibility"] = json!("private");
    body["repo_url"] = json!("https://github.com/octo/closed-tool");
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = 'secret-tool'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('c', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    app.send(
        req(Method::POST, "/v1/apps/secret-tool/shares")
            .cookie(&owner)
            .json(json!({ "user": friend_id.to_string() })),
    )
    .await;

    // The repository is still private: refused.
    let make_public =
        |cookie: &str| req(Method::POST, "/v1/apps/secret-tool/make-public").cookie(cookie);
    let res = app.send(make_public(&owner)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("public")
    );
    assert_eq!(
        app.send(make_public(&friend)).await.status(),
        StatusCode::NOT_FOUND
    );

    // Public now: it waits for a reviewer, still private meanwhile.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/open-tool' where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(make_public(&owner)).await.status(),
        StatusCode::NO_CONTENT
    );
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/secret-tool/manage").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(overview["public_requested"], true);
    assert!(
        list_slugs(&app, req(Method::GET, "/v1/apps"))
            .await
            .is_empty()
    );
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    let ready = queue["ready_to_list"].as_array().unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0]["going_public"], true);
    assert!(
        admin::list_app(&app.db, "secret-tool", "test")
            .await
            .is_err(),
        "only through the panel, which checks the repository again"
    );

    // The reviewer lists it: public, in the store, shares no longer needed.
    let res = app
        .send(req(Method::POST, "/v1/admin/apps/secret-tool/list").cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        list_slugs(&app, req(Method::GET, "/v1/apps")).await,
        ["secret-tool"]
    );
    let shares: i64 = sqlx::query_scalar("select count(*) from app_shares")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(shares, 0);
    let detail = body_json(app.send(req(Method::GET, "/v1/apps/secret-tool")).await).await;
    assert_eq!(detail["private"], false);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn frozen_private_slop_stops_being_shared_after_14_days(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    let mut body = new_app("secret-tool");
    body["visibility"] = json!("private");
    body["repo_url"] = json!("https://github.com/octo/deleted-tool");
    app.send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         select $1, id, '1.0.0', repeat('c', 40), 'approved', $2, now(), true from apps where slug = 'secret-tool'",
    )
    .bind(Uuid::now_v7())
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    app.send(
        req(Method::POST, "/v1/apps/secret-tool/shares")
            .cookie(&owner)
            .json(json!({ "user": friend_id.to_string() })),
    )
    .await;
    slop_api::frozen::run_once(&app.state).await.unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&friend))
            .await
            .status(),
        StatusCode::OK,
        "still shared during the grace period"
    );
    sqlx::query("update apps set frozen_at = now() - interval '15 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&friend))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/secret-tool").cookie(&owner))
            .await
            .status(),
        StatusCode::OK,
        "the owner still sees it"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn community_builds_of_pulse_games_need_a_pulse_handshake(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (author_id, _) = app.user("Bo Brancher", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    listed_with(
        &app,
        author_id,
        "castle-mod",
        "released",
        "status = 'draft', parent_app_id = (select id from apps where slug = 'castle'), branch_kind = 'mod'",
    )
    .await;
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         select $1, id, '1.0.0', repeat('b', 40), 'approved', $2, now(), true from apps where slug = 'castle-mod'",
    )
    .bind(Uuid::now_v7())
    .bind(author_id)
    .execute(&app.db)
    .await
    .unwrap();
    let session = |slug: &'static str, user: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query(
                "insert into pulse_sessions (id, token_hash, app_id, user_id, started_at)
                 select $1, $2, id, $3, now() from apps where slug = $4",
            )
            .bind(Uuid::now_v7())
            .bind(Uuid::now_v7().as_bytes().to_vec())
            .bind(user)
            .bind(slug)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    // The original doesn't use Pulse: no requirement.
    let (player, _) = app.user("Pat Player", "US").await;
    session("castle", player).await;
    let err = admin::list_app(&app.db, "castle-mod", "test")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Pulse"), "{err}");
    session("castle-mod", author_id).await;
    admin::list_app(&app.db, "castle-mod", "test")
        .await
        .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn changing_where_donations_go_takes_72_hours_even_from_an_old_session(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (donor_id, donor) = app.user("Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", true).await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/arena/donate")
                .cookie(&donor)
                .json(json!({ "amount_minor": 300, "monthly": true, "consent": true, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let old: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let change = |kind: &str| {
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": kind }))
    };

    // A sign-in from long ago is enough (no "confirm it's you" since 2026-10-25, SECURITY §4.3):
    // the 72 hours' wait and the announcement are what protect it.
    sqlx::query("update sessions set created_at = now() - interval '30 days' where user_id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();

    // A switch waits 72 hours, announced at once; another can't be stacked; it can be cancelled.
    let res = app.send(change("switch")).await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let account = body_json(
        app.send(req(Method::GET, "/v1/me/donation-account").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(account["change"]["kind"], "switch");
    let notices: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 and category = 'donations'",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert!(
        notices
            .iter()
            .any(|t| t.contains("different Stripe account"))
    );
    assert_eq!(
        app.send(change("disconnect")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .unwrap(),
        0,
        "not before 72 hours"
    );
    sqlx::query("update stripe_account_changes set effective_at = now()")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .unwrap(),
        1
    );
    let new: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_ne!(new, old);
    let monthly: String = sqlx::query_scalar("select status from server_donations")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(monthly, "active", "server donations carry on");

    // Disconnecting: cancelled once, then done after 72 hours; donations off, monthly ones end.
    assert_eq!(
        app.send(change("disconnect")).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/me/donation-account/change/cancel").cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(change("disconnect")).await.status(),
        StatusCode::OK
    );
    sqlx::query("update stripe_account_changes set effective_at = now() where applied_at is null and cancelled_at is null")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::donations::apply_account_changes(&app.state)
        .await
        .unwrap();
    let on: bool = sqlx::query_scalar("select donations_on from apps where slug = 'arena'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!on);
    let (status, by): (String, String) =
        sqlx::query_as("select status, cancelled_by from server_donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), by.as_str()),
        ("cancelled", "maker_disconnected")
    );
    let donor_told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%has ended%'",
    )
    .bind(donor_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(donor_told, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repeated_unfinished_checkouts_need_a_proof_of_work(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (donor_id, donor) = app.user("Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", false).await;
    for _ in 0..3 {
        sqlx::query(
            "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                    slop_fee_minor, processing_minor, total_minor, donor_covers_fees, status)
             select $1, 'one_off', id, $2, 'acct_x', 'usd', 300, 15, 25, 340, true, 'expired' from apps where slug = 'arena'",
        )
        .bind(Uuid::now_v7())
        .bind(donor_id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let donate = |extra: Value| {
        let mut body = json!({ "amount_minor": 300, "confirm_adult": true });
        if let Value::Object(map) = extra {
            body.as_object_mut().unwrap().extend(map);
        }
        req(Method::POST, "/v1/apps/arena/donate")
            .cookie(&donor)
            .json(body)
    };
    let res = app.send(donate(json!({}))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(res).await["error"]["code"], "challenge_required");

    let challenge = body_json(
        app.send(req(Method::POST, "/v1/donations/challenge").cookie(&donor))
            .await,
    )
    .await;
    let prefix = challenge["prefix"].as_str().unwrap();
    let difficulty = challenge["difficulty"].as_i64().unwrap() as i16;
    let answer = json!({ "challenge": { "id": challenge["id"], "nonce": "nope" } });
    assert_eq!(
        body_json(app.send(donate(answer)).await).await["error"]["code"],
        "challenge_required",
        "a wrong answer uses the challenge up"
    );
    let challenge = body_json(
        app.send(req(Method::POST, "/v1/donations/challenge").cookie(&donor))
            .await,
    )
    .await;
    let prefix2 = challenge["prefix"].as_str().unwrap();
    assert_ne!(prefix, prefix2);
    let nonce = slop_api::challenge::solve(prefix2, difficulty);
    let answer = json!({ "challenge": { "id": challenge["id"], "nonce": nonce } });
    let res = app.send(donate(answer.clone())).await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    assert_eq!(
        body_json(app.send(donate(answer)).await).await["error"]["code"],
        "challenge_required",
        "each challenge works once"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn fee_statements_show_the_vat_inside_slops_fee(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        // The fake GitHub server also plays VIES.
        c.payments.vies_url = Some(c.doh_url.join("vies").unwrap());
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "DE").await;
    let (_, donor) = app.user("Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", false).await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/arena/donate")
                .cookie(&donor)
                .json(json!({ "amount_minor": 2000, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let statements = |cookie: String| {
        let app = &app;
        async move {
            body_json(
                app.send(req(Method::GET, "/v1/me/fee-statements").cookie(cookie))
                    .await,
            )
            .await
        }
    };
    // An individual in Germany: 19% VAT inside Slop's 5% (100 cents fee → 16 cents VAT).
    let s = statements(maker.clone()).await;
    assert_eq!(s["vat"]["kind"], "included");
    assert_eq!(s["vat"]["rate_bp"], 1900);
    assert_eq!(s["statements"][0]["slop_fee_minor"], 100);
    assert_eq!(s["statements"][0]["vat_minor"], 16);

    // A business with a VAT number VIES confirms: reverse charge.
    let vat = |n: &str| {
        req(Method::PUT, "/v1/me/vat")
            .cookie(&maker)
            .json(json!({ "vat_number": n }))
    };
    let res = app.send(vat("FR12345678901")).await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "another country's number"
    );
    let saved = body_json(app.send(vat("DE 999 999 999")).await).await;
    assert_eq!(saved["checked"], false);
    assert!(saved["note"].as_str().unwrap().contains("VIES"));
    let saved = body_json(app.send(vat("de123456789")).await).await;
    assert_eq!(saved["vat_number"], "DE123456789");
    assert_eq!(saved["checked"], true);
    let s = statements(maker.clone()).await;
    assert_eq!(s["vat"]["kind"], "reverse_charge");
    assert_eq!(s["statements"][0]["vat_minor"], 0);
    assert_eq!(slop_api::donations::vat_included(100, 2000), 17);
}

// ---------------------------------------------------------------------------------------------
// Chat (DESIGN §19, SECURITY §8.5): end to end with the real client core and real encryption

use slop_client::chat::{Chat, MemorySecrets, OutgoingFile};

/// Serves the API on a local port, for clients that make real HTTP requests.
async fn serve(app: &TestApp) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}/")
}

/// A user's Slop app with chat: signed in through the desktop flow, its own secure store and
/// data folder.
async fn chat_app(app: &TestApp, base: &str, user: Uuid, cookie: &str) -> Chat {
    let tokens = desktop_sign_in(app, cookie).await;
    let store = slop_client::api::MemoryTokenStore::default();
    use slop_client::api::TokenStore;
    store.save(tokens["refresh_token"].as_str().unwrap());
    let api = slop_client::api::ApiClient::new(base.parse().unwrap(), std::sync::Arc::new(store))
        .unwrap();
    let dir = std::env::temp_dir().join(format!("slop-chat-test-{}", Uuid::now_v7()));
    Chat::open(api, dir, &MemorySecrets::default(), user).unwrap()
}

async fn befriend(app: &TestApp, a: Uuid, b: Uuid) {
    sqlx::query("insert into friends (user_a, user_b) values (least($1, $2), greatest($1, $2))")
        .bind(a)
        .bind(b)
        .execute(&app.db)
        .await
        .unwrap();
}

fn chat_limits(c: &mut Config) {
    c.rate_limits.write_per_minute = 10_000;
    c.rate_limits.read_per_minute = 10_000;
    c.rate_limits.auth_per_minute = 1_000;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn chat_is_end_to_end_encrypted_and_slop_only_relays_it(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let base = serve(&app).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (_, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;

    // Chat is in the app only: the website can't register a device.
    let res = app
        .send(req(Method::POST, "/v1/chat/devices").cookie(&ann_web).json(json!({
            "id": Uuid::now_v7(), "name": "Browser", "signature_key": "AAAA", "key_packages": [], "last_resort": "AAAA"
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);

    let ann = chat_app(&app, &base, ann_id, &ann_web).await;
    let bob = chat_app(&app, &base, bob_id, &bob_web).await;
    assert_eq!(
        ann.setup("Ann's PC").await.unwrap().state,
        "active",
        "a first device needs no approval"
    );
    assert_eq!(bob.setup("Bob's PC").await.unwrap().state, "active");

    // A direct chat: Ann's app sets up the encryption with Bob's device; Bob's joins from the Welcome.
    let conv = ann.start_direct(&bob_id.to_string()).await.unwrap();
    bob.sync(0).await.unwrap();
    let list = bob.conversations().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Ann");
    assert!(list[0].ready);

    // Typing: a few seconds of "typing…", ended by the message.
    ann.typing(conv, true).await.unwrap();
    let changes = bob.sync(0).await.unwrap();
    assert_eq!(changes.typing.len(), 1);
    assert_eq!(
        (
            changes.typing[0].conversation,
            changes.typing[0].name.as_str()
        ),
        (conv, "Ann")
    );

    // Messages: Slop stores only ciphertext.
    ann.send(
        conv,
        "hello bob, pay me on paypal for the sprites",
        None,
        vec![],
        false,
    )
    .await
    .unwrap();
    let changes = bob.sync(0).await.unwrap();
    assert_eq!(changes.conversations, [conv]);
    assert!(changes.typing.is_empty(), "sending ends typing");
    assert_eq!(changes.new_messages.len(), 1, "for a notification");
    assert_eq!(
        (
            changes.new_messages[0].sender.as_str(),
            changes.new_messages[0].title.as_str()
        ),
        ("Ann", "Ann")
    );
    let messages = bob.messages(conv, None, 50).await.unwrap();
    let hello = messages.iter().find(|m| !m.notice).unwrap();
    assert_eq!(hello.text, "hello bob, pay me on paypal for the sprites");
    assert_eq!(hello.sender.name, "Ann");
    assert!(!hello.warnings.is_empty(), "off-platform payment warning");
    assert!(hello.reportable);
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("select body from chat_messages")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert!(!stored.is_empty());
    for body in &stored {
        assert!(
            !body.windows(5).any(|w| w == b"hello" || w == b"paypa"),
            "Slop can't read messages"
        );
    }
    let unread = &bob.conversations().await.unwrap()[0];
    assert_eq!(unread.unread, 1);
    bob.mark_read(conv).await.unwrap();
    assert_eq!(bob.conversations().await.unwrap()[0].unread, 0);

    // Reactions, a meme attachment, replies, edits and deletes.
    bob.react(conv, hello.id, "😂", true).await.unwrap();
    let gif = b"GIF89a pretend this is a meme".to_vec();
    ann.send(
        conv,
        "",
        Some(hello.id),
        vec![OutgoingFile {
            name: "meme.gif".into(),
            mime: "image/gif".into(),
            bytes: gif.clone(),
        }],
        false,
    )
    .await
    .unwrap();
    ann.sync(0).await.unwrap();
    let mine = ann.messages(conv, None, 50).await.unwrap();
    assert_eq!(mine[0].reactions[0].emoji, "😂");
    assert_eq!(mine[0].reactions[0].names, ["Bob"]);
    bob.sync(0).await.unwrap();
    let messages = bob.messages(conv, None, 50).await.unwrap();
    let meme = messages.last().unwrap();
    assert_eq!(meme.reply_to.as_ref().unwrap().id, hello.id);
    assert_eq!(meme.attachments[0].mime, "image/gif");
    let (mime, bytes) = bob
        .attachment(conv, meme.id, meme.attachments[0].id)
        .await
        .unwrap();
    assert_eq!((mime.as_str(), bytes), ("image/gif", gif.clone()));
    let at_slop: Vec<u8> = sqlx::query_scalar("select body from chat_attachments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(
        !at_slop.windows(6).any(|w| w == b"GIF89a"),
        "attachments are encrypted too"
    );
    assert!(bob.edit(conv, hello.id, "not mine").await.is_err());
    ann.edit(conv, hello.id, "hello bob!").await.unwrap();
    ann.delete(conv, meme.id).await.unwrap();
    bob.sync(0).await.unwrap();
    let messages = bob.messages(conv, None, 50).await.unwrap();
    let hello2 = messages.iter().find(|m| m.id == hello.id).unwrap();
    assert_eq!((hello2.text.as_str(), hello2.edited), ("hello bob!", true));
    assert!(messages.iter().find(|m| m.id == meme.id).unwrap().deleted);

    // A second device for Bob: approved on the first after comparing codes, then it gets the
    // chats and their history.
    let bob2 = chat_app(&app, &base, bob_id, &bob_web).await;
    let status = bob2.setup("Bob's laptop").await.unwrap();
    assert_eq!(status.state, "pending");
    let code = status.code.unwrap();
    let pending = bob.status().await.unwrap().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].code, code);
    assert!(
        bob.approve(pending[0].id, "AAA-AAA-AAA").await.is_err(),
        "codes must match"
    );
    bob.approve(pending[0].id, &code).await.unwrap();
    assert_eq!(bob2.status().await.unwrap().state, "active");
    for _ in 0..3 {
        bob.sync(0).await.unwrap();
        ann.sync(0).await.unwrap();
        bob2.sync(0).await.unwrap();
    }
    let on_laptop = bob2.messages(conv, None, 50).await.unwrap();
    assert!(
        on_laptop.iter().any(|m| m.text == "hello bob!"),
        "history from the approving device: {on_laptop:?}"
    );
    ann.send(conv, "hi both of you", None, vec![], false)
        .await
        .unwrap();
    bob2.sync(0).await.unwrap();
    bob.sync(0).await.unwrap();
    assert!(
        bob2.messages(conv, None, 50)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "hi both of you")
    );
    assert!(
        bob.messages(conv, None, 50)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "hi both of you")
    );
    let codes = ann.security_codes(conv).await.unwrap();
    assert_eq!(codes.len(), 2);
    let bob_codes = bob2.security_codes(conv).await.unwrap();
    assert_eq!(
        codes.iter().find(|c| c.user == bob_id).unwrap().code,
        bob_codes.iter().find(|c| c.user == bob_id).unwrap().code,
        "everyone sees the same security code"
    );

    // Reporting: Bob's app sends the decrypted message; Slop checks it against the franking tag.
    let hello_id = hello.id;
    let ack = bob
        .report(
            conv,
            hello_id,
            "harassment",
            "Asked me to pay outside Slop.",
        )
        .await
        .unwrap();
    assert!(ack["reference"].is_string());
    let (kind, quoted, about): (String, String, Uuid) =
        sqlx::query_as("select subject_kind, quoted, subject_user_id from reports")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), about), ("chat_message", ann_id));
    assert!(
        quoted.contains("paypal"),
        "the original text, as sent: {quoted}"
    );
    let seq: i64 = sqlx::query_scalar(
        "select min(seq) from chat_messages where kind = 'application' and sender_user_id = $1",
    )
    .bind(ann_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/reports").cookie(&bob_web).json(json!({
            "kind": "chat_message", "subject": conv.to_string(), "reason": "spam", "details": "x", "good_faith": true,
            "chat": { "conversation_id": conv, "seq": seq, "plaintext": "{\"text\":\"made up\"}", "franking_key": "AAAA" },
        })))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a made-up message fails the check"
    );

    // Strangers can't start a chat; blocking stops one.
    let res = app
        .send(
            req(Method::POST, "/v1/chat/conversations")
                .cookie(&cara_web)
                .json(json!({ "kind": "direct", "user": ann_id })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "website sessions can't chat at all"
    );
    let cara_id: Uuid = sqlx::query_scalar("select id from users where display_name = 'Cara'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let cara = chat_app(&app, &base, cara_id, &cara_web).await;
    cara.setup("Cara's PC").await.unwrap();
    assert!(
        cara.start_direct(&ann_id.to_string()).await.is_err(),
        "not connected"
    );
    assert_eq!(
        app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&bob_web))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        ann.send(conv, "still there?", None, vec![], false)
            .await
            .is_err()
    );
    assert!(
        bob.conversations().await.unwrap().is_empty(),
        "the blocked person's chat leaves the list"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn group_chats_add_and_remove_people_and_their_devices(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let base = serve(&app).await;
    let mut people = Vec::new();
    for name in ["Ann", "Bob", "Cara"] {
        let (id, web) = app.user(name, "US").await;
        people.push((id, web));
    }
    befriend(&app, people[0].0, people[1].0).await;
    befriend(&app, people[0].0, people[2].0).await;
    let mut apps = Vec::new();
    for (id, web) in &people {
        let chat = chat_app(&app, &base, *id, web).await;
        chat.setup("PC").await.unwrap();
        apps.push(chat);
    }
    let (ann, bob, cara) = (&apps[0], &apps[1], &apps[2]);
    let users: Vec<String> = people[1..].iter().map(|(id, _)| id.to_string()).collect();
    let conv = ann
        .start_group(&users, Some("Roguelike nights"))
        .await
        .unwrap();
    let sync_all = || async {
        for _ in 0..2 {
            for a in &apps {
                a.sync(0).await.unwrap();
            }
        }
    };
    sync_all().await;
    for a in [bob, cara] {
        let list = a.conversations().await.unwrap();
        assert_eq!(
            list[0].title, "Roguelike nights",
            "the name travels inside the encryption"
        );
    }
    cara.send(conv, "gg everyone", None, vec![], false)
        .await
        .unwrap();
    sync_all().await;
    assert!(
        ann.messages(conv, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "gg everyone")
    );
    assert!(
        bob.messages(conv, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "gg everyone")
    );

    // Only the person who started it removes people; their devices leave the encryption.
    let res = app
        .send(
            req(
                Method::DELETE,
                format!("/v1/chat/conversations/{conv}/members/{}", people[2].0),
            )
            .cookie(&people[1].1),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    ann.remove_person(conv, people[2].0).await.unwrap();
    sync_all().await;
    let epoch_before: i64 =
        sqlx::query_scalar("select epoch from chat_conversations where id = $1")
            .bind(conv)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(epoch_before >= 2);
    ann.send(conv, "cara can't read this", None, vec![], false)
        .await
        .unwrap();
    sync_all().await;
    assert!(
        bob.messages(conv, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "cara can't read this")
    );
    assert!(
        !cara
            .messages(conv, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "cara can't read this")
    );
    assert!(
        cara.conversations()
            .await
            .unwrap()
            .iter()
            .all(|c| c.id != conv)
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn friends_and_who_may_message(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    let (teen_id, teen) = app.user("Tia", "GB").await;

    // A request, then accepting it.
    let res = app
        .send(
            req(Method::POST, "/v1/me/friends")
                .cookie(&ann)
                .json(json!({ "user": format!("http://localhost:8080/u/{bob_id}") })),
        )
        .await;
    assert_eq!(body_json(res).await["status"], "requested");
    let conn = body_json(
        app.send(req(Method::GET, format!("/v1/users/{ann_id}/connection")).cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(
        (conn["friend"].as_str(), conn["can_message"].as_bool()),
        (Some("incoming"), Some(false))
    );
    let notified: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and category = 'friends'",
    )
    .bind(bob_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(notified, 1);
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/me/friends/{ann_id}/accept")).cookie(&bob))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let friends = body_json(
        app.send(req(Method::GET, "/v1/me/friends").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(friends["friends"][0]["name"], "Bob");
    let conn = body_json(
        app.send(req(Method::GET, format!("/v1/users/{bob_id}/connection")).cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(
        (conn["friend"].as_str(), conn["can_message"].as_bool()),
        (Some("friends"), Some(true))
    );

    // "Messages from anyone": strangers may message an adult who allows it, never a UK teen.
    let conn = body_json(
        app.send(req(Method::GET, format!("/v1/users/{ann_id}/connection")).cookie(&teen))
            .await,
    )
    .await;
    assert_eq!(conn["can_message"], false);
    let res = app
        .send(
            req(Method::PUT, "/v1/me/chat-settings")
                .cookie(&ann)
                .json(json!({ "from_anyone": true })),
        )
        .await;
    assert_eq!(body_json(res).await["from_anyone"], true);
    let conn = body_json(
        app.send(req(Method::GET, format!("/v1/users/{ann_id}/connection")).cookie(&teen))
            .await,
    )
    .await;
    assert_eq!(
        conn["can_message"], false,
        "an under-18 view never messages strangers"
    );
    let settings = body_json(
        app.send(req(Method::GET, "/v1/me/chat-settings").cookie(&teen))
            .await,
    )
    .await;
    assert_eq!(settings["locked"], true);
    assert_eq!(
        app.send(
            req(Method::PUT, "/v1/me/chat-settings")
                .cookie(&teen)
                .json(json!({ "from_anyone": true }))
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let conn = body_json(
        app.send(req(Method::GET, format!("/v1/users/{teen_id}/connection")).cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(conn["can_message"], false);

    // Blocking ends the friendship and hides requests; blocked looks like nobody.
    assert_eq!(
        app.send(req(Method::PUT, format!("/v1/me/blocks/{bob_id}")).cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let friends = body_json(
        app.send(req(Method::GET, "/v1/me/friends").cookie(&ann))
            .await,
    )
    .await;
    assert!(friends["friends"].as_array().unwrap().is_empty());
    let res = app
        .send(
            req(Method::POST, "/v1/me/friends")
                .cookie(&bob)
                .json(json!({ "user": ann_id.to_string() })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let blocks = body_json(
        app.send(req(Method::GET, "/v1/me/blocks").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(blocks[0]["name"], "Bob");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn chat_commits_must_build_on_the_current_epoch(db: PgPool) {
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let app = test_app_with(db, chat_limits).await;
    let (_, web) = app.user("Ann", "US").await;
    let (_, other_web) = app.user("Bob", "US").await;
    let tokens = desktop_sign_in(&app, &web).await;
    let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    let device = Uuid::now_v7();
    let res = app
        .send(req(Method::POST, "/v1/chat/devices").header("authorization", bearer.clone()).json(json!({
            "id": device, "name": "PC", "signature_key": b64(&[1; 32]), "key_packages": [b64(b"kp")], "last_resort": b64(b"lr")
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    assert_eq!(created["status"], "active");
    assert_eq!(created["code"].as_str().unwrap().len(), 11);
    let res = app
        .send(
            req(Method::POST, "/v1/chat/conversations")
                .header("authorization", bearer.clone())
                .json(json!({ "kind": "self" })),
        )
        .await;
    let conv = body_json(res).await["id"].as_str().unwrap().to_string();
    let call = |path: String, body: Value| {
        req(Method::POST, path)
            .header("authorization", bearer.clone())
            .header("x-slop-chat-device", device.to_string())
            .json(body)
    };
    let commit = |epoch: i64| {
        call(
            format!("/v1/chat/conversations/{conv}/commit"),
            json!({ "epoch": epoch, "commit": b64(b"commit") }),
        )
    };
    let message = |epoch: i64| {
        call(
            format!("/v1/chat/conversations/{conv}/messages"),
            json!({ "epoch": epoch, "body": b64(b"ciphertext"), "franking_tag": b64(&[0; 32]) }),
        )
    };
    assert_eq!(app.send(commit(0)).await.status(), StatusCode::OK);
    let res = app.send(commit(0)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["epoch"].as_i64()
        ),
        (Some("stale_epoch"), Some(1))
    );
    assert_eq!(app.send(message(0)).await.status(), StatusCode::CONFLICT);
    assert_eq!(app.send(message(1)).await.status(), StatusCode::OK);
    let synced = body_json(
        app.send(
            req(Method::GET, "/v1/chat/sync?after=0")
                .header("authorization", bearer.clone())
                .header("x-slop-chat-device", device.to_string()),
        )
        .await,
    )
    .await;
    let kinds: Vec<&str> = synced["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["commit", "application"]);
    assert_eq!(synced["key_packages"], 1);

    // Someone else's session can't use this device, or see the chat.
    let other = desktop_sign_in(&app, &other_web).await;
    let res = app
        .send(
            req(Method::GET, "/v1/chat/sync?after=0")
                .header(
                    "authorization",
                    format!("Bearer {}", other["access_token"].as_str().unwrap()),
                )
                .header("x-slop-chat-device", device.to_string()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(req(Method::GET, format!("/v1/chat/conversations/{conv}")).cookie(&other_web))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

/// Authorization headers the fake push service received.
static PUSHES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn phones_get_content_free_pushes(db: PgPool) {
    let (key, _) = slop_api::push::generate();
    let app = test_app_with(db, |c| {
        c.push = Some(slop_api::config::PushConfig {
            key: slop_api::push::parse_key(&key).unwrap(),
            subject: "mailto:ops@slop.example".into(),
            extra_hosts: vec!["127.0.0.1".into()],
            allow_http: true,
        });
    })
    .await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, bob) = app.user("Bob", "US").await;
    let public = body_json(app.send(req(Method::GET, "/v1/push/key")).await).await;
    assert!(public["public_key"].as_str().unwrap().len() > 80);
    let res = app
        .send(
            req(Method::POST, "/v1/me/push")
                .cookie(&ann)
                .json(json!({ "endpoint": "https://evil.example/x" })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "only push services"
    );
    for path in ["push/ok", "push/gone"] {
        let res = app
            .send(req(Method::POST, "/v1/me/push").cookie(&ann).json(json!({
                "endpoint": format!("{}{path}", app.fake_base), "keys": { "p256dh": "x", "auth": "y" }
            })))
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    // Bob's friend request makes a notification for Ann: one push, with no content.
    app.send(
        req(Method::POST, "/v1/me/friends")
            .cookie(&bob)
            .json(json!({ "user": ann_id.to_string() })),
    )
    .await;
    let before = PUSHES.lock().unwrap().len();
    assert_eq!(slop_api::push::run_once(&app.state).await.unwrap(), 1);
    assert_eq!(
        slop_api::push::run_once(&app.state).await.unwrap(),
        0,
        "pushed once"
    );
    let (count, last) = {
        let pushes = PUSHES.lock().unwrap();
        (pushes.len(), pushes.last().cloned().unwrap_or_default())
    };
    assert_eq!(count, before + 1);
    assert!(last.starts_with("vapid t="));
    let left: Vec<String> = sqlx::query_scalar("select endpoint from push_subscriptions")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(left.len(), 1, "the gone subscription is dropped: {left:?}");
    assert!(left[0].ends_with("push/ok"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_a_repositorys_owner_can_link_it(db: PgPool) {
    let app = test_app_with(db, |config| {
        config.releases.require_repo_owner = true;
        config.releases.github_app = Some(slop_api::config::GithubAppConfig {
            app_id: "123".into(),
            slug: "slop-test".into(),
            private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                "../fixtures/github-app-test-key.pem"
            ))
            .unwrap(),
            webhook_secret: "hook-secret".into(),
        });
    })
    .await;
    let github = |user: Uuid, id: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query("insert into identities (provider, provider_subject, user_id) values ('github', $1, $2)")
                .bind(id)
                .bind(user)
                .execute(&db)
                .await
                .unwrap();
        }
    };
    let (octo_id, octo) = app.user("Octo", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    let (_, nobody) = app.user("No GitHub", "US").await;
    github(octo_id, "42").await;
    github(bob_id, "43").await;
    let publish = |cookie: &str, slug: &str, repo: &str| {
        let mut body = new_app(slug);
        body["repo_url"] = json!(repo);
        req(Method::POST, "/v1/apps").cookie(cookie).json(body)
    };
    let refused = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // A collaborator (or anyone else) can't link octo's repository; octo can.
    let why = refused(
        app.send(publish(
            &bob,
            "pixel",
            "https://github.com/octo/pixel-dungeon",
        ))
        .await,
    )
    .await;
    assert!(why.contains("octo owns this one"), "{why}");
    let why = refused(
        app.send(publish(
            &nobody,
            "pixel",
            "https://github.com/octo/pixel-dungeon",
        ))
        .await,
    )
    .await;
    assert!(
        why.contains("link the GitHub account that owns this repository"),
        "{why}"
    );
    let res = app
        .send(publish(
            &octo,
            "pixel",
            "https://github.com/octo/pixel-dungeon",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);

    // One app per repository, however the address is written.
    let res = app
        .send(publish(
            &octo,
            "pixel-2",
            "https://github.com/Octo/Pixel-Dungeon.git",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already linked to an app on Slop")
    );

    // Organizations: only their owners, through Slop's App on the organization.
    let res = app
        .send(publish(
            &octo,
            "acme-game",
            "https://github.com/acme/studio-game",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED, "octo owns acme");
    let why = refused(
        app.send(publish(
            &bob,
            "acme-2",
            "https://github.com/acme/other-game",
        ))
        .await,
    )
    .await;
    assert!(
        why.contains("only owners of the acme organization"),
        "{why}"
    );
    let why = refused(
        app.send(publish(&octo, "globex", "https://github.com/globex/game"))
            .await,
    )
    .await;
    assert!(why.contains("install Slop's GitHub App"), "{why}");

    // A repository Slop can't see isn't accepted.
    let why = refused(
        app.send(publish(
            &octo,
            "gone",
            "https://github.com/octo/deleted-game",
        ))
        .await,
    )
    .await;
    assert!(why.contains("can't see that repository"), "{why}");

    // Moving an app is checked the same way.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/pixel")
                .cookie(&octo)
                .json(json!({ "repo_url": "https://github.com/bob/pixel-dungeon" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Branches come from the author's own fork.
    sqlx::query("update apps set status = 'listed' where slug = 'pixel'")
        .execute(&app.db)
        .await
        .unwrap();
    let branch = |cookie: &str, repo: &str| {
        req(Method::POST, "/v1/apps/pixel/branches").cookie(cookie).json(json!({
            "slug": "pixel-linux", "title": "Pixel (Linux port)", "summary": "Native Linux.",
            "repo_url": repo, "license_spdx": "MIT", "kind": "port", "note": "Adds Linux.", "platforms": ["linux", "linux-deb"]
        }))
    };
    let why = refused(
        app.send(branch(&octo, "https://github.com/bob/pixel-dungeon"))
            .await,
    )
    .await;
    assert!(why.contains("bob owns"), "{why}");
    assert_eq!(
        app.send(branch(&bob, "https://github.com/bob/pixel-dungeon"))
            .await
            .status(),
        StatusCode::CREATED
    );
}

fn form_code(body: &str) -> String {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// The `.sig` the fake release page serves next to every file (maker keys), when not empty.
static MAKER_SIG: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// An Ed25519 key's OpenSSH public key line, and an `ssh-keygen -Y sign -n slop-release`
/// signature over `file` (PROTOCOL.sshsig).
fn ssh_sign(key: &slop_manifest::ed25519_dalek::SigningKey, file: &[u8]) -> (String, String) {
    use base64::Engine;
    use sha2::Digest;
    use slop_manifest::ed25519_dalek::Signer;
    let b64 = base64::engine::general_purpose::STANDARD;
    let string = |out: &mut Vec<u8>, s: &[u8]| {
        out.extend_from_slice(&(s.len() as u32).to_be_bytes());
        out.extend_from_slice(s);
    };
    let mut pub_blob = Vec::new();
    string(&mut pub_blob, b"ssh-ed25519");
    string(&mut pub_blob, &key.verifying_key().to_bytes());
    let mut signed = b"SSHSIG".to_vec();
    string(&mut signed, b"slop-release");
    string(&mut signed, b"");
    string(&mut signed, b"sha512");
    string(&mut signed, &sha2::Sha512::digest(file));
    let mut sig_blob = Vec::new();
    string(&mut sig_blob, b"ssh-ed25519");
    string(&mut sig_blob, &key.sign(&signed).to_bytes());
    let mut blob = b"SSHSIG".to_vec();
    blob.extend_from_slice(&1u32.to_be_bytes());
    string(&mut blob, &pub_blob);
    string(&mut blob, b"slop-release");
    string(&mut blob, b"");
    string(&mut blob, b"sha512");
    string(&mut blob, &sig_blob);
    (
        format!("ssh-ed25519 {} maker@laptop", b64.encode(&pub_blob)),
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            b64.encode(&blob)
        ),
    )
}

/// What the fake Gitea repository's README says (the ownership code, once the maker adds it).
static PROOF: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn gitea_auth(headers: &axum::http::HeaderMap, ok: impl IntoResponse) -> axum::response::Response {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("token good") {
        ok.into_response()
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn bearer(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string()
}

/// Google and GitLab sign-in against the fake provider.
fn with_providers(c: &mut Config) {
    let base = c.doh_url.clone();
    let oauth = |name: &str| slop_api::config::OAuthConfig {
        client_id: format!("{name}-client"),
        client_secret: "secret".into(),
        authorize_url: base.join(&format!("{name}/auth")).unwrap(),
        token_url: base.join(&format!("{name}/token")).unwrap(),
        api_base: base
            .join(if name == "gitlab" {
                "gitlab/api/"
            } else {
                "google/"
            })
            .unwrap(),
    };
    c.google = Some(oauth("google"));
    c.gitlab = Some(oauth("gitlab"));
}

/// Runs a provider flow from `start` (a /start or /link path) with `cookies`, through the
/// callback with `code`. Returns the callback's response.
async fn oauth_flow(app: &TestApp, start: &str, cookies: &[&str], code: &str) -> Response {
    let mut r = req(Method::GET, start);
    for c in cookies {
        r = r.cookie(*c);
    }
    let res = app.send(r).await;
    assert!(
        res.status().is_redirection(),
        "{}: {:?}",
        start,
        res.status()
    );
    let state = query_param(&location(&res), "state");
    let state_cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let provider = start.split('/').nth(3).unwrap();
    let mut r = req(
        Method::GET,
        format!("/v1/auth/{provider}/callback?code={code}&state={state}"),
    )
    .cookie(state_cookie);
    for c in cookies {
        r = r.cookie(*c);
    }
    app.send(r).await
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sign_ins_link_only_after_signing_in_to_them(db: PgPool) {
    let app = test_app_with(db, with_providers).await;
    let providers = body_json(app.send(req(Method::GET, "/v1/auth/providers")).await).await;
    assert_eq!(
        providers,
        json!({ "github": true, "google": true, "gitlab": true, "bitbucket": false, "microsoft": false, "discord": false })
    );

    // Signing up with Google.
    let res = oauth_flow(
        &app,
        "/v1/auth/google/start?return_to=/library",
        &[],
        "g-ann",
    )
    .await;
    assert_eq!(location(&res), "/signup");
    let signup = set_cookie(&res, "slop_signup").unwrap();
    let res = app
        .send(req(Method::POST, "/v1/auth/signup").cookie(&signup).json(json!({
            "display_name": "Ann", "country_code": "US", "age_confirmed": true, "accept_terms": true
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let ann = set_cookie(&res, "slop_session").unwrap();
    let ann_id: Uuid =
        sqlx::query_scalar("select user_id from identities where provider = 'google'")
            .fetch_one(&app.db)
            .await
            .unwrap();

    // Linking GitLab: from the signed-in session, by signing in to GitLab.
    let res = oauth_flow(&app, "/v1/auth/gitlab/link", &[&ann], "gl-ann").await;
    assert_eq!(location(&res), "/account?linked=gitlab#sign-ins");
    let linked = body_json(
        app.send(req(Method::GET, "/v1/me/identities").cookie(&ann))
            .await,
    )
    .await;
    let names: Vec<&str> = linked
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["provider"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["google", "gitlab"]);
    assert_eq!(linked[1]["login"], "ann");
    // Signing in with the linked GitLab account opens the same Slop account.
    let res = oauth_flow(&app, "/v1/auth/gitlab/start", &[], "gl-ann").await;
    let other = set_cookie(&res, "slop_session").unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&other)).await).await;
    assert_eq!(me["id"], json!(ann_id));

    // Not without a session: sign in first, then straight back to linking. An old sign-in goes
    // straight on to the provider (no "confirm it's you" since 2026-10-25, SECURITY §4.3).
    let res = app.send(req(Method::GET, "/v1/auth/gitlab/link")).await;
    assert!(
        location(&res).starts_with("/signin?reason=link&return_to=%2Fv1%2Fauth%2Fgitlab%2Flink")
    );
    sqlx::query("update sessions set created_at = now() - interval '30 days' where user_id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();

    // A link started in one session can't be finished in another.
    let res = app
        .send(req(Method::GET, "/v1/auth/github/link").cookie(&ann))
        .await;
    let state = query_param(&location(&res), "state");
    let state_cookie = set_cookie(&res, "slop_oauth_state").unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/auth/github/callback?code=ok&state={state}"),
            )
            .cookie(state_cookie)
            .cookie(&other),
        )
        .await;
    assert_eq!(location(&res), "/signin?error=link_expired");

    // Unlinking: not the last way in.
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/identities/gitlab/501").cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let res = app
        .send(req(Method::DELETE, "/v1/me/identities/google/g-ann").cookie(&ann))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn linking_a_sign_in_with_its_own_account_combines_them(db: PgPool) {
    let app = test_app_with(db, with_providers).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    let (cara_id, _) = app.user("Cara", "US").await;
    for (user, provider, subject) in [(ann_id, "google", "g-ann"), (bob_id, "gitlab", "502")] {
        sqlx::query(
            "insert into identities (provider, provider_subject, user_id) values ($1, $2, $3)",
        )
        .bind(provider)
        .bind(subject)
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Bob's account: an app, a friend, a vote; both voted on Ann's app.
    listed_with(&app, bob_id, "bobs-game", "released", "").await;
    listed_with(&app, ann_id, "anns-game", "released", "").await;
    befriend(&app, bob_id, cara_id).await;
    befriend(&app, bob_id, ann_id).await;
    for (user, vote) in [(ann_id, 1i16), (bob_id, -1)] {
        sqlx::query("insert into votes (user_id, app_id, vote, weight) select $1, id, $2, 1 from apps where slug = 'bobs-game'")
            .bind(user)
            .bind(vote)
            .execute(&app.db)
            .await
            .unwrap();
    }

    // Linking Bob's GitLab: it has its own account, so Ann is asked first.
    let res = oauth_flow(&app, "/v1/auth/gitlab/link", &[&ann], "gl-bob").await;
    assert_eq!(location(&res), "/account/combine");
    let combine = set_cookie(&res, "slop_combine").unwrap();
    let preview = body_json(
        app.send(
            req(Method::GET, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine),
        )
        .await,
    )
    .await;
    assert_eq!(
        (
            preview["name"].as_str(),
            preview["apps"].as_i64(),
            preview["friends"].as_i64()
        ),
        (Some("Bob"), Some(1), Some(2))
    );
    assert_eq!(preview["blocked"], Value::Null);
    // Someone else's session can't confirm it.
    let (_, eve) = app.user("Eve", "US").await;
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&eve)
                .cookie(&combine)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&ann)
                .cookie(&combine)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );

    // Everything is Ann's now; Ann's own vote stays; Bob's account is closed into Ann's.
    let maker: Uuid = sqlx::query_scalar("select maker_id from apps where slug = 'bobs-game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(maker, ann_id);
    let votes: Vec<(Uuid, i16)> = sqlx::query_as("select user_id, vote from votes")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(votes, [(ann_id, 1)]);
    let friends: Vec<(Uuid, Uuid)> = sqlx::query_as("select user_a, user_b from friends")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(
        friends,
        [(ann_id.min(cara_id), ann_id.max(cara_id))],
        "no friendship with herself"
    );
    let (closed, into): (bool, Option<Uuid>) =
        sqlx::query_as("select closed_at is not null, merged_into from users where id = $1")
            .bind(bob_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(closed);
    assert_eq!(into, Some(ann_id));
    let res = oauth_flow(&app, "/v1/auth/gitlab/start", &[], "gl-bob").await;
    let session = set_cookie(&res, "slop_session").unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&session)).await).await;
    assert_eq!(
        me["id"],
        json!(ann_id),
        "Bob's sign-in opens the combined account"
    );

    // A suspended account is never combined.
    let (_, dee) = app.user("Dee", "US").await;
    let (zed_id, _) = app.user("Zed", "US").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('gitlab', '599', $1)",
    )
    .bind(zed_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(zed_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = oauth_flow(&app, "/v1/auth/gitlab/link", &[&dee], "gl-zed").await;
    let combine = set_cookie(&res, "slop_combine").unwrap();
    let preview = body_json(
        app.send(
            req(Method::GET, "/v1/auth/combine")
                .cookie(&dee)
                .cookie(&combine),
        )
        .await,
    )
    .await;
    assert!(preview["blocked"].as_str().unwrap().contains("suspended"));
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&dee)
                .cookie(&combine)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn other_hosts_prove_ownership_by_account_or_readme_code(db: PgPool) {
    let app = test_app_with(db, |c| {
        with_providers(c);
        c.releases.require_repo_owner = true;
    })
    .await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, bob) = app.user("Bob", "US").await;
    sqlx::query("insert into identities (provider, provider_subject, user_id, login) values ('gitlab', '501', $1, 'ann')")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    let publish = |cookie: &str, slug: &str, repo: &str| {
        let mut body = new_app(slug);
        body["repo_url"] = json!(repo);
        req(Method::POST, "/v1/apps").cookie(cookie).json(body)
    };
    let ownership = |cookie: &str, repo: &str, check: bool| {
        req(
            Method::POST,
            if check {
                "/v1/repos/ownership/check"
            } else {
                "/v1/repos/ownership"
            },
        )
        .cookie(cookie)
        .json(json!({ "repo_url": repo }))
    };

    // GitLab (the fake one): ann's own project through her linked account; a group's project
    // through being its Owner.
    let gitlab = app.fake_base.trim_end_matches('/');
    let a = body_json(
        app.send(ownership(&ann, &format!("{gitlab}/ann/game"), false))
            .await,
    )
    .await;
    assert_eq!(
        (a["ok"].as_bool(), a["method"].as_str()),
        (Some(true), Some("account"))
    );
    let a = body_json(
        app.send(ownership(&bob, &format!("{gitlab}/ann/game"), false))
            .await,
    )
    .await;
    assert_eq!(
        (a["ok"].as_bool(), a["link"].as_str()),
        (Some(false), Some("gitlab"))
    );
    assert_eq!(
        app.send(publish(&ann, "studio", &format!("{gitlab}/studio/game")))
            .await
            .status(),
        StatusCode::CREATED
    );

    // Anywhere else: a code in the README, read with Git.
    let root = std::env::temp_dir().join(format!("slop-git-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&root).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let serve_root = root.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback_service(tower_http::services::ServeDir::new(serve_root)),
        )
        .await
        .unwrap()
    });
    let repo = format!("http://localhost:{port}/game.git");
    let a = body_json(app.send(ownership(&ann, &repo, false)).await).await;
    assert_eq!(
        (a["ok"].as_bool(), a["method"].as_str()),
        (Some(false), Some("code"))
    );
    let code = a["code"].as_str().unwrap().to_string();
    assert!(
        app.send(publish(&ann, "self-hosted", &repo)).await.status()
            == StatusCode::UNPROCESSABLE_ENTITY
    );

    // The repository, served the way plain web servers serve Git ("dumb" HTTP).
    let git = |args: &[&str], dir: &std::path::Path| {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Ann",
                "-c",
                "user.email=ann@example.com",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&["init", "-q"], &work);
    std::fs::write(work.join("README.md"), "# My game\n\nNothing yet.\n").unwrap();
    git(&["add", "."], &work);
    git(&["commit", "-qm", "first"], &work);
    let publish_repo = |root: &std::path::Path, work: &std::path::Path| {
        let _ = std::fs::remove_dir_all(root.join("game.git"));
        git(
            &[
                "clone",
                "-q",
                "--bare",
                work.to_str().unwrap(),
                root.join("game.git").to_str().unwrap(),
            ],
            root,
        );
        git(&["update-server-info"], &root.join("game.git"));
    };
    publish_repo(&root, &work);
    let a = body_json(app.send(ownership(&ann, &repo, true)).await).await;
    assert_eq!(a["ok"], false, "not there yet: {a}");
    std::fs::write(
        work.join("README.md"),
        format!("# My game\n\n<!-- {code} -->\n"),
    )
    .unwrap();
    git(&["commit", "-qam", "prove it"], &work);
    publish_repo(&root, &work);
    let a = body_json(app.send(ownership(&ann, &repo, true)).await).await;
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(
        app.send(publish(&ann, "self-hosted", &repo)).await.status(),
        StatusCode::CREATED
    );

    // The code is Ann's: Bob gets his own, and the repository is taken anyway.
    let a = body_json(app.send(ownership(&bob, &repo, false)).await).await;
    assert_eq!(a["ok"], false);
    // A draft isn't named to someone else.
    assert!(
        a["message"]
            .as_str()
            .unwrap()
            .contains("already linked to an app on Slop")
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------------------------
// The scan pipeline (DESIGN §10): Slop's scanner, Microsoft Defender from Slop Check, and reviewers

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn scans_and_defender_checks_gate_approval(db: PgPool) {
    // A stand-in for deploy/scanner/slop-scan: holds files named "*held*", passes the rest.
    let dir = std::env::temp_dir().join(format!("slop-scan-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$2\" in *held*) echo '{\"result\":\"on_hold\",\"detail\":\"Flagged by ClamAV: Win.Test in game.exe\"}';;\n *) echo '{\"result\":\"passed\",\"detail\":\"Scanned with ClamAV, YARA and capa: nothing found.\",\"engines\":{\"clamav\":\"test\"}}';; esac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let command = script.to_str().unwrap().to_string();
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let rules = admin::ReviewRules::from_config(&app.config);
    let (_, maker) = maker_with_app(&app, "released").await;
    let id: Uuid = submit_and_fetch(&app, &maker, "1.0.0")
        .await
        .parse()
        .unwrap();
    let (result, report, sha): (String, Value, String) = sqlx::query_as(
        "select scan_result, scan_report, sha256 from release_files where release_id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(result, "passed");
    assert_eq!(report["engines"]["clamav"], "test");

    // Scanned clean, but a Windows build also needs Microsoft Defender's check.
    let err = admin::approve(&app.db, Some(&app.signing_key), rules, id, "rev")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Microsoft Defender"), "{err}");

    // Slop Check, from the app's own repository, reports Defender flagging it.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    let check_ref = RECIPE_REF.replace("build.yml", "check.yml");
    let check = |detections: Value| {
        json!({ "app": "game", "files": [{
            "sha256": sha, "engine": "1.1.25090.3001", "signatures": "1.461.119.0",
            "detections": detections, "signed_by": null
        }] })
    };
    let send = |token: String, body: Value| {
        app.send(
            req(Method::POST, "/v1/slop-build/checks")
                .header("authorization", format!("Bearer {token}"))
                .json(body),
        )
    };
    let res = send(
        github_token(
            &app,
            json!({ "job_workflow_ref": "evil/workflow/.github/workflows/x.yml@main" }),
        ),
        check(json!([])),
    )
    .await;
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "only Slop's own check workflow"
    );
    let res = send(
        github_token(
            &app,
            json!({ "job_workflow_ref": check_ref, "repository": "someone/else" }),
        ),
        check(json!([])),
    )
    .await;
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "only from the app's repository"
    );
    let res = send(
        github_token(&app, json!({ "job_workflow_ref": check_ref })),
        check(json!([{ "file": "game.exe", "threat": "Trojan:Win32/Wacatac.B!ml" }])),
    )
    .await;
    assert_eq!(body_json(res).await["matched"], 1);
    let err = admin::approve(&app.db, Some(&app.signing_key), rules, id, "rev")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Wacatac"), "{err}");

    // A reviewer who judges it a false positive clears it, with a reason; then it can go out.
    assert!(
        admin::clear_scan(&app.db, id, "windows", "rev", "short")
            .await
            .is_err()
    );
    admin::clear_scan(
        &app.db,
        id,
        "windows",
        "rev",
        "Heuristic detection of an unsigned Godot export; submitted to Microsoft as a false positive.",
    )
    .await
    .unwrap();
    admin::approve(&app.db, Some(&app.signing_key), rules, id, "rev")
        .await
        .unwrap();
    let logged: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action = 'release.clear_scan'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(logged, 1);

    // A later clean check never washes out a detection (SECURITY §5.2): the same file in the next
    // release still needs a reviewer to clear it.
    let next: Uuid = {
        sqlx::query("update apps set repo_url = $1")
            .bind(format!("{}octo/game", app.fake_base))
            .execute(&app.db)
            .await
            .unwrap();
        submit_and_fetch(&app, &maker, "1.0.1")
            .await
            .parse()
            .unwrap()
    };
    let clean = json!({ "app": "game", "files": [{
        "sha256": sha, "engine": "1.1.25090.3001", "signatures": "1.461.119.0", "detections": [], "signed_by": "SignPath Foundation"
    }] });
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = send(
        github_token(&app, json!({ "job_workflow_ref": check_ref })),
        clean,
    )
    .await;
    assert_eq!(
        body_json(res).await["matched"],
        0,
        "a clean report doesn't replace a detection"
    );
    let err = admin::approve(&app.db, Some(&app.signing_key), rules, next, "rev")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Wacatac"), "{err}");
    admin::clear_scan(
        &app.db,
        next,
        "windows",
        "rev",
        "Same file as 1.0.0, already judged a false positive and reported to Microsoft.",
    )
    .await
    .unwrap();
    admin::approve(&app.db, Some(&app.signing_key), rules, next, "rev")
        .await
        .unwrap();
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        mine[0]["files"][0]["defender"]["detections"][0]["threat"], "Trojan:Win32/Wacatac.B!ml",
        "the detection stays on record, cleared by the reviewer"
    );

    // Players see the engines and Defender's result, not the findings or the reviewer's reasons.
    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    let public = body_json(app.send(req(Method::GET, "/v1/apps/game/releases")).await).await;
    let cleared = public
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["files"].as_array().unwrap().iter())
        .collect::<Vec<_>>();
    assert!(!cleared.is_empty());
    for f in cleared {
        assert!(f["cleared_reason"].is_null());
        if let Some(report) = f["scan_report"].as_object() {
            assert!(report.keys().all(|k| k == "engines"), "{report:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn storefront_shelves_featured_and_rating_breakdown(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    for slug in ["good-game", "sloppy-game", "hidden-game"] {
        listed_with(&app, maker_id, slug, "released", "").await;
    }
    sqlx::query("update apps set status = 'delisted' where slug = 'hidden-game'")
        .execute(&app.db)
        .await
        .unwrap();

    // Six players launch both games on Windows and vote: good-game 👍, sloppy-game 👎.
    for i in 0..6 {
        let (_, player) = app.user(&format!("Player {i}"), "US").await;
        for (slug, vote) in [("good-game", "not_slop"), ("sloppy-game", "slop")] {
            let res = app
                .send(
                    req(
                        Method::POST,
                        format!("/v1/apps/{slug}/launch?platform=windows"),
                    )
                    .cookie(&player),
                )
                .await;
            assert_eq!(res.status(), StatusCode::OK);
            let res = app
                .send(
                    req(Method::POST, format!("/v1/apps/{slug}/vote"))
                        .cookie(&player)
                        .json(json!({ "vote": vote })),
                )
                .await;
            assert_eq!(res.status(), StatusCode::OK);
        }
        if i < 2 {
            app.send(req(Method::PUT, "/v1/me/library/sloppy-game").cookie(&player))
                .await;
        }
    }

    let now = list_slugs(&app, req(Method::GET, "/v1/apps?sort=not_slop_now")).await;
    assert_eq!(now.first().map(String::as_str), Some("good-game"));
    let fresh = list_slugs(&app, req(Method::GET, "/v1/apps?sort=fresh_slop")).await;
    assert_eq!(
        fresh,
        vec!["sloppy-game"],
        "only apps with 👎 votes this month"
    );
    let popular = list_slugs(&app, req(Method::GET, "/v1/apps?sort=popular")).await;
    assert_eq!(
        popular.first().map(String::as_str),
        Some("sloppy-game"),
        "most library adds this month"
    );

    // The maker's breakdown: by platform, and only for the maker.
    let b = body_json(
        app.send(req(Method::GET, "/v1/apps/good-game/ratings/breakdown").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(b["by_platform"][0]["key"], "windows");
    assert_eq!(b["by_platform"][0]["not_slop"], 6);
    let (_, stranger) = app.user("Stranger", "US").await;
    let res = app
        .send(req(Method::GET, "/v1/apps/good-game/ratings/breakdown").cookie(&stranger))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Content labels: only the known ones.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/good-game")
                .cookie(&maker)
                .json(json!({ "content_labels": ["romance", "romance"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["content_labels"], json!(["romance"]));
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/good-game")
                .cookie(&maker)
                .json(json!({ "content_labels": ["explicit"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Featured: admins only, in order, and only apps in the store show.
    let slides = json!({ "apps": [
        { "slug": "sloppy-game", "blurb": "Gloriously bad" },
        { "slug": "hidden-game", "blurb": "" },
        { "slug": "good-game", "blurb": "Just hit 1.0" },
    ] });
    let res = app
        .send(
            req(Method::PUT, "/v1/admin/featured")
                .cookie(&maker)
                .json(slides.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let (_, admin) = make_admin(&app, "Admin").await;
    let res = app
        .send(
            req(Method::PUT, "/v1/admin/featured")
                .cookie(&admin)
                .json(slides),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let shown = body_json(app.send(req(Method::GET, "/v1/featured")).await).await;
    let order: Vec<&str> = shown
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["slug"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["sloppy-game", "good-game"]);
    assert_eq!(shown[0]["blurb"], "Gloriously bad");
    let res = app
        .send(
            req(Method::PUT, "/v1/admin/featured")
                .cookie(&admin)
                .json(json!({ "apps": [{ "slug": "good-game", "blurb": "x".repeat(161) }] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let logged: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action = 'featured.set'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(logged, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn installed_apps_see_updates_queued_releases_and_other_builds(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();

    let check = || req(Method::GET, "/v1/library/updates?apps=game,nope");
    let u = body_json(app.send(check()).await).await;
    assert_eq!(u.as_array().unwrap().len(), 1, "unknown apps are left out");
    assert_eq!(u[0]["latest"]["version"], "1.0.0");
    assert_eq!(u[0]["latest"]["release_id"], first);
    assert!(u[0]["pending"].is_null());
    assert_eq!(u[0]["builds"], json!([]), "no other builds to switch to");

    // A newer release being checked is the queued update.
    sqlx::query("update apps set auto_update = false")
        .execute(&app.db)
        .await
        .unwrap();
    submit_and_fetch(&app, &maker, "1.1.0").await;
    let u = body_json(app.send(check()).await).await;
    assert_eq!(u[0]["latest"]["version"], "1.0.0");
    assert_eq!(u[0]["pending"]["version"], "1.1.0");
    assert_eq!(u[0]["pending"]["status"], "awaiting_review");

    // A community build in the store makes a family to switch between.
    listed_with(&app, maker_id, "game-fix", "released", "").await;
    sqlx::query(
        "update apps set parent_app_id = (select id from apps where slug = 'game'), branch_kind = 'fix',
                platforms = '{windows}' where slug = 'game-fix'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let u = body_json(app.send(check()).await).await;
    let builds: Vec<(&str, bool)> = u[0]["builds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["slug"].as_str().unwrap(),
                b["official"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(builds, vec![("game", true), ("game-fix", false)]);

    let too_many = (0..101)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let res = app
        .send(req(
            Method::GET,
            format!("/v1/library/updates?apps={too_many}"),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn microsoft_and_discord_sign_in(db: PgPool) {
    let app = test_app_with(db, |c| {
        let base = c.doh_url.clone();
        let oauth = |name: &str| slop_api::config::OAuthConfig {
            client_id: format!("{name}-client"),
            client_secret: "secret".into(),
            authorize_url: base.join(&format!("{name}/auth")).unwrap(),
            token_url: base.join(&format!("{name}/token")).unwrap(),
            api_base: base.join(&format!("{name}/")).unwrap(),
        };
        c.microsoft = Some(oauth("microsoft"));
        c.discord = Some(oauth("discord"));
    })
    .await;
    let providers = body_json(app.send(req(Method::GET, "/v1/auth/providers")).await).await;
    assert_eq!(providers["microsoft"], true);
    assert_eq!(providers["discord"], true);

    // Microsoft: PKCE, and the email is never taken (Microsoft doesn't verify it).
    let res = app.send(req(Method::GET, "/v1/auth/microsoft/start")).await;
    let authorize = location(&res);
    assert!(authorize.contains("code_challenge="), "{authorize}");
    assert!(
        authorize.contains("scope=openid+profile+email"),
        "{authorize}"
    );
    let res = oauth_flow(&app, "/v1/auth/microsoft/start", &[], "mia").await;
    assert_eq!(location(&res), "/signup");
    let signup = set_cookie(&res, "slop_signup").unwrap();
    let info = body_json(
        app.send(req(Method::GET, "/v1/auth/signup").cookie(&signup))
            .await,
    )
    .await;
    assert_eq!(info["suggested_name"], "Mia Soft");
    assert!(info["email"].is_null(), "an unverified email is never used");

    // Discord: no PKCE; the verified email and the username come through.
    let res = app.send(req(Method::GET, "/v1/auth/discord/start")).await;
    assert!(!location(&res).contains("code_challenge="));
    let res = oauth_flow(&app, "/v1/auth/discord/start", &[], "dee").await;
    let signup = set_cookie(&res, "slop_signup").unwrap();
    let info = body_json(
        app.send(req(Method::GET, "/v1/auth/signup").cookie(&signup))
            .await,
    )
    .await;
    assert_eq!(info["suggested_name"], "Dee");
    assert_eq!(info["email"], "dee@example.com");
    let res = app
        .send(req(Method::POST, "/v1/auth/signup").cookie(&signup).json(json!({
            "display_name": "Dee", "country_code": "US", "age_confirmed": true, "accept_terms": true
        })))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let login: String =
        sqlx::query_scalar("select login from identities where provider = 'discord'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(login, "dee");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn private_repositories_outside_github_use_a_read_only_token(db: PgPool) {
    let app = test_app_with(db, |c| c.releases.require_repo_owner = true).await;
    let (_, ann) = app.user("Ann", "US").await;
    let (friend_id, friend) = app.user("Finn", "US").await;
    let repo = format!("{}ann/secret-game", app.fake_base);

    // A wrong token is refused; the right one lets Slop read the README for the code.
    let ownership = |token: &str| {
        req(Method::POST, "/v1/repos/ownership")
            .cookie(&ann)
            .json(json!({ "repo_url": repo, "repo_token": { "kind": "gitea", "token": token } }))
    };
    let res = app.send(ownership("bad")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let a = body_json(app.send(ownership("good")).await).await;
    assert_eq!(a["method"], "code");
    assert_eq!(a["ok"], false);
    *PROOF.lock().unwrap() = a["code"].as_str().unwrap().to_string();
    let res = app
        .send(
            req(Method::POST, "/v1/repos/ownership/check")
                .cookie(&ann)
                .json(
                    json!({ "repo_url": repo, "repo_token": { "kind": "gitea", "token": "good" } }),
                ),
        )
        .await;
    let checked = body_json(res).await;
    assert_eq!(checked["ok"], true, "{checked}");

    // Publishing it as Private Slop keeps the token, encrypted, and never shows it.
    let mut body = new_app("secret-game");
    body["repo_url"] = json!(repo);
    body["visibility"] = json!("private");
    body["platforms"] = json!(["windows"]);
    body["repo_token"] = json!({ "kind": "gitea", "token": "good" });
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&ann).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let sealed: Vec<u8> = sqlx::query_scalar("select sealed from repo_tokens")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!sealed.windows(4).any(|w| w == b"good"), "stored encrypted");
    let status = body_json(
        app.send(req(Method::GET, "/v1/apps/secret-game/repo-token").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(status["saved"], true);
    assert_eq!(status["kind"], "gitea");
    assert!(status.get("token").is_none() && !status.to_string().contains("good"));
    let res = app
        .send(req(Method::GET, "/v1/apps/secret-game/repo-token").cookie(&friend))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // A release is fetched with the token (which isn't sent on to the file storage).
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret-game/releases")
                .cookie(&ann)
                .json(json!({
                    "version": "1.0.0",
                    "commit_sha": "a".repeat(40),
                    "files": [{
                        "platform": "windows",
                        "kind": "archive",
                        "url": format!("{repo}/releases/download/v1/game.zip"),
                        "exec": "game.exe"
                    }]
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release_id = body_json(res).await["id"].as_str().unwrap().to_string();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let status: String = sqlx::query_scalar("select status::text from releases")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "awaiting_review");
    // Its source check read the private repository with the token, as a Gitea host.
    let scan: Value = sqlx::query_scalar("select repo_scan from releases")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(scan["status"], "checked", "{scan}");
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        release_id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();

    // Someone it's shared with gets the host's short-lived link, never the token.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/secret-game/shares")
                .cookie(&ann)
                .json(json!({ "user": friend_id.to_string() })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let link = body_json(
        app.send(
            req(
                Method::GET,
                format!("/v1/apps/secret-game/releases/{release_id}/download/windows"),
            )
            .cookie(&friend),
        )
        .await,
    )
    .await;
    assert!(
        link["url"]
            .as_str()
            .unwrap()
            .starts_with("http://localhost:"),
        "{link}"
    );

    // Forgetting the token: players only get the file's own (private) address back.
    let res = app
        .send(req(Method::DELETE, "/v1/apps/secret-game/repo-token").cookie(&ann))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let link = body_json(
        app.send(
            req(
                Method::GET,
                format!("/v1/apps/secret-game/releases/{release_id}/download/windows"),
            )
            .cookie(&friend),
        )
        .await,
    )
    .await;
    assert_eq!(link["url"], format!("{repo}/releases/download/v1/game.zip"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn admins_are_alerted_once_and_alerts_clear(db: PgPool) {
    let backups = std::env::temp_dir().join(format!("slop-ops-test-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&backups).unwrap();
    let dir = backups.clone();
    let app = test_app_with(db, move |c| c.ops.backup_dir = Some(dir)).await;
    let (admin_id, admin) = make_admin(&app, "Admin").await;
    let (_, player) = app.user("Player", "US").await;
    let ops_notices = || async {
        sqlx::query_scalar::<_, i64>(
            "select coalesce(sum(count), 0)::int8 from notifications where user_id = $1 and category = 'ops'",
        )
        .bind(admin_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };

    // No backups yet: told once, not again five minutes later.
    slop_api::ops::run_once(&app.state).await.unwrap();
    let first = ops_notices().await;
    assert!(first >= 2, "missing backup and restore check");
    slop_api::ops::run_once(&app.state).await.unwrap();
    assert_eq!(ops_notices().await, first);

    // An event from the audit log: a withdrawn release.
    sqlx::query(
        "insert into audit_log (actor, action, subject) values ('Rev', 'release.revoke', 'game 1.0')",
    )
    .execute(&app.db)
    .await
    .unwrap();
    slop_api::ops::run_once(&app.state).await.unwrap();
    assert_eq!(ops_notices().await, first + 1);
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/ops").cookie(&admin))
            .await,
    )
    .await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["key"] == "backup-missing")
    );
    let res = app
        .send(req(Method::GET, "/v1/admin/ops").cookie(&player))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Backups appear: those alerts clear.
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    std::fs::write(backups.join("last-success"), &now).unwrap();
    std::fs::write(backups.join("last-restore-check"), &now).unwrap();
    slop_api::ops::run_once(&app.state).await.unwrap();
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/ops").cookie(&admin))
            .await,
    )
    .await;
    assert!(
        !listed
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["key"].as_str().unwrap().starts_with("backup")),
        "{listed}"
    );
    let player_ops: i64 = sqlx::query_scalar(
        "select count(*) from notifications where category = 'ops' and user_id <> $1",
    )
    .bind(admin_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(player_ops, 0, "only admins get alerts");
    std::fs::remove_dir_all(&backups).unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn live_releases_are_rescanned_and_new_findings_go_to_review(db: PgPool) {
    // A stand-in scanner that finds something once `mode` says so.
    let dir = std::env::temp_dir().join(format!("slop-rescan-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let mode = dir.join("mode");
    std::fs::write(&mode, "clean").unwrap();
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif grep -q held {mode}; then echo '{{\"result\":\"on_hold\",\"detail\":\"Flagged by YARA: Trojan.Test in game.exe\",\"findings\":[{{\"engine\":\"YARA\",\"file\":\"game.exe\",\"name\":\"Trojan.Test\"}}],\"capabilities\":[],\"engines\":{{}}}}'; else echo '{{\"result\":\"passed\",\"detail\":\"nothing found\",\"findings\":[],\"capabilities\":[],\"engines\":{{}}}}'; fi\n",
            mode = mode.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let command = script.to_str().unwrap().to_string();
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (_, maker) = maker_with_app(&app, "released").await;
    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    let id: Uuid = submit_and_fetch(&app, &maker, "1.0.0")
        .await
        .parse()
        .unwrap();
    admin::approve(&app.db, Some(&app.signing_key), lax(), id, "rev")
        .await
        .unwrap();

    // Just approved: not due for a week.
    assert_eq!(slop_api::rescan::run_once(&app.state).await.unwrap(), 0);

    // A malware report re-scans it now; still clean.
    let (_, reporter) = app.user("Reporter", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&reporter)
                .json(json!({
                    "kind": "app", "subject": "game", "reason": "malware",
                    "details": "It asked for my password.", "good_faith": true
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(slop_api::rescan::run_once(&app.state).await.unwrap(), 1);
    let (result, flagged): (String, Option<time::OffsetDateTime>) = sqlx::query_as(
        "select rescan_result, rescan_flagged_at from release_files where release_id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(result, "passed");
    assert!(flagged.is_none());
    assert_eq!(
        slop_api::rescan::run_once(&app.state).await.unwrap(),
        0,
        "the request is done"
    );

    // New signatures find something: the release goes to review, players keep it meanwhile.
    std::fs::write(&mode, "held").unwrap();
    slop_api::rescan::request(&app.db, app_id(&app, "game").await, "test")
        .await
        .unwrap();
    assert_eq!(slop_api::rescan::run_once(&app.state).await.unwrap(), 1);
    let (_, admin_cookie) = make_admin(&app, "Admin").await;
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(queue["rescans"][0]["slug"], "game");
    assert_eq!(
        queue["rescans"][0]["rescan_report"]["findings"][0]["name"],
        "Trojan.Test"
    );
    let status: String = sqlx::query_scalar("select status::text from releases where id = $1")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "approved");

    // The reviewer keeps it (a false positive): it isn't raised again for the same finding.
    let clear = |reason: &str| {
        req(
            Method::POST,
            format!("/v1/admin/releases/{id}/files/windows/rescan-clear"),
        )
        .cookie(&admin_cookie)
        .json(json!({ "reason": reason }))
    };
    assert_eq!(
        app.send(clear("ok")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(clear("Test signature, matches a known false positive"))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    slop_api::rescan::request(&app.db, app_id(&app, "game").await, "test")
        .await
        .unwrap();
    slop_api::rescan::run_once(&app.state).await.unwrap();
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
            .await,
    )
    .await;
    assert_eq!(queue["rescans"], json!([]));
    let logged: i64 = sqlx::query_scalar(
        "select count(*) from audit_log where action in ('release.rescan_flagged', 'release.rescan_cleared')",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(logged, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

async fn app_id(app: &TestApp, slug: &str) -> Uuid {
    sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_watch_and_moderate_through_the_api_and_mcp(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Admin").await;
    let (troll_id, troll) = app.user("Troll Account", "US").await;
    let mut tokens = std::collections::HashMap::new();
    for scope in ["review", "moderate"] {
        let res = app
            .send(
                req(Method::POST, "/v1/admin/tokens")
                    .cookie(&admin)
                    .json(json!({ "name": format!("Claude {scope}"), "scope": scope, "days": 7 })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        tokens.insert(
            scope,
            format!("Bearer {}", body_json(res).await["token"].as_str().unwrap()),
        );
    }
    let as_token = |scope: &str, r: Req| r.header("authorization", tokens[scope].clone());

    // Watching: the status overview, and finding an account.
    let status = body_json(
        app.send(as_token("review", req(Method::GET, "/v1/admin/status")))
            .await,
    )
    .await;
    assert!(status["waiting"]["reports_urgent"].is_number(), "{status}");
    assert!(status["alerts"].is_array());
    let found = body_json(
        app.send(as_token(
            "review",
            req(Method::GET, "/v1/admin/users?q=troll"),
        ))
        .await,
    )
    .await;
    assert_eq!(found[0]["id"], troll_id.to_string());
    let res = app
        .send(req(Method::GET, format!("/v1/admin/users/{troll_id}")).cookie(&troll))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "admins only");

    // A review token can warn, not suspend.
    let decide = |scope: &str, action: &str| {
        as_token(
            scope,
            req(Method::POST, format!("/v1/admin/users/{troll_id}/decide")).json(json!({
                "action": action, "rule": "harassment",
                "facts": "Sent the same insult to twelve people in a day."
            })),
        )
    };
    assert_eq!(
        app.send(decide("review", "warn")).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(decide("review", "suspend_account")).await.status(),
        StatusCode::FORBIDDEN
    );

    // Over MCP, with the moderate token: the tools, then a suspension.
    let rpc =
        |scope: &str, body: Value| as_token(scope, req(Method::POST, "/v1/admin/mcp").json(body));
    let init = body_json(
        app.send(rpc(
            "moderate",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        ))
        .await,
    )
    .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "slop-admin");
    let res = app
        .send(rpc(
            "moderate",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let tools = body_json(
        app.send(rpc(
            "moderate",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ))
        .await,
    )
    .await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"slop_status") && names.contains(&"moderate_user"),
        "{names:?}"
    );
    let call = |scope: &str, name: &str, args: Value| {
        rpc(
            scope,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": name, "arguments": args } }),
        )
    };
    let suspend = json!({
        "id": troll_id.to_string(), "action": "suspend_account", "rule": "harassment",
        "facts": "Kept messaging people after a warning."
    });
    let refused = body_json(
        app.send(call("review", "moderate_user", suspend.clone()))
            .await,
    )
    .await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let done = body_json(app.send(call("moderate", "moderate_user", suspend)).await).await;
    assert_eq!(done["result"]["isError"], false, "{done}");
    let suspended: bool =
        sqlx::query_scalar("select suspended_at is not null from users where id = $1")
            .bind(troll_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(suspended);
    let details = body_json(
        app.send(call(
            "moderate",
            "user_details",
            json!({ "id": troll_id.to_string() }),
        ))
        .await,
    )
    .await;
    let text = details["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("suspend_account") && text.contains("warn"),
        "{text}"
    );

    // Lifting it, with a reason; the person is told, and every step is in the audit log.
    let lifted = body_json(
        app.send(call(
            "moderate",
            "lift_suspension",
            json!({ "id": troll_id.to_string(), "reason": "Apologised and the reports were a misunderstanding." }),
        ))
        .await,
    )
    .await;
    assert_eq!(lifted["result"]["isError"], false, "{lifted}");
    let notices: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and category = 'moderation'",
    )
    .bind(troll_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(notices, 3, "warning, suspension, lifted");
    let actors: Vec<String> = sqlx::query_scalar(
        "select actor from audit_log where action in ('moderation.suspend_account', 'moderation.lift_suspension', 'user.view')",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(actors.len(), 3);
    assert!(
        actors.iter().all(|a| a.contains("Claude moderate")),
        "{actors:?}"
    );
    let unknown = body_json(app.send(call("moderate", "drop_database", json!({}))).await).await;
    assert!(unknown["error"].is_object());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sensitive_actions_need_two_people_and_roles_limit_staff(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (a_id, a) = make_admin(&app, "Ada Owner").await;
    let (_, b) = make_admin(&app, "Ben Owner").await;
    let (c_id, c) = app.user("Cleo", "US").await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let release_status = || async {
        sqlx::query_scalar::<_, String>("select status::text from releases where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    // Ada asks to withdraw it: recorded, waiting for someone else.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/releases/{id}/revoke"))
                .cookie(&a)
                .json(json!({ "reason": "Ships a crypto miner." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let pending = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(release_status().await, "approved");
    let approve = |cookie: &str| {
        req(Method::POST, format!("/v1/admin/actions/{pending}/approve")).cookie(cookie)
    };
    assert_eq!(
        app.send(approve(&a)).await.status(),
        StatusCode::CONFLICT,
        "not twice by Ada"
    );
    // Nor through any agent: only people approve.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&a)
                .json(json!({ "name": "Ada's agent", "scope": "moderate", "days": 7 })),
        )
        .await;
    let token = format!("Bearer {}", body_json(res).await["token"].as_str().unwrap());
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/actions/{pending}/approve"))
                .header("authorization", token),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/actions").cookie(&b))
            .await,
    )
    .await;
    assert_eq!(listed["approvals_required"], 2);
    assert_eq!(listed["actions"][0]["kind"], "release.revoke");
    // An old session needs no "confirm it's you" (SECURITY §4.3, 2026-10-25).
    sqlx::query("update sessions set created_at = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();
    // Ben approves: now it happens, in both names.
    assert_eq!(app.send(approve(&b)).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(release_status().await, "revoked");
    let actor: String =
        sqlx::query_scalar("select actor from audit_log where action = 'release.revoke'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        actor.contains("Ada Owner") && actor.contains("approved by Ben Owner"),
        "{actor}"
    );

    // Ada makes Cleo support staff; Ben approves.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{c_id}/roles"))
                .cookie(&a)
                .json(json!({ "role": "support", "grant": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let role_action = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/actions/{role_action}/approve"),
            )
            .cookie(&b),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let staff = body_json(
        app.send(req(Method::GET, "/v1/admin/staff").cookie(&c))
            .await,
    )
    .await;
    assert!(staff["my_roles"] == json!(["support"]), "{staff}");

    // Support staff look and handle reports, but don't review releases or act on sensitive ones.
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    let res = app
        .send(req(Method::POST, format!("/v1/admin/releases/{second}/approve")).cookie(&c))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{pending}/approve")).cookie(&c))
        .await;
    assert!(res.status() == StatusCode::FORBIDDEN || res.status() == StatusCode::CONFLICT);

    // A suspension request Ben turns down never happens.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/users/{c_id}/decide"))
                .cookie(&a)
                .json(
                    json!({ "action": "suspend_account", "rule": "harassment", "facts": "Test." }),
                ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let suspend = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{suspend}/cancel")).cookie(&b))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let suspended: bool =
        sqlx::query_scalar("select suspended_at is not null from users where id = $1")
            .bind(c_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!suspended);
    let _ = a_id;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn accounts_are_rate_limited_per_action(db: PgPool) {
    // Per-IP limits off (every test request comes from one address), to reach the account's.
    let app = test_app_with(db, |c| c.rate_limits.write_per_minute = 10_000).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "soon-game", "coming_soon", "").await;
    let (_, player) = app.user("Busy Player", "US").await;
    let (_, other) = app.user("Other Player", "US").await;
    let mark = |cookie: &str| req(Method::PUT, "/v1/me/interested/soon-game").cookie(cookie);
    for _ in 0..120 {
        assert!(app.send(mark(&player)).await.status().is_success());
    }
    let res = app.send(mark(&player)).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().get("retry-after").is_some());
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    assert!(
        app.send(mark(&other)).await.status().is_success(),
        "others aren't affected"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn abuse_checks_raise_vote_bursts_and_lookalike_names_once(db: PgPool) {
    let app = test_app_with(db, |c| c.rate_limits.write_per_minute = 10_000).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let (other_id, _) = app.user("Copycat", "US").await;
    listed_with(
        &app,
        maker_id,
        "stardew",
        "released",
        "title = 'Stardew Valley'",
    )
    .await;
    // Twelve brand-new accounts vote on it within the hour.
    for i in 0..12 {
        let (_, voter) = app.user(&format!("New {i}"), "US").await;
        app.send(req(Method::POST, "/v1/apps/stardew/launch?platform=windows").cookie(&voter))
            .await;
        let res = app
            .send(
                req(Method::POST, "/v1/apps/stardew/vote")
                    .cookie(&voter)
                    .json(json!({ "vote": "slop" })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    // Another maker names theirs almost the same.
    listed_with(
        &app,
        other_id,
        "stardew-2",
        "released",
        "title = 'Stardew Va1ley', status = 'draft'",
    )
    .await;

    let raised = slop_api::abuse::run_once(&app.state).await.unwrap();
    assert!(raised >= 2, "{raised}");
    assert_eq!(
        slop_api::abuse::run_once(&app.state).await.unwrap(),
        0,
        "raised once"
    );
    let (_, admin) = make_admin(&app, "Admin").await;
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    let kinds: Vec<&str> = queue["abuse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"vote_burst") && kinds.contains(&"lookalike_name"),
        "{kinds:?}"
    );
    let look = queue["abuse"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "lookalike_name")
        .unwrap();
    assert_eq!(look["slug"], "stardew-2");
    assert_eq!(look["detail"]["looks_like"], "Stardew Valley");
    let id = look["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/abuse/{id}/dismiss"))
                .cookie(&admin)
                .json(json!({ "note": "Same maker's sequel? No: asked them to rename.", "acted": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(queue["abuse"].as_array().unwrap().len(), kinds.len() - 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn payments_and_vote_history_are_append_only_and_vat_numbers_sealed(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (voter_id, voter) = app.user("Voter", "US").await;
    app.send(req(Method::POST, "/v1/apps/game/launch").cookie(&voter))
        .await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/vote")
                .cookie(&voter)
                .json(json!({ "vote": "not_slop" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    sqlx::query(
        "insert into payments (id, kind, amount_minor, currency, simulated) values (gen_random_uuid(), 'people_job', 1000, 'usd', true)",
    )
    .execute(&app.db)
    .await
    .unwrap();

    // No edits or deletions, even from the server's own database account.
    for sql in [
        "update payments set amount_minor = 1",
        "delete from payments",
        "update vote_events set vote = -1",
        "delete from vote_events",
    ] {
        let err = sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&app.db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
    // Except replacing the voter's ID when they close their account.
    sqlx::query("update vote_events set user_id = gen_random_uuid() where user_id = $1")
        .bind(voter_id)
        .execute(&app.db)
        .await
        .unwrap();

    // A VAT number is stored sealed and shown as entered.
    let (_, brit) = app.user("Brit", "GB").await;
    let res = app
        .send(
            req(Method::PUT, "/v1/me/vat")
                .cookie(&brit)
                .json(json!({ "vat_number": "GB 123 4567 89" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let stored: String =
        sqlx::query_scalar("select vat_number from users where display_name = 'Brit'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(
        stored.starts_with("enc1:") && !stored.contains("123456789"),
        "{stored}"
    );
    let statements = body_json(
        app.send(req(Method::GET, "/v1/me/fee-statements").cookie(&brit))
            .await,
    )
    .await;
    assert_eq!(statements["vat"]["vat_number"], "GB123456789");
    let export = body_json(
        app.send(req(Method::GET, "/v1/me/export").cookie(&brit))
            .await,
    )
    .await;
    assert_eq!(export["account"]["vat_number"], "GB123456789");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_hear_about_every_release_and_security_txt_is_served(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let titles: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 and category = 'apps' order by created_at",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        titles,
        ["Game 1.0.0 was submitted", "Game 1.0.0 is approved"],
        "no notice asks for a passkey (optional since 2026-10-25)"
    );

    let res = app
        .send(req(Method::GET, "/.well-known/security.txt"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()["permissions-policy"]
            .to_str()
            .unwrap()
            .contains("camera=()")
    );
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.starts_with("Contact: mailto:security@slop.example\nExpires: "),
        "{text}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn passkeys_are_added_from_any_session_and_still_confirm_staff_tokens(db: PgPool) {
    use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
    let app = test_app(db).await;
    let (user_id, cookie) = app.user("Pia", "US").await;
    let origin = url::Url::parse("http://localhost:8080").unwrap();
    let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));

    // Add one.
    let started = body_json(
        app.send(req(Method::POST, "/v1/me/passkeys/register/start").cookie(&cookie))
            .await,
    )
    .await;
    let options = serde_json::from_value(started["options"].clone()).unwrap();
    let credential = authenticator
        .do_registration(origin.clone(), options)
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/me/passkeys/register/finish")
                .cookie(&cookie)
                .json(json!({ "challenge_id": started["challenge_id"], "name": "Laptop", "credential": credential })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    // A challenge works once.
    let res = app
        .send(
            req(Method::POST, "/v1/me/passkeys/register/finish")
                .cookie(&cookie)
                .json(json!({ "challenge_id": started["challenge_id"], "name": "Again", "credential": credential })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let listed = body_json(
        app.send(req(Method::GET, "/v1/me/passkeys").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(listed[0]["name"], "Laptop");

    // An old session adds another without confirming first (SECURITY §4.3, 2026-10-25).
    sqlx::query("update sessions set created_at = now() - interval '1 hour' where user_id = $1")
        .bind(user_id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/me/passkeys/register/start").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    // Passkeys no longer have a step-up ceremony.
    let res = app
        .send(req(Method::POST, "/v1/me/step-up/start").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Signing in starts a usernameless ceremony.
    let started = body_json(app.send(req(Method::POST, "/v1/auth/passkey/start")).await).await;
    assert!(
        started["options"]["publicKey"]["challenge"].is_string(),
        "{started}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_sign_hand_built_files_with_a_registered_key(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[21; 32]);
    let (public_line, signature) = ssh_sign(&key, BUILD_BYTES);

    // Adding a key works from an old session (SECURITY §4.3, 2026-10-25); it waits 72 hours.
    sqlx::query("update sessions set created_at = now() - interval '1 hour' where user_id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let add = || {
        req(Method::POST, "/v1/me/signing-keys")
            .cookie(&maker)
            .json(json!({ "name": "Laptop", "public_key": public_line }))
    };
    let res = app.send(add()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let added = body_json(res).await;
    assert_eq!(added["active"], false);
    assert!(
        added["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title = 'A signing key was added to your account'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);

    // A signed file before the wait is over doesn't count.
    *MAKER_SIG.lock().unwrap() = signature.clone();
    let early = submit_and_fetch(&app, &maker, "1.0.0").await;
    let problem: String = sqlx::query_scalar(
        "select provenance_problem from release_files where release_id = $1::uuid",
    )
    .bind(&early)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(problem.contains("72-hour"), "{problem}");

    // After it: the file is the maker's, approval accepts it, and the manifest carries it.
    sqlx::query("update maker_keys set active_from = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();
    let id = submit_and_fetch(&app, &maker, "1.0.1").await;
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    let file = &mine
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap()["files"][0];
    assert_eq!(file["maker_key"]["key_name"], "Laptop", "{file}");
    assert!(file["maker_key"].get("signature").is_none());
    let strict = admin::ReviewRules {
        require_maker_signature: true,
        scanner: slop_api::scan::Scanner::Off,
    };
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        strict,
        early.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap_err();
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        strict,
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let manifest: Value =
        sqlx::query_scalar("select signed_manifest from releases where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let text = manifest.to_string();
    assert!(
        text.contains("maker_signature") && text.contains("BEGIN SSH SIGNATURE"),
        "{text}"
    );

    // A wrong signature is refused with the reason.
    let other = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[22; 32]);
    *MAKER_SIG.lock().unwrap() = ssh_sign(&other, BUILD_BYTES).1;
    let bad = submit_and_fetch(&app, &maker, "1.0.2").await;
    let problem: String = sqlx::query_scalar(
        "select provenance_problem from release_files where release_id = $1::uuid",
    )
    .bind(&bad)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(problem.contains("different key"), "{problem}");
    MAKER_SIG.lock().unwrap().clear();
}

/// The API's own database role (migration 0050): it reads and writes as usual, but only adds
/// to the ledgers, and can't switch off their append-only triggers.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_apis_role_only_adds_to_the_ledgers(db: PgPool) {
    let role = format!("slop_app_test_{:08x}", rand::random::<u32>());
    sqlx::query(sqlx::AssertSqlSafe(format!("create role {role} nologin")))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("select slop_grant_app_role($1)")
        .bind(&role)
        .execute(&db)
        .await
        .unwrap();
    let can = |table: &'static str, what: &'static str| {
        let (db, role) = (db.clone(), role.clone());
        async move {
            sqlx::query_scalar::<_, bool>("select has_table_privilege($1, $2, $3)")
                .bind(role)
                .bind(table)
                .bind(what)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    for ledger in ["payments", "vote_events", "audit_log"] {
        assert!(
            can(ledger, "INSERT").await && can(ledger, "SELECT").await,
            "{ledger}"
        );
        for what in ["UPDATE", "DELETE", "TRUNCATE"] {
            assert!(!can(ledger, what).await, "{ledger} {what}");
        }
    }
    for table in ["apps", "users", "votes", "releases"] {
        assert!(
            can(table, "UPDATE").await && can(table, "DELETE").await,
            "{table}"
        );
    }
    let user_id: bool =
        sqlx::query_scalar("select has_column_privilege($1, 'vote_events', 'user_id', 'UPDATE')")
            .bind(&role)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(user_id, "closing an account replaces its ID");

    // As that role: the triggers stay on, and the ledgers can't be changed.
    let mut tx = db.begin().await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("set local role {role}")))
        .execute(&mut *tx)
        .await
        .unwrap();
    for (sql, expect) in [
        (
            "alter table payments disable trigger payments_append_only",
            "must be owner",
        ),
        (
            "drop trigger vote_events_append_only on vote_events",
            "must be owner",
        ),
        ("delete from payments", "permission denied"),
        ("update audit_log set action = action", "permission denied"),
    ] {
        sqlx::query("savepoint try_it")
            .execute(&mut *tx)
            .await
            .unwrap();
        let err = sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains(expect), "{sql}: {err}");
        sqlx::query("rollback to savepoint try_it")
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.rollback().await.unwrap();

    // Roles belong to the whole server: tidy up.
    for sql in [format!("drop owned by {role}"), format!("drop role {role}")] {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&db)
            .await
            .unwrap();
    }
}

/// Passkeys are optional (the owner's decision of 2026-10-25): publishing works without one,
/// long after first publishing and past a deadline set before then, with no notice asking for
/// one, and the staff "passkey grace" is gone.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn publishers_never_need_a_passkey(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, admin) = make_admin(&app, "Ada Owner").await;
    let create = |slug: &str| {
        req(Method::POST, "/v1/apps")
            .cookie(&maker)
            .json(new_app(slug))
    };

    let res = app.send(create("one")).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let notices: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title ilike '%passkey%'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(notices, 0, "no notice asks for one");
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&maker)).await).await;
    assert_eq!(me["has_passkey"], false);
    assert!(me.get("passkey_due_at").is_none());

    // A deadline set before 2026-10-25, long past, and an old session: still fine.
    sqlx::query("update users set passkey_due_at = now() - interval '60 days' where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "update sessions set created_at = now() - interval '40 days', stepped_up_at = null, passkey_at = null where user_id = $1",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(app.send(create("two")).await.status(), StatusCode::CREATED);

    // With a passkey that hasn't been used for days: the same.
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, '\\x01', '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        app.send(create("three")).await.status(),
        StatusCode::CREATED
    );

    // Nothing for staff to give any more.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/users/{maker_id}/passkey-grace"),
            )
            .cookie(&admin)
            .json(json!({ "reason": "Video call; matched the GitHub account." })),
        )
        .await;
    assert!(res.status().is_client_error(), "{}", res.status());
}

/// A new icon like another maker's live one is pointed out to the reviewer (media.rs
/// `lookalikes`); the maker's own apps don't count.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn icons_like_another_makers_are_pointed_out(db: PgPool) {
    let app = test_app(db).await;
    let (famous_maker, _) = app.user("Famous Studio", "US").await;
    let (copycat, cookie) = app.user("Copy Cat", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Admin").await;
    // The famous game's live icon: stripes on a gradient.
    let icon = |w: u32, h: u32, tint: u8| {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            let band = (x * 8 / w + y * 3 / h).is_multiple_of(2);
            let fx = (x * 255 / w) as u8;
            if band {
                image::Rgb([fx.saturating_add(tint), 40, 200])
            } else {
                image::Rgb([20, fx, 60u8.saturating_add(tint)])
            }
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    };
    listed_with(&app, famous_maker, "famous", "released", "").await;
    let hash = format!(
        "{:016x}",
        slop_api::media::dhash(&icon(920, 430, 0)).unwrap()
    );
    sqlx::query(
        "with s as (
             insert into media_sets (id, app_id, commit_sha, thumbnail, fingerprint, source, status)
             select $1, a.id, repeat('c', 40),
                    jsonb_build_object('path', 'thumb.png', 'url', 'https://example.com/t.png', 'sha256', 'x',
                                       'width', 920, 'height', 430, 'format', 'png', 'dhash', $2::text),
                    'f', 'maker', 'approved'
             from apps a where a.slug = 'famous' returning id, app_id)
         update apps set media_set_id = s.id from s where apps.id = s.app_id",
    )
    .bind(Uuid::now_v7())
    .bind(&hash)
    .execute(&app.db)
    .await
    .unwrap();

    // The copy: the same picture, a little bigger and tinted.
    let head = "b".repeat(40);
    app_with_pictures(&app, copycat, "not-famous", &head).await;
    put_raw(&head, "thumb.png", icon(1000, 470, 10));
    let res = app
        .send(req(Method::POST, "/v1/apps/not-famous/media/check").cookie(&cookie))
        .await;
    let body = body_json(res).await;
    assert_eq!(body["result"], "pending", "{body}");
    let like = &body["pending"]["lookalikes"];
    assert_eq!(like[0]["slug"], "famous", "{body}");
    assert!(like[0]["distance"].as_u64().unwrap() <= 10);

    // Reviewers see it with the pictures.
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/review").cookie(&admin_cookie))
            .await,
    )
    .await;
    let media = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "media")
        .expect("pictures waiting");
    assert_eq!(
        media["detail"]["pictures"]["lookalikes"][0]["title"],
        "famous"
    );

    // The same maker reusing their own icon isn't pointed out.
    sqlx::query("update apps set maker_id = $1 where slug = 'famous'")
        .bind(copycat)
        .execute(&app.db)
        .await
        .unwrap();
    put_raw(&head, "thumb.png", icon(1000, 470, 12));
    let body = body_json(
        app.send(req(Method::POST, "/v1/apps/not-famous/media/check").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(body["pending"]["lookalikes"], json!([]), "{body}");
}

/// Known-bad links (bad_links.rs): a store page linking to an address on a public malware list
/// becomes an abuse signal for a person; shared sites only match the exact address.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn links_to_known_bad_addresses_are_pointed_out(db: PgPool) {
    put_raw(
        "feed",
        "bad.txt",
        b"# URLhaus-style list\nhttp://evil.example/other.exe\nhttps://raw.githubusercontent.com/x/y/main/z.exe\n"
            .to_vec(),
    );
    let fake = fake_github().await;
    let app = test_app_with(db, |c| {
        c.bad_link_feeds = vec![
            format!("{fake}raw/octo/game/feed/.slop/bad.txt")
                .parse()
                .unwrap(),
        ]
    })
    .await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    sqlx::query(
        "update apps set description_md = $1, updated_at = now() where slug = 'castle'",
    )
    .bind(
        "Get the [mod tools](https://www.evil.example/tools.zip). Code: https://github.com/mia/castle. \
         Extras: https://raw.githubusercontent.com/x/y/main/z.exe and https://raw.githubusercontent.com/mia/castle/main/readme.md",
    )
    .execute(&app.db)
    .await
    .unwrap();

    slop_api::abuse::run_once(&app.state).await.unwrap();
    let mut found: Vec<(String, String)> = sqlx::query_as(
        "select detail->>'link', detail->>'listed_by' from abuse_signals where kind = 'bad_link'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    found.sort();
    let host = url::Url::parse(&fake)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    assert_eq!(
        found,
        [
            (
                "https://raw.githubusercontent.com/x/y/main/z.exe".to_string(),
                host.clone()
            ),
            ("https://www.evil.example/tools.zip".to_string(), host),
        ],
        "the listed site, and on a shared site only the listed address"
    );
    let lists: i64 = sqlx::query_scalar("select count(*) from bad_links")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(lists, 2);

    // Raised once.
    assert_eq!(slop_api::abuse::run_once(&app.state).await.unwrap(), 0);
}

/// Days the abuse checks found unusual voting are marked on the ratings chart, unless staff
/// found nothing wrong (votes.rs `unusual_days`).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unusual_voting_days_are_marked_on_the_chart(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    for (kind, ago, status) in [
        ("review_bomb", "2 days", "open"),
        ("vote_burst", "5 days", "actioned"),
        ("vote_swing", "9 days", "dismissed"),
        ("lookalike_name", "1 day", "open"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail, status, detected_at)
             select $1, $2, $2 || a.id, 'app', a.id, a.title, '{{}}', $3, now() - interval '{ago}'
             from apps a where a.slug = 'castle'"
        )))
        .bind(Uuid::now_v7())
        .bind(kind)
        .bind(status)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let ratings = body_json(app.send(req(Method::GET, "/v1/apps/castle/ratings")).await).await;
    let day = |ago: i64| {
        (OffsetDateTime::now_utc() - time::Duration::days(ago))
            .date()
            .to_string()
    };
    assert_eq!(
        ratings["unusual_days"],
        json!([day(5), day(2)]),
        "votes only, and not what staff dismissed"
    );
}

/// The law-enforcement request log (CHAT-3) and the transparency report (CHAT-4).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn legal_requests_are_logged_answered_and_counted(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Owner").await;
    let (player, someone) = app.user("Pat Player", "US").await;
    let log = |cookie: &str, body: Value| {
        req(Method::POST, "/v1/admin/legal-requests")
            .cookie(cookie)
            .json(body)
    };
    let now = OffsetDateTime::now_utc();
    let received = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let order = json!({
        "received_at": received, "country": "de", "authority": "Bundeskriminalamt",
        "kind": "removal_order", "legal_basis": "Regulation (EU) 2021/784 Art. 3",
        "asked": "Remove the app castle-rock within one hour."
    });
    assert_eq!(
        app.send(log(&someone, order.clone())).await.status(),
        StatusCode::NOT_FOUND
    );
    let mut bad = order.clone();
    bad["kind"] = json!("please");
    assert_eq!(
        app.send(log(&admin, bad)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(log(&admin, order)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let removal = body_json(res).await;
    assert!(removal["reference"].as_str().unwrap().starts_with("LR-"));
    let res = app
        .send(log(
            &admin,
            json!({
                "received_at": received, "country": "US", "authority": "County Sheriff",
                "kind": "subpoena", "asked": "Account details for Pat Player.",
                "subject_user_id": player
            }),
        ))
        .await;
    let subpoena = body_json(res).await;

    // The removal order's hour is counted from when it arrived, and admins are told.
    let due = slop_api::legal_requests::due_soon(&app.db).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].1, "removal_order");
    let list = body_json(
        app.send(req(Method::GET, "/v1/admin/legal-requests").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(
        list[0]["kind"], "removal_order",
        "open, soonest deadline first"
    );
    assert_eq!(list[1]["subject_name"], "Pat Player", "{list}");
    assert_eq!(list[1]["user_notice"], "pending");

    // Answered once, with what was given and whether the person was told.
    let answer = |id: &Value| {
        req(Method::POST, format!("/v1/admin/legal-requests/{}/answer", id.as_str().unwrap()))
            .cookie(&admin)
            .json(json!({ "status": "partly_answered", "response": "Gave the sign-up date and country; no messages exist on our servers.", "user_notice": "notified" }))
    };
    assert_eq!(
        app.send(answer(&subpoena["id"])).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(answer(&subpoena["id"])).await.status(),
        StatusCode::CONFLICT
    );
    let audited: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action like 'legal_request.%'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(audited, 3);

    // The public report: counts only, signed out.
    let year = now.year();
    let report = body_json(
        app.send(req(Method::GET, format!("/v1/transparency/{year}")))
            .await,
    )
    .await;
    assert_eq!(
        report["legal_requests"]["by_country"],
        json!([{ "key": "DE", "count": 1 }, { "key": "US", "count": 1 }])
    );
    assert_eq!(
        report["legal_requests"]["by_outcome"],
        json!([{ "key": "open", "count": 1 }, { "key": "partly_answered", "count": 1 }])
    );
    assert!(!report.to_string().contains("Pat Player"), "never who");
    assert_eq!(
        app.send(req(Method::GET, "/v1/transparency/1999"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

/// Global Privacy Control (PRIV-10): honoured as an opt-out of activity-based recommendations,
/// across devices, unless the person turns them on themselves.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn global_privacy_control_turns_personal_recommendations_off(db: PgPool) {
    let app = test_app(db).await;
    let (_, user) = app.user("Pat Player", "US").await;
    let me = |gpc: bool| {
        let mut r = req(Method::GET, "/v1/me")
            .cookie(&user)
            .header("x-country", "US");
        if gpc {
            r = r.header("sec-gpc", "1");
        }
        let app = &app;
        async move { body_json(app.send(r).await).await }
    };
    assert_eq!(me(false).await["personalize"], true);
    let with = me(true).await;
    assert_eq!(
        (&with["personalize"], &with["gpc"]),
        (&json!(false), &json!(true))
    );
    assert_eq!(
        me(false).await["personalize"],
        false,
        "remembered, on every device"
    );
    let res = app
        .send(
            req(Method::PATCH, "/v1/me")
                .cookie(&user)
                .json(json!({ "personalize": true })),
        )
        .await;
    assert!(res.status().is_success());
    assert_eq!(me(true).await["personalize"], true, "their own choice wins");
}

/// Records go once their keeping period is over (retention.rs, LEGAL-GUIDANCE §15), and not before.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn records_are_purged_after_their_keeping_period(db: PgPool) {
    let app = test_app(db).await;
    let (old_user, _) = app.user("Gone Long Ago", "US").await;
    let (user, _) = app.user("Still Here", "US").await;
    let run = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query(sql).execute(&db).await.unwrap();
        }
    };
    sqlx::query("update users set closed_at = now() - interval '7 years' where id = $1")
        .bind(old_user)
        .execute(&app.db)
        .await
        .unwrap();
    let report = |reference: &str, age: &str, preserve: &str| {
        format!(
            "insert into reports (id, reference, subject_kind, subject_id, subject_label, reason, details, priority, due_at, created_at, preserve_until)
             values (gen_random_uuid(), '{reference}', 'app', gen_random_uuid(), 'x', 'spam', 'x', 2, now(), now() - interval '{age}', {preserve})"
        )
    };
    for sql in [
        report("R-OLD", "7 years", "null"),
        report("R-NEW", "5 years", "null"),
        report("R-PRESERVED", "7 years", "now() + interval '1 month'"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&app.db)
            .await
            .unwrap();
    }
    for (age, reference) in [("7 years", "D-OLD"), ("1 year", "D-NEW")] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into moderation_decisions (id, reference, user_id, subject_kind, subject_id, subject_label, action, rule, facts, decided_by, created_at)
             values (gen_random_uuid(), '{reference}', $1, 'app', gen_random_uuid(), 'x', 'warn', 'spam', 'x', 'test', now() - interval '{age}')"
        )))
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    }
    run("insert into legal_requests (id, reference, received_at, country, authority, kind, asked, created_by, status, answered_at)
         values (gen_random_uuid(), 'LR-OLD', now() - interval '7 years', 'US', 'x', 'subpoena', 'x', 'test', 'answered', now() - interval '7 years'),
                (gen_random_uuid(), 'LR-NEW', now() - interval '7 years', 'US', 'x', 'subpoena', 'x', 'test', 'answered', now() - interval '5 years')").await;

    let purged = slop_api::retention::purge(&app.db).await.unwrap();
    assert_eq!(
        purged,
        slop_api::retention::Purged {
            reports: 1,
            decisions: 1,
            legal_requests: 1,
            removed_content: 0,
            acceptances: 3,
            bug_logs: 0,
            ai_messages: 0,
        }
    );
    let left = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>(sql)
                .fetch_all(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        left("select reference from reports order by 1").await,
        ["R-NEW", "R-PRESERVED"]
    );
    assert_eq!(
        left("select reference from moderation_decisions").await,
        ["D-NEW"]
    );
    assert_eq!(
        left("select reference from legal_requests").await,
        ["LR-NEW"]
    );
    let mine: i64 = sqlx::query_scalar("select count(*) from legal_acceptances where user_id = $1")
        .bind(user)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mine, 3, "open accounts keep theirs");
    assert_eq!(
        slop_api::retention::purge(&app.db).await.unwrap(),
        Default::default()
    );
}

/// A terrorist content removal order is acted on in one step (CON-11): taken down as a decision
/// the maker can appeal, a sealed copy kept six months, the time it took recorded.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removal_orders_take_content_down_in_one_step(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Owner").await;
    let (maker, maker_cookie) = app.user("Mia Maker", "US").await;
    let (_, someone) = app.user("Sam", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    let received = (OffsetDateTime::now_utc() - time::Duration::minutes(20))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let order = |cookie: &str, subject: &str| {
        req(Method::POST, "/v1/admin/removal-orders")
            .cookie(cookie)
            .json(json!({
                "received_at": received, "country": "FR", "authority": "OCLCTIC",
                "order": "Order 2026-117: the store page of castle", "subject_kind": "app", "subject": subject
            }))
    };
    assert_eq!(
        app.send(order(&someone, "castle")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(order(&admin, "nothing")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(order(&admin, "castle")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let done = body_json(res).await;
    let minutes = done["minutes"].as_i64().unwrap();
    assert!((20..=22).contains(&minutes), "{done}");

    // Gone from the store, as a decision the maker sees and can appeal.
    let status: String = sqlx::query_scalar("select status::text from apps where slug = 'castle'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "delisted");
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker_cookie))
            .await,
    )
    .await;
    assert_eq!(decisions[0]["action"], "delist_app");
    assert!(decisions[0]["law"].as_str().unwrap().contains("2021/784"));
    assert_eq!(decisions[0]["can_appeal"], true);

    // Logged as answered, with a sealed copy for six months.
    let (status, response): (String, String) =
        sqlx::query_as("select status, response from legal_requests where reference = $1")
            .bind(done["reference"].as_str().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "answered");
    assert!(response.contains(&format!("{minutes} minutes")));
    let (title, months): (String, f64) = sqlx::query_as(
        "select snapshot->'app'->>'title', (extract(epoch from preserve_until - removed_at) / 2592000)::float8 from removed_content",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(title, "castle");
    assert!((5.9..6.2).contains(&months));
}

/// Ratings per release (AGE-7): each release carries its rating, the store page takes the
/// approved one, raising is immediate, and lowering waits for a person.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ratings_are_kept_per_release(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let submit = |version: &str, rating: Option<Value>| {
        let mut body = release_body(&app, version, "game-windows.zip");
        if let Some(r) = rating {
            body["rating"] = r;
        }
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };
    let page = || async {
        body_json(
            app.send(req(Method::GET, "/v1/apps/game").cookie(&maker))
                .await,
        )
        .await
    };

    // The first release says 12+ with violence; approving it puts that on the store page.
    let res = app
        .send(submit(
            "1.0.0",
            Some(json!({ "min_age": 12, "content_labels": ["violence"] })),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let first = body_json(res).await;
    assert_eq!(first["rating"]["min_age"], 12);
    assert!(releases::fetch_next(&app.state).await.unwrap());
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first["id"].as_str().unwrap().parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let p = page().await;
    assert_eq!(
        (&p["min_age"], &p["content_labels"]),
        (&json!(12), &json!(["violence"]))
    );
    assert_eq!(
        app.send(submit(
            "9.0.0",
            Some(json!({ "min_age": 13, "content_labels": [] }))
        ))
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a real rating"
    );

    // The store page can't quietly go lower; it can go higher.
    let patch = |body: Value| {
        req(Method::PATCH, "/v1/apps/game")
            .cookie(&maker)
            .json(body)
    };
    let res = app.send(patch(json!({ "min_age": 7 }))).await;
    assert_eq!(body_json(res).await["error"]["field"], "min_age");
    let res = app.send(patch(json!({ "content_labels": [] }))).await;
    assert_eq!(body_json(res).await["error"]["field"], "content_labels");
    assert_eq!(
        app.send(patch(json!({ "min_age": 16 }))).await.status(),
        StatusCode::OK
    );

    // An update that lowers it waits for a person, even where updates go out by themselves.
    let res = app
        .send(submit(
            "1.1.0",
            Some(json!({ "min_age": 7, "content_labels": [] })),
        ))
        .await;
    let lower: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    slop_api::updates::try_auto_approve(&app.state, lower)
        .await
        .unwrap();
    let (status, reason): (String, Option<String>) =
        sqlx::query_as("select status::text, status_reason from releases where id = $1")
            .bind(lower)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "awaiting_review");
    assert!(reason.unwrap().contains("age rating"));

    // A person approves it: the store page follows.
    admin::approve(&app.db, Some(&app.signing_key), lax(), lower, "rev")
        .await
        .unwrap();
    let p = page().await;
    assert_eq!(
        (&p["min_age"], &p["content_labels"]),
        (&json!(7), &json!([]))
    );

    // Left out, a release takes the store page's rating.
    let res = app.send(submit("1.2.0", None)).await;
    assert_eq!(
        body_json(res).await["rating"],
        json!({ "min_age": 7, "content_labels": [] })
    );
}

/// A self-hosted Forgejo (or Gitea) server nobody configured: Slop asks it what it is, then
/// reads the release's source from it (source_scan.rs `detect`).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn self_hosted_forges_are_recognised_and_read(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let forge = Router::new()
        .route(
            "/api/v1/version",
            get(|| async { axum::Json(json!({ "version": "11.0.0+gitea-1.22.0" })) }),
        )
        .route(
            "/api/v1/repos/forge/game/archive/{file}",
            get(|| async { source_archive(SCANNED_SOURCE) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, forge).await.unwrap() });
    let app_id = Uuid::now_v7();
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'forged', 'Forged', 'A game', 'game', 'beta', $2, $3, 'MIT', '{windows}')",
    )
    .bind(app_id)
    .bind(maker)
    .bind(format!("http://{addr}/forge/game"))
    .execute(&app.db)
    .await
    .unwrap();
    let release = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by)
         values ($1, $2, '1.0.0', repeat('c', 40), 'awaiting_review', $3)",
    )
    .bind(release)
    .bind(app_id)
    .bind(maker)
    .execute(&app.db)
    .await
    .unwrap();
    slop_api::source_scan::check_release(&app.state, release)
        .await
        .unwrap();
    let scan: Value = sqlx::query_scalar("select repo_scan from releases where id = $1")
        .bind(release)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(scan["status"], "checked", "{scan}");
    assert_eq!(scan["licence"]["declared"], "MIT");
}

/// Shared runtimes (slop-manifest runtimes.rs): only IDs from Slop's list, kept per release,
/// inherited when an update leaves them out, and signed into the manifest.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_name_shared_runtimes(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let submit = |version: &str, runtimes: Option<Value>| {
        let mut body = release_body(&app, version, "game-windows.zip");
        if let Some(r) = runtimes {
            body["runtimes"] = r;
        }
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };

    let res = app.send(req(Method::GET, "/v1/runtimes")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert!(list.to_string().contains("vcredist-x64"), "{list}");

    let res = app
        .send(submit("0.9.0", Some(json!(["vcredist-x64", "nope"]))))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "runtimes");

    let res = app
        .send(submit(
            "1.0.0",
            Some(json!(["vcredist-x64", "dotnet-desktop-8"])),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let first = body_json(res).await;
    assert_eq!(
        first["runtimes"],
        json!(["vcredist-x64", "dotnet-desktop-8"])
    );
    assert!(releases::fetch_next(&app.state).await.unwrap());
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first["id"].as_str().unwrap().parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let res = app
        .send(req(Method::GET, "/v1/apps/game/manifest").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let signed: slop_manifest::Signed = serde_json::from_value(body_json(res).await).unwrap();
    let manifest: slop_manifest::ReleaseManifest = slop_manifest::verify(
        &signed,
        slop_manifest::Purpose::ReleaseManifest,
        &[app.signing_key.verifying_key()],
    )
    .unwrap();
    assert_eq!(manifest.runtimes, ["vcredist-x64", "dotnet-desktop-8"]);

    // Left out, the update keeps them; an empty list says none.
    let res = app.send(submit("1.1.0", None)).await;
    assert_eq!(
        body_json(res).await["runtimes"],
        json!(["vcredist-x64", "dotnet-desktop-8"])
    );
    let res = app.send(submit("1.2.0", Some(json!([])))).await;
    assert_eq!(body_json(res).await["runtimes"], json!([]));
}

/// The AGPL's offer of source: the licence, where the code is, and the running commit when the
/// build knows it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_source_code_is_offered(db: PgPool) {
    let app = test_app(db).await;
    let res = app.send(req(Method::GET, "/v1/source")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["licence"], "AGPL-3.0-or-later");
    assert_eq!(body["url"], "https://github.com/slop/slop");
    // Tests aren't built by the deploy, so there's no commit to point at.
    assert_eq!(body["commit"], Value::Null);
    assert_eq!(body["version_url"], Value::Null);
}

/// "Does Slop know this file?" (docs/standard/file-verification.md): a signed answer for anyone,
/// naming public apps only, and revoked releases too.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn anyone_can_verify_a_file_by_its_hash(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let verify = |hash: String| {
        let app = &app;
        async move {
            let res = app
                .send(req(Method::GET, format!("/v1/verify/{hash}")))
                .await;
            assert_eq!(res.status(), StatusCode::OK);
            let signed: slop_manifest::Signed =
                serde_json::from_value(body_json(res).await).unwrap();
            slop_manifest::verify::<slop_manifest::FileVerification>(
                &signed,
                slop_manifest::Purpose::FileVerification,
                &[app.signing_key.verifying_key()],
            )
            .unwrap()
        }
    };

    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "game-windows.zip")),
        )
        .await;
    let release: Uuid = body_json(res).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let sha256: String =
        sqlx::query_scalar("select sha256 from release_files where release_id = $1")
            .bind(release)
            .fetch_one(&app.db)
            .await
            .unwrap();

    // Not approved yet: unknown.
    assert!(verify(sha256.clone()).await.matches.is_empty());

    admin::approve(&app.db, Some(&app.signing_key), lax(), release, "rev")
        .await
        .unwrap();
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    let answer = verify(sha256.to_ascii_uppercase()).await;
    assert_eq!(answer.sha256, sha256);
    assert_eq!(answer.matches.len(), 1);
    let m = &answer.matches[0];
    assert_eq!(
        (m.app_slug.as_str(), m.version.as_str(), m.status),
        ("game", "1.0.0", slop_manifest::MatchStatus::Approved)
    );
    assert_eq!(m.maker, "Maker");
    assert_eq!(m.scan, "not_scanned", "the test server doesn't scan");
    assert!(m.page_url.ends_with("/app/game"));
    assert!(!answer.is_expired(time::OffsetDateTime::now_utc()));

    // Revoked: still named, even once the app leaves the store.
    admin::revoke(&app.db, release, "rev", "Malware found by re-scan")
        .await
        .unwrap();
    sqlx::query("update apps set status = 'delisted' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let answer = verify(sha256.clone()).await;
    assert_eq!(
        answer.matches[0].status,
        slop_manifest::MatchStatus::Revoked
    );
    assert!(answer.matches[0].revoked_at.is_some());

    // A private app is never disclosed.
    sqlx::query("update apps set visibility = 'private' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(verify(sha256).await.matches.is_empty());

    assert!(verify("ab".repeat(32)).await.matches.is_empty());
    let res = app.send(req(Method::GET, "/v1/verify/not-a-hash")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Makers' AI agents (maker_agents.rs): a token does maker work and nothing else, within its
/// scope and apps, over REST and MCP; releases get notes; fixes and offers work for it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_does_maker_work_only(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let new_token = |body: Value| {
        req(Method::POST, "/v1/me/maker-tokens")
            .cookie(&maker)
            .json(body)
    };
    let res = app
        .send(new_token(
            json!({ "name": "Claude", "scope": "publish", "people": true }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    let secret = created["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("slopmk_"));
    // No ready-made command with the secret in it (the website shows how to keep it out of
    // AI chats).
    assert!(created.get("claude_command").is_none(), "{created}");
    assert_eq!(created["people"], true);
    let bearer = format!("Bearer {secret}");
    let agent = |m: Method, path: &str| req(m, path).header("authorization", bearer.clone());

    // Maker work, yes; anything else, no.
    let res = app.send(agent(Method::GET, "/v1/me/apps")).await;
    let status = res.status();
    assert_eq!(status, StatusCode::OK, "{}", body_json(res).await);
    for (m, path) in [
        (Method::GET, "/v1/me/export"),
        (Method::GET, "/v1/me/maker-tokens"),
        (Method::POST, "/v1/me/close"),
    ] {
        assert_eq!(
            app.send(agent(m, path)).await.status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }

    // The admin area doesn't exist for it.
    assert_eq!(
        app.send(agent(Method::GET, "/v1/admin/status"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // A release with notes, then too-long notes refused.
    let mut body = release_body(&app, "1.0.0", "game-windows.zip");
    body["notes"] = "First release: the whole game.".into();
    let res = app
        .send(agent(Method::POST, "/v1/apps/game/releases").json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["notes"], "First release: the whole game.");
    let id = release["id"].as_str().unwrap();
    let res = app
        .send(
            agent(Method::PATCH, &format!("/v1/apps/game/releases/{id}"))
                .json(json!({ "notes": "x".repeat(5001) })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Fixes for the newest release.
    let res = app.send(agent(Method::GET, "/v1/apps/game/fixes")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let fixes = body_json(res).await;
    assert_eq!(fixes["release"]["version"], "1.0.0");
    assert!(fixes["fixes"].is_array());

    // MCP: the tools, and calls through them.
    let rpc = |method: &str, params: Value| {
        agent(Method::POST, "/v1/maker/mcp")
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
    };
    let tools = body_json(app.send(rpc("tools/list", json!({}))).await).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in [
        "maker_guide",
        "get_fixes",
        "submit_release",
        "register_community_build",
        "send_quote",
    ] {
        assert!(names.contains(&want), "{want}");
    }
    let called = body_json(
        app.send(rpc(
            "tools/call",
            json!({ "name": "list_releases", "arguments": { "slug": "game" } }),
        ))
        .await,
    )
    .await;
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert!(
        called["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("First release")
    );
    let guide = body_json(
        app.send(rpc(
            "tools/call",
            json!({ "name": "publishing_guide", "arguments": { "section": "Saves and sandbox mode" } }),
        ))
        .await,
    )
    .await;
    assert!(
        guide["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("SLOP_SAVE_DIR")
    );
    // The website's own session can't drive the agent endpoint.
    let res = app
        .send(
            req(Method::POST, "/v1/maker/mcp")
                .cookie(&maker)
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // A read-only token for some apps only, without People access.
    let res = app
        .send(new_token(
            json!({ "name": "Watcher", "scope": "read", "apps": ["game"] }),
        ))
        .await;
    let read = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    let reader = |m: Method, path: &str| req(m, path).header("authorization", read.clone());
    assert_eq!(
        app.send(reader(Method::GET, "/v1/apps/game/releases"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(reader(Method::PATCH, "/v1/apps/game").json(json!({ "title": "X" })))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(reader(Method::GET, "/v1/me/jobs")).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = app
        .send(new_token(
            json!({ "name": "Bad", "scope": "publish", "apps": ["someone-elses"] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Revoked: signed out at once.
    let tokens = body_json(
        app.send(req(Method::GET, "/v1/me/maker-tokens").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(tokens.as_array().unwrap().len(), 2);
    let id = tokens[1]["id"].as_str().unwrap();
    assert_eq!(
        app.send(req(Method::DELETE, format!("/v1/me/maker-tokens/{id}")).cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(agent(Method::GET, "/v1/me/apps")).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

/// A community build offers its changes back to the original; the original's maker answers.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn community_builds_offer_fixes_to_the_original(db: PgPool) {
    let app = test_app(db).await;
    let (original_maker, original_cookie) = maker_with_app(&app, "released").await;
    let (fork_maker, fork_cookie) = app.user("Forker", "US").await;
    // Pull requests are https; the fake GitHub isn't.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms,
                           parent_app_id, branch_kind, branch_note)
         select $1, 'game-fix', 'Game (fixed)', 'Fixed', 'game', 'released', $2, 'https://github.com/forker/game',
                'MIT', '{windows}', id, 'fix', 'Fixes the jump' from apps where slug = 'game'",
    )
    .bind(Uuid::now_v7())
    .bind(fork_maker)
    .execute(&app.db)
    .await
    .unwrap();
    let offer = |url: &str| {
        req(Method::POST, "/v1/apps/game-fix/offers")
            .cookie(&fork_cookie)
            .json(json!({ "pull_request_url": url, "message": "Fixes the floaty jump." }))
    };
    assert_eq!(
        app.send(offer("https://github.com/evil/game/pull/1"))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = app.send(offer("https://github.com/octo/game/pull/7")).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    // The same pull request twice: no.
    assert_eq!(
        app.send(offer("https://github.com/octo/game/pull/7"))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // The original's maker sees it, and was told.
    let offers = body_json(
        app.send(req(Method::GET, "/v1/apps/game/offers").cookie(&original_cookie))
            .await,
    )
    .await;
    assert_eq!(offers[0]["direction"], "received");
    assert_eq!(offers[0]["branch_slug"], "game-fix");
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%offered you a fix'",
    )
    .bind(original_maker)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);
    // Only the original's maker answers.
    let answer = |cookie: &str| {
        req(Method::POST, format!("/v1/apps/game/offers/{id}/answer"))
            .cookie(cookie)
            .json(json!({ "decision": "accepted", "message": "Merged, thanks!" }))
    };
    assert_eq!(
        app.send(answer(&fork_cookie)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(answer(&original_cookie)).await.status(),
        StatusCode::NO_CONTENT
    );
    let offers = body_json(
        app.send(req(Method::GET, "/v1/apps/game-fix/offers").cookie(&fork_cookie))
            .await,
    )
    .await;
    assert_eq!(
        (
            offers[0]["direction"].as_str(),
            offers[0]["status"].as_str()
        ),
        (Some("sent"), Some("accepted"))
    );
}

/// The app's maker replies under a review; the reviewer is told; staff can remove a reply, and
/// then the maker can't post it again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_reply_to_reviews(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (player_id, player) = app.user("Pat Player", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(&player))
        .await;
    app.send(
        req(Method::POST, "/v1/apps/tower/vote")
            .cookie(&player)
            .json(json!({ "vote": "slop" })),
    )
    .await;
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/tower/review")
                .cookie(&player)
                .json(json!({ "body": "Too grindy after level 5." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/reviews").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(page["can_reply"], true);
    let id = page["reviews"][0]["id"].as_str().unwrap().to_string();
    let reply = |cookie: &str, body: &str| {
        req(Method::PUT, format!("/v1/apps/tower/reviews/{id}/reply"))
            .cookie(cookie)
            .json(json!({ "body": body }))
    };

    // Only the maker replies.
    assert_eq!(
        app.send(reply(&player, "Me too")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(reply(&maker, "Thanks! 1.2 halves the grind."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(page["can_reply"], false);
    let r = &page["reviews"][0];
    assert_eq!(r["reply"], "Thanks! 1.2 halves the grind.");
    assert_eq!(r["reply_outdated"], false);
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%replied to your review'",
    )
    .bind(player_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);

    // The reviewer changes the review later: the reply is marked as about an earlier version.
    sqlx::query("update app_reviews set updated_at = now() + interval '1 hour' where id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(page["reviews"][0]["reply_outdated"], true);

    // Staff remove the reply: the decision is about the maker, and the reply can't come back.
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&player)
                .json(json!({
                    "kind": "app_review", "subject": id, "reason": "harassment",
                    "details": "The maker's reply insults me.", "good_faith": true
                })),
        )
        .await;
    let status = res.status();
    assert_eq!(status, StatusCode::OK, "{}", body_json(res).await);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    let report_id = queue["reports"][0]["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{report_id}/act")).cookie(&admin).json(json!({
                "action": "remove_reply", "rule": "harassment", "facts": "The reply insults the reviewer.",
                "outcome": "The maker's reply was removed."
            })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let about: Uuid = sqlx::query_scalar(
        "select user_id from moderation_decisions where action = 'remove_reply'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(about, maker_id);
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tower/reviews")).await).await;
    assert_eq!(page["reviews"][0]["reply"], Value::Null);
    assert_eq!(
        page["reviews"][0]["body"], "Too grindy after level 5.",
        "the review stays"
    );
    assert_eq!(
        app.send(reply(&maker, "Sorry.")).await.status(),
        StatusCode::CONFLICT
    );
}

/// Phase 11 (label.rs): the "what this app does" label, accessibility features players confirm,
/// the accessibility filter, and the "New and barely seen" and "Glow-ups" shelves.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn labels_accessibility_and_fair_shelves(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (_, player) = app.user("Pat Player", "US").await;

    // The maker declares features; players who played confirm or dispute them.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/tower")
                .cookie(&maker)
                .json(json!({ "accessibility": ["subtitles", "remap_controls"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let bad = app
        .send(
            req(Method::PATCH, "/v1/apps/tower")
                .cookie(&maker)
                .json(json!({ "accessibility": ["telepathy"] })),
        )
        .await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let confirm = |cookie: &str, feature: &str, works: bool| {
        req(
            Method::PUT,
            format!("/v1/apps/tower/accessibility/{feature}"),
        )
        .cookie(cookie)
        .json(json!({ "works": works }))
    };
    assert_eq!(
        app.send(confirm(&player, "subtitles", true)).await.status(),
        StatusCode::CONFLICT,
        "not played yet"
    );
    app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(&player))
        .await;
    assert_eq!(
        app.send(confirm(&player, "subtitles", true)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(confirm(&player, "colorblind", true))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "not declared"
    );
    assert_eq!(
        app.send(confirm(&maker, "subtitles", true)).await.status(),
        StatusCode::CONFLICT,
        "not the maker"
    );

    let label = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/label").cookie(&player))
            .await,
    )
    .await;
    assert_eq!(label["can_report"], true);
    let subtitles = label["accessibility"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "subtitles")
        .unwrap()
        .clone();
    assert_eq!(
        (subtitles["confirmed"].as_i64(), subtitles["mine"].as_bool()),
        (Some(1), Some(true))
    );
    assert!(
        label["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["key"] == "licence")
    );
    assert!(!label.to_string().to_lowercase().contains("safe"));

    // The store filters by accessibility.
    let found = |q: &str| {
        let app = &app;
        let q = q.to_string();
        async move {
            let page = body_json(
                app.send(req(Method::GET, format!("/v1/apps?accessibility={q}")))
                    .await,
            )
            .await;
            page["apps"]
                .as_array()
                .map(|a| a.iter().any(|x| x["slug"] == "tower"))
        }
    };
    assert_eq!(found("subtitles,remap_controls").await, Some(true));
    assert_eq!(found("colorblind").await, Some(false));
    let res = app
        .send(req(Method::GET, "/v1/apps?accessibility=telepathy"))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // New and barely seen: released lately, in few libraries.
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps?sort=barely_seen"))
            .await,
    )
    .await;
    assert!(
        page["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["slug"] == "tower")
    );
    sqlx::query(
        "update apps set stage_entered_at = now() - interval '90 days' where slug = 'tower'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps?sort=barely_seen"))
            .await,
    )
    .await;
    assert!(
        !page["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["slug"] == "tower")
    );

    // Glow-ups: 2 of 6 Not Slop 90 days ago, 5 of 6 now.
    let tower: Uuid = sqlx::query_scalar("select id from apps where slug = 'tower'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for i in 0..6 {
        let (voter, _) = app.user(&format!("Voter {i}"), "US").await;
        let (then, now) = (if i < 2 { 1i16 } else { -1 }, if i < 5 { 1i16 } else { -1 });
        sqlx::query(
            "insert into vote_events (id, user_id, app_id, vote, weight, created_at) values ($1, $2, $3, $4, 1, now() - interval '90 days')",
        )
        .bind(Uuid::now_v7())
        .bind(voter)
        .bind(tower)
        .bind(then)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query("insert into votes (user_id, app_id, vote, weight) values ($1, $2, $3, 1)")
            .bind(voter)
            .bind(tower)
            .bind(now)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let glow = body_json(app.send(req(Method::GET, "/v1/shelves/glow-ups")).await).await;
    assert_eq!(glow[0]["slug"], "tower", "{glow}");
    assert_eq!(
        (glow[0]["before"].as_i64(), glow[0]["after"].as_i64()),
        (Some(33), Some(83))
    );
}

/// An approved, served release for an app, without the fetch and review steps.
async fn approved_release(app: &TestApp, slug: &str, by: Uuid) {
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, signed_manifest, reviewed_at)
         select $1, id, '1.0.0', repeat('a', 40), 'approved', $2, '{}'::jsonb, now() from apps where slug = $3",
    )
    .bind(Uuid::now_v7())
    .bind(by)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Fix bounties (bounties.rs), simulated payments: pledging, a community build delivering, the
/// app's maker confirming, every pledge charged with Slop's 5%; the backers deciding the maker's
/// own delivery; withdrawing; staff hiding one; expiry.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bounties_pay_whoever_delivers(db: PgPool) {
    let app = test_app_with(db, |c| c.payments.simulated = true).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
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
    let (_, ann) = app.user("Ann Backer", "US").await;
    let (_, bob) = app.user("Bob Backer", "US").await;

    let open = |cookie: &str, title: &str| {
        req(Method::POST, "/v1/apps/tower/bounties")
            .cookie(cookie)
            .json(json!({ "title": title, "body": "Saves vanish when the game crashes.", "currency": "usd" }))
    };
    let res = app.send(open(&ann, "Fix lost saves")).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let pledge = |cookie: &str, id: &str, amount: i64| {
        req(Method::POST, format!("/v1/bounties/{id}/pledge"))
            .cookie(cookie)
            .json(json!({ "amount_minor": amount, "confirm_adult": true }))
    };
    assert_eq!(
        app.send(pledge(&ann, &id, 50)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        body_json(app.send(pledge(&ann, &id, 1000)).await).await["status"],
        "active"
    );
    assert_eq!(
        app.send(pledge(&ann, &id, 1000)).await.status(),
        StatusCode::CONFLICT,
        "one each"
    );
    app.send(pledge(&bob, &id, 3000)).await;
    let list = body_json(app.send(req(Method::GET, "/v1/bounties")).await).await;
    assert_eq!(
        (
            list[0]["backers"].as_i64(),
            list[0]["pledged_minor"].as_i64()
        ),
        (Some(2), Some(4000))
    );

    // A community build delivers; the app's maker confirms; each pledge is charged.
    let claim = |cookie: &str, id: &str, slug: &str| {
        req(Method::POST, format!("/v1/bounties/{id}/claims"))
            .cookie(cookie)
            .json(json!({ "app_slug": slug, "note": "1.0.0 writes saves safely." }))
    };
    assert_eq!(
        app.send(claim(&ann, &id, "tower-fix")).await.status(),
        StatusCode::NOT_FOUND,
        "not theirs"
    );
    assert_eq!(
        app.send(claim(&forker, &id, "tower-fix")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(claim(&forker, &id, "tower-fix")).await.status(),
        StatusCode::CONFLICT
    );
    let detail = body_json(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(detail["is_app_maker"], true);
    let claim_id = detail["claims"][0]["id"].as_str().unwrap().to_string();
    let answer = |cookie: &str, decision: &str| {
        req(
            Method::POST,
            format!("/v1/bounties/{id}/claims/{claim_id}/answer"),
        )
        .cookie(cookie)
        .json(json!({ "decision": decision }))
    };
    assert_eq!(
        app.send(answer(&ann, "accepted")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(answer(&maker, "accepted")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 2);
    let detail = body_json(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")))
            .await,
    )
    .await;
    assert_eq!(detail["status"], "paid");
    let paid: Vec<(i32, i32, String)> = sqlx::query_as(
        "select amount_minor, slop_fee_minor, stripe_account_id from payments where kind = 'bounty' order by amount_minor",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(paid.len(), 2);
    assert_eq!(
        (paid[0].0, paid[0].1, paid[1].0, paid[1].1),
        (1000, 50, 3000, 150)
    );
    assert!(paid[0].2.starts_with("acct_sim_"));
    // A paid bounty takes no more pledges or claims.
    assert_eq!(
        app.send(pledge(&bob, &id, 500)).await.status(),
        StatusCode::CONFLICT
    );

    // The maker delivers their own fix: the backers decide, by amount pledged.
    let res = app.send(open(&ann, "Add a colourblind mode")).await;
    let id2 = body_json(res).await["id"].as_str().unwrap().to_string();
    app.send(pledge(&ann, &id2, 1000)).await;
    app.send(pledge(&bob, &id2, 3000)).await;
    // The maker backs it too, then delivers it: their own pledge never counts in the vote.
    assert_eq!(
        app.send(pledge(&maker, &id2, 50000)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(claim(&maker, &id2, "tower")).await.status(),
        StatusCode::NO_CONTENT
    );
    // And nobody can pledge while it waits for a decision (a pledge then could buy the vote).
    assert_eq!(
        app.send(pledge(&forker, &id2, 50000)).await.status(),
        StatusCode::CONFLICT
    );
    let detail = body_json(
        app.send(req(Method::GET, format!("/v1/bounties/{id2}")).cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(detail["can_vote"], true);
    let claim2 = detail["claims"][0]["id"].as_str().unwrap().to_string();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/bounties/{id2}/claims/{claim2}/answer"),
            )
            .cookie(&maker)
            .json(json!({ "decision": "accepted" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "not their own");
    let vote = |cookie: &str, accept: bool| {
        req(
            Method::POST,
            format!("/v1/bounties/{id2}/claims/{claim2}/vote"),
        )
        .cookie(cookie)
        .json(json!({ "accept": accept }))
    };
    assert_eq!(
        app.send(vote(&maker, true)).await.status(),
        StatusCode::CONFLICT,
        "not on their own delivery"
    );
    assert_eq!(
        app.send(vote(&ann, true)).await.status(),
        StatusCode::NO_CONTENT
    );
    let status = |id: String| {
        let app = &app;
        async move {
            body_json(
                app.send(req(Method::GET, format!("/v1/bounties/{id}")))
                    .await,
            )
            .await["status"]
                .clone()
        }
    };
    assert_eq!(
        status(id2.clone()).await,
        "claimed",
        "1,000 of 4,000 isn't more than half"
    );
    assert_eq!(
        app.send(vote(&bob, true)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(status(id2.clone()).await, "paying");

    // Withdrawing a pledge, and staff hiding a bounty.
    let res = app.send(open(&bob, "Spam spam spam")).await;
    let id3 = body_json(res).await["id"].as_str().unwrap().to_string();
    app.send(pledge(&ann, &id3, 500)).await;
    assert_eq!(
        app.send(req(Method::DELETE, format!("/v1/bounties/{id3}/pledge")).cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(&ann)
                .json(json!({ "kind": "bounty", "subject": id3, "reason": "spam", "details": "Not a real request.", "good_faith": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(queue["reports"][0]["link"], format!("/bounties/{id3}"));
    let report_id = queue["reports"][0]["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{report_id}/act")).cookie(&admin).json(json!({
                "action": "hide_bounty", "rule": "spam", "facts": "Not a bounty.", "outcome": "Hidden."
            })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bounties/{id3}")))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Expiry: nobody delivered in time; the cards are released.
    let res = app.send(open(&bob, "Port to Linux")).await;
    let id4 = body_json(res).await["id"].as_str().unwrap().to_string();
    app.send(pledge(&ann, &id4, 700)).await;
    sqlx::query("update bounties set expires_at = now() - interval '1 day' where id = $1")
        .bind(Uuid::parse_str(&id4).unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(status(id4.clone()).await, "expired");
    let released: String =
        sqlx::query_scalar("select status from bounty_pledges where bounty_id = $1")
            .bind(Uuid::parse_str(&id4).unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(released, "released");
}

/// With Stripe: a pledge saves the card on Slop's account (Checkout in setup mode); a confirmed
/// delivery clones it to the builder's own Stripe account and charges it there, off-session,
/// with Slop's 5% as the application fee.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bounty_pledges_are_charged_on_the_builders_stripe_account(db: PgPool) {
    use std::sync::{Arc, Mutex};
    // (which call, the Stripe account it was on, the form sent)
    type Seen = Arc<Mutex<Vec<(String, Option<String>, String)>>>;
    let seen: Seen = Arc::default();
    let record = |path: &'static str, reply: Value| {
        let seen = seen.clone();
        move |headers: axum::http::HeaderMap, body: String| {
            let seen = seen.clone();
            let reply = reply.clone();
            async move {
                assert_eq!(headers["authorization"], "Bearer sk_test_slop");
                let account = headers
                    .get("stripe-account")
                    .map(|v| v.to_str().unwrap().to_string());
                seen.lock().unwrap().push((path.to_string(), account, body));
                axum::Json(reply)
            }
        }
    };
    let fake = Router::new()
        .route("/v1/customers", post(record("customers", json!({ "id": "cus_1" }))))
        .route(
            "/v1/checkout/sessions",
            post(record("sessions", json!({ "id": "cs_b1", "url": "https://checkout.stripe.example/cs_b1" }))),
        )
        .route(
            "/v1/checkout/sessions/{id}",
            get(|| async { axum::Json(json!({ "id": "cs_b1", "status": "complete", "setup_intent": { "payment_method": "pm_1" } })) }),
        )
        .route("/v1/payment_methods", post(record("clone", json!({ "id": "pm_clone" }))))
        .route(
            "/v1/payment_intents",
            post(record("charge", json!({ "id": "pi_1", "status": "succeeded" }))),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let app = test_app_with(db, |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: "whsec_slop".into(),
            api_base: base.parse().unwrap(),
        })
    })
    .await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (fixer_id, fixer) = app.user("Vera Fixer", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx,
                           parent_app_id, branch_kind, branch_note)
         select $1, 'tower-fix', 'Tower (fixed)', 'Fixed', 'game', 'released', 'listed', $2, 'https://github.com/f/t',
                'MIT', id, 'fix', 'Fixes saves' from apps where slug = 'tower'",
    )
    .bind(Uuid::now_v7())
    .bind(fixer_id)
    .execute(&app.db)
    .await
    .unwrap();
    approved_release(&app, "tower-fix", fixer_id).await;
    let (_, ann) = app.user("Ann Backer", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/tower/bounties")
                .cookie(&ann)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "eur" })),
        )
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let started = body_json(
        app.send(
            req(Method::POST, format!("/v1/bounties/{id}/pledge"))
                .cookie(&ann)
                .json(json!({ "amount_minor": 2000, "confirm_adult": true })),
        )
        .await,
    )
    .await;
    assert_eq!(started["url"], "https://checkout.stripe.example/cs_b1");
    assert_eq!(started["status"], "saving");
    // Back from Checkout: the card is saved.
    app.send(req(Method::POST, format!("/v1/bounties/{id}/pledge/check")).cookie(&ann))
        .await;
    let mine = body_json(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")).cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(mine["my_pledge"]["status"], "active");

    // Delivering needs a Stripe account to be paid to.
    let claim = || {
        req(Method::POST, format!("/v1/bounties/{id}/claims"))
            .cookie(&fixer)
            .json(json!({ "app_slug": "tower-fix", "note": "Fixed in 1.0.0." }))
    };
    assert_eq!(app.send(claim()).await.status(), StatusCode::CONFLICT);
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_vera', 'active', now())",
    )
    .bind(fixer_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(app.send(claim()).await.status(), StatusCode::NO_CONTENT);
    let detail = body_json(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")).cookie(&maker))
            .await,
    )
    .await;
    let claim_id = detail["claims"][0]["id"].as_str().unwrap();
    app.send(
        req(
            Method::POST,
            format!("/v1/bounties/{id}/claims/{claim_id}/answer"),
        )
        .cookie(&maker)
        .json(json!({ "decision": "accepted" })),
    )
    .await;
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);

    let seen = seen.lock().unwrap().clone();
    let find = |p: &str| seen.iter().find(|(path, ..)| path == p).unwrap().clone();
    let (_, account, body) = find("sessions");
    assert_eq!(account, None, "the card is saved on Slop's own account");
    assert!(
        body.contains("mode=setup")
            && body.contains("customer=cus_1")
            && body.contains("currency=eur"),
        "{body}"
    );
    let (_, account, body) = find("clone");
    assert_eq!(account.as_deref(), Some("acct_vera"));
    assert!(body.contains("payment_method=pm_1"), "{body}");
    let (_, account, body) = find("charge");
    assert_eq!(
        account.as_deref(),
        Some("acct_vera"),
        "a direct charge on the builder's account"
    );
    for part in [
        "amount=2000",
        "currency=eur",
        "payment_method=pm_clone",
        "off_session=true",
        "application_fee_amount=100",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }
    let (object, amount, slop_fee, simulated): (String, i32, i32, bool) = sqlx::query_as(
        "select stripe_object_id, amount_minor, slop_fee_minor, simulated from payments where kind = 'bounty'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (object.as_str(), amount, slop_fee, simulated),
        ("pi_1", 2000, 100, false)
    );
}

/// Feedback for makers (bug_reports.rs): players who played file bug reports, others add "me
/// too", makers see them in full and mark them fixed; feedback per release; crash logs go after
/// 90 days.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bug_reports_and_feedback_per_release(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (sam_id, sam) = app.user("Sam Player", "US").await;
    let report = |cookie: &str| {
        req(Method::POST, "/v1/apps/tower/bug-reports")
            .cookie(cookie)
            .json(json!({
                "title": "Crash when the wave starts", "body": "Wave 3 always crashes.",
                "version": "1.0.0", "platform": "windows", "log": "panic at wave.rs:12"
            }))
    };
    assert_eq!(
        app.send(report(&pat)).await.status(),
        StatusCode::CONFLICT,
        "hasn't played"
    );
    for c in [&pat, &sam] {
        app.send(req(Method::POST, "/v1/apps/tower/launch").cookie(c))
            .await;
    }
    let res = app.send(report(&pat)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like 'A bug report on%'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);

    // Another player sees the open report's title, not its details, and says "me too".
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/bug-reports").cookie(&sam))
            .await,
    )
    .await;
    assert_eq!(theirs["reports"][0]["title"], "Crash when the wave starts");
    assert!(
        theirs["reports"][0].get("log").is_none() && theirs["reports"][0].get("body").is_none()
    );
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/bug-reports/{id}/me-too")).cookie(&sam))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/bug-reports/{id}/me-too")).cookie(&pat))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bug-reports/{id}")).cookie(&sam))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // The maker sees it all, and marks it fixed: both players are told.
    let all = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/bug-reports").cookie(&maker))
            .await,
    )
    .await;
    let r = &all["reports"][0];
    assert_eq!(
        (
            r["log"].as_str(),
            r["reporter"].as_str(),
            r["me_too"].as_i64()
        ),
        (Some("panic at wave.rs:12"), Some("Pat Player"), Some(1))
    );
    let fix = |cookie: &str| {
        req(Method::PATCH, format!("/v1/bug-reports/{id}"))
            .cookie(cookie)
            .json(json!({ "status": "fixed", "maker_note": "Fixed in 1.0.1." }))
    };
    assert_eq!(app.send(fix(&pat)).await.status(), StatusCode::NOT_FOUND);
    let res = app.send(fix(&maker)).await;
    assert_eq!(res.status(), StatusCode::OK);
    for user in [pat_id, sam_id] {
        let n: i64 = sqlx::query_scalar("select count(*) from notifications where user_id = $1 and title = 'Your bug report was fixed'")
            .bind(user)
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(n, 1);
    }
    let mine = body_json(
        app.send(req(Method::GET, format!("/v1/bug-reports/{id}")).cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(mine["maker_note"], "Fixed in 1.0.1.");

    // The maker can turn reports off.
    app.send(
        req(Method::PATCH, "/v1/apps/tower")
            .cookie(&maker)
            .json(json!({ "bug_reports": false })),
    )
    .await;
    assert_eq!(app.send(report(&sam)).await.status(), StatusCode::CONFLICT);

    // Feedback per release: votes on 1.0.0, and what its reviews say.
    let tower: Uuid = sqlx::query_scalar("select id from apps where slug = 'tower'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for (user, vote, text) in [
        (
            pat_id,
            1i16,
            "The new jump feels floaty but the music is great.",
        ),
        (sam_id, 1, "Great music, and the jump feels floaty."),
    ] {
        sqlx::query("insert into vote_events (id, user_id, app_id, vote, weight, release_version) values ($1, $2, $3, $4, 1, '1.0.0')")
            .bind(Uuid::now_v7())
            .bind(user)
            .bind(tower)
            .bind(vote)
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("insert into votes (user_id, app_id, vote, weight) values ($1, $2, $3, 1)")
            .bind(user)
            .bind(tower)
            .bind(vote)
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("insert into app_reviews (id, user_id, app_id, body, updated_at) values ($1, $2, $3, $4, now() + interval '1 minute')")
            .bind(Uuid::now_v7())
            .bind(user)
            .bind(tower)
            .bind(text)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let feedback = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/release-feedback").cookie(&maker))
            .await,
    )
    .await;
    let f = &feedback[0];
    assert_eq!(
        (
            f["version"].as_str(),
            f["votes"].as_i64(),
            f["not_slop_percent"].as_i64()
        ),
        (Some("1.0.0"), Some(2), Some(100))
    );
    assert_eq!(
        (f["reviews_up"].as_i64(), f["bug_reports"].as_i64()),
        (Some(2), Some(1))
    );
    let praised: Vec<&str> = f["praised"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p[0].as_str().unwrap())
        .collect();
    assert!(
        praised.contains(&"feels floaty") && praised.contains(&"music"),
        "{praised:?}"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/tower/release-feedback").cookie(&pat))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Crash logs go after 90 days; the report stays.
    sqlx::query("update bug_reports set created_at = now() - interval '91 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        slop_api::retention::purge(&app.db).await.unwrap().bug_logs,
        1
    );
    let log: Option<String> = sqlx::query_scalar("select log from bug_reports")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(log, None);
}

/// Save sync (save_sync.rs): encrypted saves stored per player and app, newest two kept; an
/// upload on top of an old generation is refused; 20 MB at most; only the player's own.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saves_sync_without_silent_overwrites(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let (_, pat) = app.user("Pat Player", "US").await;
    let (_, sam) = app.user("Sam Player", "US").await;
    let put = |cookie: &str, base: i32, body: Vec<u8>| {
        Request::builder()
            .method(Method::PUT)
            .uri("/v1/me/saves/tower")
            .header(header::HOST, "localhost:8080")
            .header(header::COOKIE, cookie)
            .header("x-slop-base", base.to_string())
            .header("x-slop-device", "d-home")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(body))
            .unwrap()
    };
    let send = |r: Request<Body>| {
        let router = app.router.clone();
        async move { router.oneshot(r).await.unwrap() }
    };
    let res = send(put(&pat, 0, b"sealed-1".to_vec())).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["generation"], 1);
    // Another computer still on generation 0: refused, nothing lost.
    assert_eq!(
        send(put(&pat, 0, b"sealed-x".to_vec())).await.status(),
        StatusCode::CONFLICT
    );
    for (base, body) in [(1, "sealed-2"), (2, "sealed-3")] {
        assert_eq!(
            send(put(&pat, base, body.as_bytes().to_vec()))
                .await
                .status(),
            StatusCode::OK
        );
    }
    let kept: Vec<i32> =
        sqlx::query_scalar("select generation from save_blobs order by generation")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(kept, [2, 3], "the newest and the one before");

    let res = app
        .send(req(Method::GET, "/v1/me/saves/tower").cookie(&pat))
        .await;
    assert_eq!(res.headers()["x-slop-generation"], "3");
    assert_eq!(res.headers()["x-slop-device"], "d-home");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"sealed-3");
    let res = app
        .send(req(Method::GET, "/v1/me/saves/tower?generation=2").cookie(&pat))
        .await;
    assert_eq!(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap()[..],
        b"sealed-2"
    );
    // Someone else's saves aren't theirs.
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/saves/tower").cookie(&sam))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/saves").cookie(&pat))
            .await,
    )
    .await;
    assert_eq!(
        (
            list["saves"][0]["generation"].as_i64(),
            list["used"].as_i64()
        ),
        // The newest (8 bytes) and the copy kept before it (8) both count.
        (Some(3), Some(16))
    );

    // Over 20 MB: refused.
    let big = send(put(&pat, 3, vec![0u8; 21 * 1024 * 1024])).await;
    assert!(
        big.status() == StatusCode::PAYLOAD_TOO_LARGE
            || big.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        big.status()
    );

    // Stopping sync deletes them from Slop.
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/me/saves/tower").cookie(&pat))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/saves/tower").cookie(&pat))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

/// A tiny web build (an HTML5 export) for browser demos.
fn web_build() -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        (
            "web/index.html",
            "<!doctype html><script src=game.js></script>",
        ),
        ("web/game.js", "console.log('pong')"),
    ] {
        w.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(data.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// Browser demos (demos.rs): a new game's web build, scanned, unpacked and served from the play
/// origin only, with its own headers; not for apps released too long ago; ended and deleted.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn new_games_get_a_browser_demo_for_a_while(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-demos-test-{}", rand::random::<u64>()));
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
    let url = format!("{}octo/game/releases/download/v1/web.zip", app.fake_base);
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/demo")
                .cookie(&maker)
                .json(json!({ "url": url })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "{}",
        body_json(res).await
    );
    let mut status = Value::Null;
    for _ in 0..100 {
        status = body_json(
            app.send(req(Method::GET, "/v1/apps/game/demo").cookie(&maker))
                .await,
        )
        .await;
        if status["demo"]["status"] != "fetching" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    assert_eq!(status["demo"]["status"], "live", "{status}");
    assert_eq!(status["play_url"], "http://play.localhost:8080/play/game/");
    assert_eq!(
        status["demo"]["scan_result"], "skipped",
        "this test server doesn't scan"
    );

    let get = |host: &'static str, path: &str| {
        let router = app.router.clone();
        let path = path.to_string();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header::HOST, host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let res = get("play.localhost:8080", "/play/game/").await;
    assert_eq!(res.status(), StatusCode::OK);
    let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("connect-src 'self'") && csp.contains("wasm-unsafe-eval"),
        "{csp}"
    );
    assert_eq!(
        res.headers()["cross-origin-embedder-policy"],
        "require-corp"
    );
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("game.js"));
    assert_eq!(
        get("play.localhost:8080", "/play/game/game.js")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get("play.localhost:8080", "/play/game/../secret")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Never on Slop's own origin; nothing but demos on the play origin.
    assert_eq!(
        get("localhost:8080", "/play/game/").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("play.localhost:8080", "/v1/apps").await.status(),
        StatusCode::NOT_FOUND
    );
    let plays: i32 = sqlx::query_scalar("select plays from demos")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(plays, 1);

    // Past its days: ended and deleted.
    sqlx::query("update demos set ends_at = now() - interval '1 minute'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::demos::expire(&app.state).await.unwrap(), 1);
    assert!(!dir.join("game").exists());
    assert_eq!(
        get("play.localhost:8080", "/play/game/").await.status(),
        StatusCode::NOT_FOUND
    );
    // Released long ago: no demo.
    sqlx::query(
        "update apps set stage_entered_at = now() - interval '30 days' where slug = 'game'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/demo")
                .cookie(&maker)
                .json(json!({ "url": url })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_can_pause_an_apps_donations_and_the_maker_can_ask_for_them_back(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 1000;
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (donor_id, donor) = app.user("Dee Donor", "US").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (reviewer_id, reviewer) = app.user("Rex Reviewer", "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(reviewer_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'reviewer', 'test')",
    )
    .bind(reviewer_id)
    .execute(&app.db)
    .await
    .unwrap();
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", true).await;
    let donate = |body: Value| {
        req(Method::POST, "/v1/apps/arena/donate")
            .cookie(&donor)
            .json(body)
    };
    let monthly =
        json!({ "amount_minor": 500, "monthly": true, "consent": true, "confirm_adult": true });
    assert_eq!(
        body_json(app.send(donate(monthly)).await).await["done"],
        true
    );

    // Only staff, and only those who look after money or moderation.
    let hold = |cookie: &str, body: Value| {
        req(Method::POST, "/v1/admin/apps/arena/donation-hold")
            .cookie(cookie)
            .json(body)
    };
    let review = json!({ "kind": "review", "reason": "A trademark complaint says the name is theirs.", "ground": "Complaint TM-1" });
    assert_eq!(
        app.send(hold(&donor, review.clone())).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(hold(&reviewer, review.clone())).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(hold(&owner, json!({ "kind": "review", "reason": " " })))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(hold(&owner, review)).await.status(),
        StatusCode::OK
    );

    // Donors can't donate and see only that it's paused; the app stays in the store.
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(info["on"], false);
    assert_eq!(info["off_reason"], "Donations to this app are paused.");
    assert!(!info.to_string().contains("trademark"));
    assert_eq!(
        app.send(donate(
            json!({ "amount_minor": 300, "confirm_adult": true })
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena")).await.status(),
        StatusCode::OK
    );

    // The monthly donor isn't charged while it's paused.
    sqlx::query("update server_donations set next_charge_at = now() - interval '1 day'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::donations::hourly(&app.state).await.unwrap();
    let charges: i64 = sqlx::query_scalar("select count(*) from payments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(charges, 1, "only the first month");
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%is paused'",
    )
    .bind(donor_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);

    // The maker is told why and asks for a review once they've fixed it; one at a time.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donations/hold").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine["open"]["kind"], "review");
    assert_eq!(
        mine["open"]["reason"],
        "A trademark complaint says the name is theirs."
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donations/hold").cookie(&donor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let ask = |message: &str| {
        req(Method::POST, "/v1/apps/arena/donations/hold/request")
            .cookie(&maker)
            .json(json!({ "message": message }))
    };
    assert_eq!(
        app.send(ask("Renamed it to Brawl Pit.")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.send(ask("Again?")).await.status(), StatusCode::CONFLICT);

    // Staff turn it off instead (a sensitive action), keep it with an answer…
    assert_eq!(
        app.send(hold(
            &owner,
            json!({ "kind": "off", "reason": "The old name is still in the window title." })
        ))
        .await
        .status(),
        StatusCode::OK
    );
    let holds = body_json(
        app.send(req(Method::GET, "/v1/admin/donation-holds").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(holds.as_array().unwrap().len(), 1);
    assert_eq!(holds[0]["kind"], "off");
    let request_id = holds[0]["requests"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/donation-hold-requests/{request_id}/answer")
            )
            .cookie(&owner)
            .json(json!({ "lift": false, "answer": "The window title still says Arena." }))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donations/hold").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine["open"]["requests"][0]["outcome"], "kept");
    assert!(mine["can_ask_at"].is_string(), "a day before asking again");

    // …then lift it: donations are back, and the monthly donor's next charge is a month on.
    let hold_id = holds[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/donation-holds/{hold_id}/lift")
            )
            .cookie(&reviewer)
            .json(json!({ "note": "Fixed." }))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/donation-holds/{hold_id}/lift")
            )
            .cookie(&owner)
            .json(json!({ "note": "Renamed everywhere now. Thanks!" }))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let info = body_json(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await,
    )
    .await;
    assert_eq!(info["on"], true);
    let later: bool = sqlx::query_scalar(
        "select next_charge_at > now() + interval '29 days' from server_donations",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(later);
    let actions: Vec<String> = sqlx::query_scalar(
        "select action from audit_log where action like 'donations.hold%' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "donations.hold_placed",
            "donations.hold_review_asked",
            "donations.hold_changed",
            "donations.hold_kept",
            "donations.hold_lifted"
        ]
    );
    let export = body_json(
        app.send(req(Method::GET, "/v1/me/export").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        export["donation_holds"][0]["requests"][0]["outcome"],
        "kept"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_see_an_apps_whole_record_and_can_fix_its_store_page(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (reviewer_id, reviewer) = app.user("Rex Reviewer", "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(reviewer_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'reviewer', 'test')",
    )
    .bind(reviewer_id)
    .execute(&app.db)
    .await
    .unwrap();
    listed_with(&app, maker_id, "arena", "released", "").await;

    // Search finds it by title, address or maker.
    for q in ["aren", "Mia"] {
        let found = body_json(
            app.send(req(Method::GET, format!("/v1/admin/apps?q={q}")).cookie(&owner))
                .await,
        )
        .await;
        assert_eq!(found[0]["slug"], "arena", "{q}");
    }
    let none = body_json(
        app.send(req(Method::GET, "/v1/admin/apps?q=zzz").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(none.as_array().unwrap().len(), 0);

    let record = body_json(
        app.send(req(Method::GET, "/v1/admin/apps/arena/record").cookie(&reviewer))
            .await,
    )
    .await;
    assert_eq!(record["app"]["maker"], "Mia Maker");
    assert!(record["releases"].is_array());
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/apps/arena/record").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Editing needs moderation rights and a reason; the maker is told.
    let edit = |cookie: &str, body: Value| {
        req(Method::PATCH, "/v1/admin/apps/arena/listing")
            .cookie(cookie)
            .json(body)
    };
    assert_eq!(
        app.send(edit(&reviewer, json!({ "title": "Arena!", "reason": "x" })))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(edit(&owner, json!({ "title": "Arena!", "reason": "" })))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = body_json(
        app.send(edit(
            &owner,
            json!({ "title": "Brawl Pit", "min_age": 12, "reason": "The old name is someone else's trademark." }),
        ))
        .await,
    )
    .await;
    assert_eq!(res["changed"], json!(["title", "age rating"]));
    let page = body_json(app.send(req(Method::GET, "/v1/apps/arena")).await).await;
    assert_eq!(page["title"], "Brawl Pit");
    let told: String = sqlx::query_scalar(
        "select body from notifications where user_id = $1 and title like 'Slop changed%'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(told.contains("someone else's trademark"));
    let record = body_json(
        app.send(req(Method::GET, "/v1/admin/apps/arena/record").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(record["audit"][0]["action"], "app.listing_edited");
    assert_eq!(
        record["audit"][0]["detail"]["changed"]["title"]["from"],
        "arena"
    );

    // The audit log searches and pages.
    let found = body_json(
        app.send(req(Method::GET, "/v1/admin/audit?q=listing_edited").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(found.as_array().unwrap().len(), 1);
    let first = found[0]["id"].as_i64().unwrap();
    let older = body_json(
        app.send(
            req(
                Method::GET,
                format!("/v1/admin/audit?q=listing_edited&before={first}"),
            )
            .cookie(&owner),
        )
        .await,
    )
    .await;
    assert_eq!(older.as_array().unwrap().len(), 0);

    // Money is for finance (an owner has every role); a reviewer can't see it.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/money").cookie(&reviewer))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let money = body_json(
        app.send(req(Method::GET, "/v1/admin/money").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(money["open_holds"], 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_fix_profiles_and_teams_and_freeze_apps_by_hand(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
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

    // What can be fixed, then the fix: only what changed, the person told why.
    let profile = body_json(
        app.send(req(Method::GET, format!("/v1/admin/users/{maker_id}/profile")).cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(profile["creative"]["handle"], "mia");
    assert_eq!(profile["teams"][0]["slug"], "mias-crew");
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/admin/users/{maker_id}/profile")).cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = body_json(
        app.send(
            req(Method::PATCH, format!("/v1/admin/users/{maker_id}/profile"))
                .cookie(&owner)
                .json(json!({
                    "display_name": "Mia Maker",
                    "creative": { "headline": "Pixel artist", "remove_links": true },
                    "reason": "Claiming to be a studio you aren't is impersonation, and the link was a scam."
                })),
        )
        .await,
    )
    .await;
    assert_eq!(res["changed"], json!(["headline", "links"]));
    let (headline, links): (String, Value) =
        sqlx::query_as("select headline, links from creatives where user_id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((headline.as_str(), links), ("Pixel artist", json!([])));
    let res = app
        .send(
            req(Method::PATCH, "/v1/admin/teams/mias-crew")
                .cookie(&owner)
                .json(json!({ "name": "Mia's Crew", "reason": "Not affiliated with Valve." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let told: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 order by created_at",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert!(
        told.contains(&"Slop changed your profile".to_string()),
        "{told:?}"
    );
    assert!(
        told.contains(&"Slop changed your team's page".to_string()),
        "{told:?}"
    );

    // A hand freeze: no new releases, still in the store, and the daily check leaves it.
    listed_with(&app, maker_id, "arena", "released", "").await;
    let frozen = body_json(
        app.send(
            req(Method::POST, "/v1/admin/apps/arena/freeze")
                .cookie(&owner)
                .json(json!({ "reason": "A player reported the last build steals passwords; we're checking." })),
        )
        .await,
    )
    .await;
    assert_eq!(frozen["by_staff"], true);
    let id = app_id(&app, "arena").await;
    let state = app.state.clone();
    slop_api::frozen::apply(&state, id, slop_api::frozen::Access::Readable)
        .await
        .unwrap();
    assert!(
        slop_api::frozen::frozen_reason(&app.db, id)
            .await
            .unwrap()
            .is_some(),
        "the repository check doesn't lift a staff freeze"
    );
    sqlx::query("update apps set frozen_at = now() - interval '30 days' where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::frozen::end_grace(&state).await.unwrap();
    let status: String = sqlx::query_scalar("select status::text from apps where id = $1")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "listed", "a staff freeze never delists on its own");
    let res = body_json(
        app.send(
            req(Method::POST, "/v1/admin/apps/arena/unfreeze")
                .cookie(&owner)
                .json(json!({ "note": "Checked the build: it was a false report." })),
        )
        .await,
    )
    .await;
    assert!(res.is_null(), "{res}");
    let audit: Vec<String> =
        sqlx::query_scalar("select action from audit_log where subject = 'arena' order by id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(audit, ["app.frozen", "app.unfrozen"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_make_shelves_and_hide_automatic_ones(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    for slug in ["cozy-farm", "tea-shop", "dark-dungeon"] {
        listed_with(&app, maker_id, slug, "released", "").await;
    }
    let shelf = json!({ "slug": "cozy", "title": "Cozy games", "blurb": "Slow, kind, warm.",
                        "apps": ["tea-shop", "cozy-farm"] });
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/admin/shelves")
                .cookie(&maker)
                .json(shelf.clone())
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(
            req(Method::POST, "/v1/admin/shelves")
                .cookie(&owner)
                .json(shelf),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::POST, "/v1/admin/shelves")
                .cookie(&owner)
                .json(json!({ "slug": "Bad Slug", "title": "x", "apps": [] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = app
        .send(
            req(Method::POST, "/v1/admin/front-shelves/fresh_slop")
                .cookie(&owner)
                .json(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["hidden"], json!(["fresh_slop"]));
    assert_eq!(front["shelves"][0]["title"], "Cozy games");
    let order: Vec<&str> = front["shelves"][0]["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["tea-shop", "cozy-farm"]);

    // Off the front page, it keeps its own page.
    let res = app
        .send(
            req(Method::PUT, "/v1/admin/shelves/cozy")
                .cookie(&owner)
                .json(json!({ "title": "Cozy games", "on_front": false, "apps": ["cozy-farm"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let front = body_json(app.send(req(Method::GET, "/v1/shelves")).await).await;
    assert_eq!(front["shelves"], json!([]));
    let page = body_json(app.send(req(Method::GET, "/v1/shelves/staff/cozy")).await).await;
    assert_eq!(page["apps"][0]["slug"], "cozy-farm");
    let res = app
        .send(req(Method::DELETE, "/v1/admin/shelves/cozy").cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/v1/shelves/staff/cozy"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_publish_legal_changes_and_agents_only_ask_for_staff_changes(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = make_admin(&app, "Olive Owner").await;
    let (helper_id, _) = app.user("Hal Helper", "US").await;

    // A new Terms version from the panel (longer than most requests may be): announced, in
    // effect 15 days later, withdrawable.
    let docs = body_json(
        app.send(req(Method::GET, "/v1/admin/legal").cookie(&owner))
            .await,
    )
    .await;
    let terms = docs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "terms")
        .unwrap()
        .clone();
    let body = format!(
        "{}\n\n{}",
        terms["latest_body_md"].as_str().unwrap(),
        "One more clause, spelled out in full.\n".repeat(2_000)
    );
    assert!(body.len() > 64 * 1024);
    let res = app
        .send(
            req(Method::POST, "/v1/admin/legal/terms")
                .cookie(&owner)
                .json(json!({ "summary": "Adds some clauses.", "body_md": body })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let published = body_json(res).await;
    let version = published["version"].as_str().unwrap().to_string();
    let index = body_json(app.send(req(Method::GET, "/v1/legal")).await).await;
    let terms = index
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "terms")
        .unwrap()
        .clone();
    assert_eq!(terms["upcoming"]["version"], version.as_str());
    let res = app
        .send(
            req(Method::POST, "/v1/admin/legal/terms")
                .cookie(&owner)
                .json(json!({ "summary": "Again", "body_md": "Different words." })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "one announced change at a time"
    );
    let docs = body_json(
        app.send(req(Method::GET, "/v1/admin/legal").cookie(&owner))
            .await,
    )
    .await;
    let file = docs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "terms")
        .unwrap()["file_text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(slop_api::legal::parse(&file).is_ok(), "{file}");
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/legal/terms/{version}/withdraw"),
            )
            .cookie(&owner)
            .json(json!({ "reason": "Published by mistake." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let index = body_json(app.send(req(Method::GET, "/v1/legal")).await).await;
    assert!(index[0]["upcoming"].is_null());

    // An agent asks to make someone staff; it waits for a person, even with one approval enough.
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(&owner)
                .json(json!({ "name": "Phone Claude", "scope": "moderate", "days": 7 })),
        )
        .await;
    let token = format!("Bearer {}", body_json(res).await["token"].as_str().unwrap());
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{helper_id}/roles"))
                .header("authorization", token.clone())
                .json(json!({ "role": "reviewer", "grant": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let pending = body_json(res).await;
    let action = pending["pending_action"].as_str().unwrap().to_string();
    let is_staff = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, bool>("select is_admin from users where id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert!(!is_staff(helper_id).await);
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/actions/{action}/approve"))
                .header("authorization", token.clone()),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "a token never approves"
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&owner))
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    assert!(is_staff(helper_id).await);

    // Removing them: every role and token goes. The last owner can't be removed.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{owner_id}/remove"))
                .cookie(&owner)
                .json(json!({ "reason": "Testing the last-owner rule." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{helper_id}/remove"))
                .cookie(&owner)
                .json(json!({ "reason": "Only needed for the launch week." })),
        )
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    assert!(!is_staff(helper_id).await);

    // The tools are there over MCP.
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
    for want in [
        "freeze_app",
        "edit_profile",
        "edit_team",
        "create_shelf",
        "publish_legal_document",
        "change_staff_role",
        "remove_from_staff",
        "pause_donations",
        "app_record",
    ] {
        assert!(names.contains(&want), "{want}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_public_site_behind_a_proxy_needs_the_client_ip_header(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.public_url = "https://slop.example".parse().unwrap();
        c.client_ip_header = None;
    })
    .await;
    let keys: Vec<String> = slop_api::ops::config_alerts(&app.state.config)
        .into_iter()
        .map(|a| a.key)
        .collect();
    assert_eq!(keys, ["client-ip-header-missing"]);
    let mut config = (*app.state.config).clone();
    config.client_ip_header = Some("x-forwarded-for".into());
    assert!(slop_api::ops::config_alerts(&config).is_empty());
}

/// The owner's decision of 2026-10-25 (SECURITY §4.3): an old session, of an account with a
/// passkey it never used, approves a desktop sign-in and closes the account, with no "confirm
/// it's you" first. (Adding a passkey from an old session: `passkeys_are_added_from_any_session_and_still_confirm_staff_tokens`.)
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn old_sessions_approve_desktop_sign_ins_and_close_the_account(db: PgPool) {
    let app = test_app(db).await;
    let (user_id, cookie) = app.user("Pat Player", "US").await;
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, '\\x01', '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update sessions set created_at = now() - interval '30 days', stepped_up_at = null, passkey_at = null where user_id = $1")
        .bind(user_id)
        .execute(&app.db)
        .await
        .unwrap();

    // Approving the desktop app.
    let res = app
        .send(req(Method::POST, "/v1/auth/desktop/requests").json(json!({
            "code_challenge": pkce_challenge(&"v".repeat(43)),
            "redirect_port": 49152,
            "state": "state-state-state-1",
            "device_name": "Slop for Linux"
        })))
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/auth/desktop/requests/{id}/approve"),
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        body_json(res).await["redirect_url"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:49152/")
    );

    // Closing the account (typing the confirmation is the safeguard).
    let res = app
        .send(
            req(Method::POST, "/v1/me/close")
                .cookie(&cookie)
                .json(json!({ "confirm": "close my account" })),
        )
        .await;
    assert!(res.status().is_success(), "{}", res.status());
    let closed: bool = sqlx::query_scalar("select closed_at is not null from users where id = $1")
        .bind(user_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(closed);
}
