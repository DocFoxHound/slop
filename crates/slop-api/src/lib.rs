// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Slop API: one modular application (docs/ARCHITECTURE.md §4).

pub mod abuse;
pub mod account;
pub mod admin;
pub mod admin_web;
pub mod agent;
pub mod auth;
pub mod bad_links;
pub mod bounties;
pub mod branches;
pub mod bug_reports;
pub mod catalog;
pub mod challenge;
pub mod chat;
pub mod cleanup;
pub mod config;
pub mod demos;
pub mod dep_watch;
pub mod desktop_updates;
pub mod donations;
pub mod error;
pub mod featured;
pub mod fields;
pub mod fixes;
pub mod foryou;
pub mod friends;
pub mod frozen;
pub mod github_app;
pub mod interested;
pub mod label;
pub mod legal;
pub mod legal_requests;
pub mod library;
pub mod limits;
pub mod maker_agents;
pub mod maker_keys;
pub mod manage;
pub mod media;
pub mod moderation;
pub mod notify;
pub mod offers;
pub mod ops;
pub mod passkeys;
pub mod people;
pub mod policy;
pub mod private;
pub mod pulse;
pub mod push;
pub mod ratelimit;
pub mod releases;
pub mod repo_owner;
pub mod repo_tokens;
pub mod rescan;
pub mod retention;
pub mod review;
pub mod reviews;
pub mod save_sync;
pub mod scan;
pub mod sim;
pub mod sim_donations;
pub mod sim_moderation;
pub mod sim_people;
pub mod slop_build;
pub mod source_scan;
pub mod staff;
pub mod teams;
pub mod tokens;
pub mod updates;
pub mod votes;
pub mod web;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, StatusCode, header},
    routing::get,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower_http::{
    csrf::CsrfLayer, limit::RequestBodyLimitLayer, set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config::Config;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// For calls to sign-in providers: short timeout, no redirects.
    pub http: reqwest::Client,
    /// For fetching release files from makers' release pages: long timeout, a few redirects
    /// (GitHub release downloads redirect to its file storage).
    pub fetch_http: reqwest::Client,
    /// Wakes the release fetcher when a release is submitted.
    pub fetch_wakeup: Arc<tokio::sync::Notify>,
    /// Wakes the bounty charger when a delivery is confirmed (bounties.rs).
    pub bounty_wakeup: Arc<tokio::sync::Notify>,
    /// Wakes the GitHub release watcher when a maker changes its settings.
    pub watch_wakeup: Arc<tokio::sync::Notify>,
    /// Checks makers' GitHub artifact attestations.
    pub provenance: Arc<slop_provenance::ProvenanceVerifier>,
    pub rate_limiter: Arc<ratelimit::RateLimiter>,
    /// GitHub's OIDC signing keys, for Slop Build submissions.
    pub oidc_keys: Arc<tokio::sync::RwLock<slop_build::OidcKeys>>,
    /// Slop's GitHub App installation tokens.
    pub github_app_tokens: Arc<github_app::TokenCache>,
    /// Wakes chat long-polls (chat.rs).
    pub chat_hub: chat::Hub,
    /// Rate limits per account (limits.rs).
    pub account_limiter: Arc<limits::AccountLimiter>,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let fetch_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(reqwest::redirect::Policy::limited(5))
            .https_only(!config.releases.allow_http_downloads)
            .build()?;
        let rate_limiter = Arc::new(ratelimit::RateLimiter::new(config.rate_limits));
        Ok(Self {
            db,
            config: Arc::new(config),
            http,
            fetch_http,
            fetch_wakeup: Arc::new(tokio::sync::Notify::new()),
            bounty_wakeup: Arc::new(tokio::sync::Notify::new()),
            watch_wakeup: Arc::new(tokio::sync::Notify::new()),
            provenance: Arc::new(slop_provenance::ProvenanceVerifier::public_good()?),
            rate_limiter,
            oidc_keys: Arc::default(),
            github_app_tokens: Arc::default(),
            chat_hub: chat::Hub::default(),
            account_limiter: Arc::default(),
        })
    }
}

pub fn router(state: AppState) -> anyhow::Result<Router> {
    // Cross-site state-changing requests are refused using Sec-Fetch-Site / Origin. With
    // SameSite=Lax cookies this replaces per-request CSRF tokens (docs/SECURITY.md §4.2).
    let mut csrf = CsrfLayer::new();
    for origin in &state.config.trusted_origins {
        csrf = csrf.add_trusted_origin(origin)?;
    }

    let v1 = Router::new()
        .route("/source", get(source))
        .merge(abuse::routes())
        .merge(auth::routes())
        .merge(catalog::routes())
        .merge(releases::routes())
        .merge(library::routes())
        .merge(slop_build::routes())
        .merge(staff::routes())
        .merge(updates::routes())
        .merge(maker_keys::routes())
        .merge(manage::routes())
        .merge(media::routes())
        .merge(desktop_updates::routes())
        .merge(admin_web::routes())
        .merge(agent::routes())
        .merge(maker_agents::routes())
        .merge(fixes::routes())
        .merge(offers::routes())
        .merge(label::routes())
        .merge(bounties::routes())
        .merge(bug_reports::routes())
        .merge(demos::routes())
        .merge(review::routes())
        .merge(pulse::routes())
        .merge(legal_requests::routes())
        .merge(votes::routes())
        .merge(featured::routes())
        .merge(foryou::routes())
        .merge(sim::routes())
        .merge(people::routes())
        .merge(legal::routes())
        .merge(moderation::routes())
        .merge(account::routes())
        .merge(notify::routes())
        .merge(passkeys::routes())
        .merge(ops::routes())
        .merge(donations::routes())
        .merge(reviews::routes())
        .merge(branches::routes())
        .merge(private::routes())
        .merge(teams::routes())
        .merge(frozen::routes())
        .merge(friends::routes())
        .merge(push::routes())
        .merge(repo_owner::routes())
        .merge(repo_tokens::routes())
        .merge(chat::routes())
        .merge(interested::routes())
        // Everything except the desktop build upload and webhooks: small bodies, quick answers.
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            Duration::from_secs(20),
        ))
        .merge(
            github_app::webhook_routes()
                .layer(axum::extract::DefaultBodyLimit::max(
                    github_app::MAX_WEBHOOK_BYTES,
                ))
                .layer(RequestBodyLimitLayer::new(github_app::MAX_WEBHOOK_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(20),
                )),
        )
        .merge(
            chat::message_routes()
                .layer(axum::extract::DefaultBodyLimit::max(
                    chat::MAX_COMMIT_BODY * 2,
                ))
                .layer(RequestBodyLimitLayer::new(chat::MAX_COMMIT_BODY * 2))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(20),
                )),
        )
        .merge(
            chat::upload_routes()
                .layer(axum::extract::DefaultBodyLimit::max(
                    chat::MAX_ATTACHMENT_BYTES,
                ))
                .layer(RequestBodyLimitLayer::new(chat::MAX_ATTACHMENT_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(120),
                )),
        )
        .merge(chat::sync_routes().layer(TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            Duration::from_secs(40),
        )))
        .merge(
            people::jobs::evidence_routes()
                .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
                .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(20),
                )),
        )
        .merge(
            save_sync::routes()
                .layer(axum::extract::DefaultBodyLimit::max(
                    save_sync::MAX_SAVE_BYTES,
                ))
                .layer(RequestBodyLimitLayer::new(save_sync::MAX_SAVE_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(120),
                )),
        )
        .merge(
            desktop_updates::upload_routes()
                .layer(axum::extract::DefaultBodyLimit::disable())
                .layer(RequestBodyLimitLayer::new(desktop_updates::MAX_BUILD_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(600),
                )),
        );

    let not_found = || async {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": { "code": "not_found", "message": "not found" } })),
        )
    };
    let mut router = Router::new()
        .route("/healthz", get(health))
        .route("/.well-known/security.txt", get(security_txt))
        .merge(demos::play_routes())
        .nest("/v1", v1.fallback(not_found));
    // Optionally serve the built website (web/dist) from the same origin, so one process runs the
    // whole site. Otherwise everything outside /v1 is a JSON 404.
    router = match &state.config.web_dir {
        Some(dir) => router.fallback_service(web::service(dir)),
        None => router.fallback(not_found),
    };
    // The play origin serves demos and nothing else; demos are only on it.
    let play_only = axum::middleware::from_fn_with_state(
        state.clone(),
        |axum::extract::State(state): axum::extract::State<AppState>,
         req: axum::extract::Request,
         next: axum::middleware::Next| async move {
            let on_play = demos::is_play_host(&state, req.headers());
            let is_demo = req.uri().path().starts_with("/play/");
            if on_play != is_demo {
                return axum::response::IntoResponse::into_response(StatusCode::NOT_FOUND);
            }
            next.run(req).await
        },
    );
    let mut router = router
        .layer(play_only)
        .layer(csrf)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ratelimit::middleware,
        ))
        .layer(header_layer(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_layer(
            header::REFERRER_POLICY,
            "strict-origin-when-cross-origin",
        ))
        .layer(header_layer(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'none'",
        ))
        .layer(header_layer(header::CACHE_CONTROL, "no-store"))
        // Nothing on Slop needs the camera, microphone, location, USB or payment APIs.
        .layer(header_layer(
            HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=(), interest-cohort=()",
        ))
        .layer(header_layer(
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ))
        .layer(TraceLayer::new_for_http());
    if state.config.secure_cookies() {
        router = router.layer(header_layer(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=63072000; includeSubDomains; preload",
        ));
    }
    Ok(router.with_state(state))
}

fn header_layer(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// The commit this server was built from (`SLOP_COMMIT` at build time, deploy/update.sh).
const COMMIT: Option<&str> = option_env!("SLOP_COMMIT");

/// Where to get this Slop's source code, at the version that's running (the AGPL's section 13).
async fn source(axum::extract::State(state): axum::extract::State<AppState>) -> Json<Value> {
    let url = state.config.source_url.as_deref();
    let commit = COMMIT.filter(|c| c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    // GitHub, GitLab, Codeberg and Forgejo all show a commit's files at /tree/<commit>.
    let version_url = url.zip(commit).map(|(u, c)| format!("{u}/tree/{c}"));
    Json(json!({
        "licence": "AGPL-3.0-or-later",
        "url": url,
        "commit": commit,
        "version_url": version_url,
    }))
}

/// How to report a security problem (RFC 9116), once `SLOP_SECURITY_CONTACT` names a contact.
async fn security_txt(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(contact) = state.config.security_contact.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::days(180))
        .replace_nanosecond(0)
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let site = state.config.public_url.as_str().trim_end_matches('/');
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!(
            "Contact: {contact}\nExpires: {expires}\nPreferred-Languages: en\nCanonical: {site}/.well-known/security.txt\n"
        ),
    )
        .into_response()
}

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> StatusCode {
    match sqlx::query("select 1").execute(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
