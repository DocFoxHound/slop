// SPDX-License-Identifier: AGPL-3.0-or-later
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
            or (kind <> 'agent'
                and last_seen_at < now() - (case when kind = 'desktop' then $3 else $2 end) - $1)",
    )
    .bind(ENDED_SESSION_RETENTION)
    .bind(session::IDLE_TIMEOUT)
    .bind(session::DESKTOP_IDLE_TIMEOUT)
    .execute(db)
    .await?
    .rows_affected();
    crate::passkeys::cleanup(db).await?;
    crate::admin_messages::cleanup(db).await?;
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
        match crate::demos::expire(&state).await {
            Ok(n) if n > 0 => tracing::info!(ended = n, "browser demos"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "ending demos failed"),
        }
        match crate::retention::purge(&state.db).await {
            Ok(p) if p != crate::retention::Purged::default() => {
                tracing::info!(?p, "records past their time")
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "retention purge failed"),
        }
        match crate::media::hash_old_thumbnails(&state, 20).await {
            Ok(n) if n > 0 => tracing::info!(hashed = n, "old thumbnails"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "hashing old thumbnails failed"),
        }
        match crate::maker_agents::warn_expiring(&state.db).await {
            Ok(n) if n > 0 => tracing::info!(sent = n, "AI agent token expiry warnings"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "AI agent token expiry warnings failed"),
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
        // Profile pictures last read over a day ago (avatars.rs), slowly, beside the rest.
        tokio::spawn({
            let state = state.clone();
            async move {
                match crate::avatars::refresh_due(&state).await {
                    Ok(n) if n > 0 => tracing::info!(updated = n, "profile pictures"),
                    Ok(_) => {}
                    Err(err) => tracing::error!(error = ?err, "profile picture checks failed"),
                }
            }
        });
        state.rate_limiter.prune();
        tokio::time::sleep(StdDuration::from_secs(3600)).await;
    }
}
