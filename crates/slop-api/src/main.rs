use std::time::Duration;

use anyhow::Context;
use slop_api::{AppState, MIGRATOR, config::Config, router};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "slop_api=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    if config.github.is_none() {
        tracing::warn!("GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET not set: sign-in is off");
    }

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    MIGRATOR.run(&db).await.context("running migrations")?;

    let bind = config.bind;
    let app = router(AppState::new(db, config)?)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "Slop API listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
