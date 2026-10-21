//! Deletes what's no longer needed (LEGAL-GUIDANCE PRIV-1, §15: keep as little as possible), and
//! completes People jobs whose 7-day window has passed.
//!
//! Runs hourly in every API instance; the deletes are idempotent, so instances overlapping is
//! harmless.

use std::time::Duration as StdDuration;

use sqlx::PgPool;
use time::Duration;

use crate::auth::session;

/// Ended sessions are kept this long (so a just-revoked device can still be explained), then
/// deleted with their tokens.
pub const ENDED_SESSION_RETENTION: Duration = Duration::days(7);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Deleted {
    pub oauth_flows: u64,
    pub pending_signups: u64,
    pub desktop_requests: u64,
    pub access_tokens: u64,
    pub refresh_tokens: u64,
    pub sessions: u64,
}

pub async fn run_once(db: &PgPool) -> anyhow::Result<Deleted> {
    let exec = |sql: &'static str| async move {
        anyhow::Ok(sqlx::query(sql).execute(db).await?.rows_affected())
    };
    let mut deleted = Deleted {
        oauth_flows: exec("delete from oauth_flows where expires_at < now()").await?,
        pending_signups: exec("delete from pending_signups where expires_at < now()").await?,
        desktop_requests: exec("delete from desktop_auth_requests where expires_at < now()")
            .await?,
        access_tokens: exec("delete from access_tokens where expires_at < now()").await?,
        // Used refresh tokens are kept until they expire: presenting one again is how theft is
        // detected (auth/desktop.rs).
        refresh_tokens: exec("delete from refresh_tokens where expires_at < now()").await?,
        sessions: 0,
    };
    deleted.sessions = sqlx::query(
        "delete from sessions
         where revoked_at < now() - $1
            or expires_at < now() - $1
            or last_seen_at < now() - (case kind when 'desktop' then $3 else $2 end) - $1",
    )
    .bind(ENDED_SESSION_RETENTION)
    .bind(session::IDLE_TIMEOUT)
    .bind(session::DESKTOP_IDLE_TIMEOUT)
    .execute(db)
    .await?
    .rows_affected();
    crate::passkeys::cleanup(db).await?;
    Ok(deleted)
}

/// Runs forever: cleans up hourly and prunes idle rate-limit buckets.
pub async fn run(state: crate::AppState) {
    loop {
        match run_once(&state.db).await {
            Ok(d) if d != Deleted::default() => tracing::info!(?d, "cleanup"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "cleanup failed"),
        }
        match crate::pulse::delete_old_sessions(&state.db).await {
            Ok(n) if n > 0 => tracing::info!(deleted = n, "old pulse sessions"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "pulse cleanup failed"),
        }
        match crate::passkeys::remind(&state.db).await {
            Ok(n) if n > 0 => tracing::info!(sent = n, "passkey reminders"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "passkey reminders failed"),
        }
        if let Err(err) = crate::notify::cleanup(&state.db).await {
            tracing::error!(error = ?err, "notification cleanup failed");
        }
        // Chat: encrypted messages, Welcomes and attachments after 30 days; unapproved devices
        // after a day.
        match crate::chat::cleanup(&state.db).await {
            Ok(n) if n > 0 => tracing::info!(deleted = n, "old chat deliveries"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "chat cleanup failed"),
        }
        if let Err(err) = crate::chat::expire_pending(&state.db).await {
            tracing::error!(error = ?err, "chat device expiry failed");
        }
        match crate::people::jobs::auto_complete(&state.db, time::OffsetDateTime::now_utc()).await {
            Ok(n) if n > 0 => tracing::info!(completed = n, "people jobs completed after 7 days"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "people auto-complete failed"),
        }
        match crate::people::jobs::forget_conversations(&state.db, time::OffsetDateTime::now_utc())
            .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(messages = n, "job conversation text deleted after 30 days")
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "forgetting job conversations failed"),
        }
        state.rate_limiter.prune();
        tokio::time::sleep(StdDuration::from_secs(3600)).await;
    }
}
