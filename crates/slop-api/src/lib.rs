//! The Slop API: one modular application (docs/ARCHITECTURE.md §4).

pub mod account;
pub mod admin;
pub mod admin_web;
pub mod auth;
pub mod branches;
pub mod catalog;
pub mod challenge;
pub mod chat;
pub mod cleanup;
pub mod config;
pub mod desktop_updates;
pub mod donations;
pub mod error;
pub mod featured;
pub mod foryou;
pub mod friends;
pub mod frozen;
pub mod github_app;
pub mod interested;
pub mod legal;
pub mod library;
pub mod manage;
pub mod media;
pub mod moderation;
pub mod notify;
pub mod people;
pub mod policy;
pub mod private;
pub mod pulse;
pub mod push;
pub mod ratelimit;
pub mod releases;
pub mod repo_owner;
pub mod review;
pub mod reviews;
pub mod scan;
pub mod sim;
pub mod sim_donations;
pub mod sim_moderation;
pub mod sim_people;
pub mod slop_build;
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
use serde_json::json;
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
            watch_wakeup: Arc::new(tokio::sync::Notify::new()),
            provenance: Arc::new(slop_provenance::ProvenanceVerifier::public_good()?),
            rate_limiter,
            oidc_keys: Arc::default(),
            github_app_tokens: Arc::default(),
            chat_hub: chat::Hub::default(),
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
        .merge(auth::routes())
        .merge(catalog::routes())
        .merge(releases::routes())
        .merge(library::routes())
        .merge(slop_build::routes())
        .merge(updates::routes())
        .merge(manage::routes())
        .merge(media::routes())
        .merge(desktop_updates::routes())
        .merge(admin_web::routes())
        .merge(review::routes())
        .merge(pulse::routes())
        .merge(votes::routes())
        .merge(featured::routes())
        .merge(foryou::routes())
        .merge(sim::routes())
        .merge(people::routes())
        .merge(legal::routes())
        .merge(moderation::routes())
        .merge(account::routes())
        .merge(notify::routes())
        .merge(donations::routes())
        .merge(reviews::routes())
        .merge(branches::routes())
        .merge(private::routes())
        .merge(teams::routes())
        .merge(frozen::routes())
        .merge(friends::routes())
        .merge(push::routes())
        .merge(repo_owner::routes())
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
        .nest("/v1", v1.fallback(not_found));
    // Optionally serve the built website (web/dist) from the same origin, so one process runs the
    // whole site. Otherwise everything outside /v1 is a JSON 404.
    router = match &state.config.web_dir {
        Some(dir) => router.fallback_service(web::service(dir)),
        None => router.fallback(not_found),
    };
    let mut router = router
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

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> StatusCode {
    match sqlx::query("select 1").execute(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
