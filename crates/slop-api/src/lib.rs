//! The Slop API: one modular application (docs/ARCHITECTURE.md §4).

pub mod auth;
pub mod catalog;
pub mod config;
pub mod error;
pub mod policy;
pub mod tokens;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, StatusCode, header},
    routing::get,
};
use serde_json::json;
use sqlx::PgPool;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    csrf::CsrfLayer,
    limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::config::Config;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            db,
            config: Arc::new(config),
            http,
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

    // Read-only, cookie-less access for the packaged desktop app. Signed-in desktop requests
    // will use tokens, not cookies (docs/SECURITY.md §4.1-4.2).
    let cors_origins = state
        .config
        .cors_origins
        .iter()
        .map(|o| HeaderValue::from_str(o))
        .collect::<Result<Vec<_>, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(cors_origins))
        .allow_methods([axum::http::Method::GET])
        .max_age(Duration::from_secs(3600));

    let v1 = Router::new().merge(auth::routes()).merge(catalog::routes());

    let mut router = Router::new()
        .route("/healthz", get(health))
        .nest("/v1", v1)
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": { "code": "not_found", "message": "not found" } })),
            )
        })
        .layer(csrf)
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            Duration::from_secs(20),
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
