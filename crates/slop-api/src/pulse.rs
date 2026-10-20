//! Slop Pulse (DESIGN §11, METRICS §2): live player counts and playtime, from games that ping.
//!
//! Optional for makers. A game reads `SLOP_PULSE_TOKEN` and `SLOP_PULSE_URL` from its
//! environment (set by the Slop desktop app at launch) and posts the token to `…/start`, then to
//! `…/heartbeat` about once a minute, and to `…/end` when it quits. The token is the only thing
//! sent (PRIV-8). Each account counts once per game however many copies it runs.

use std::time::Duration as StdDuration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
    tokens::{hash_token, new_token},
};

/// How often games should send a heartbeat.
pub const HEARTBEAT_SECONDS: i64 = 60;
/// A player counts as "playing now" if their last heartbeat is this recent.
pub const LIVE_WINDOW: Duration = Duration::minutes(3);
/// A launch token must be used (`start`) this soon after the app was launched.
const START_WINDOW: Duration = Duration::minutes(15);
/// After this long without a heartbeat, a session is over; the game needs a new launch.
const SESSION_TIMEOUT: Duration = Duration::minutes(10);
/// Heartbeats closer together than this don't add playtime.
const MIN_BEAT_GAP: Duration = Duration::seconds(20);
/// The most playtime one heartbeat can add (a game that paused, or a gap in the network).
const MAX_BEAT_CREDIT: Duration = Duration::minutes(3);
/// Player counts are kept per bucket of this many seconds.
pub const BUCKET_SECONDS: i64 = 300;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/launch", post(launch))
        .route("/apps/{slug}/players", get(players))
        .route("/pulse/start", post(start))
        .route("/pulse/heartbeat", post(heartbeat))
        .route("/pulse/end", post(end))
}

#[derive(Serialize)]
struct Launched {
    /// Hand this to the game as SLOP_PULSE_TOKEN. Absent when the player turned off "Share my
    /// playtime".
    token: Option<String>,
    /// Hand this to the game as SLOP_PULSE_URL.
    pulse_url: String,
    heartbeat_seconds: i64,
}

/// Called by the desktop app when a signed-in player launches an app: records the launch day
/// and issues a one-time Pulse token.
async fn launch(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Launched>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    let pulse_url = format!(
        "{}/v1/pulse",
        state.config.public_url.as_str().trim_end_matches('/')
    );

    let share: bool = sqlx::query_scalar("select share_playtime from users where id = $1")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
    if !share {
        return Ok(Json(Launched {
            token: None,
            pulse_url,
            heartbeat_seconds: HEARTBEAT_SECONDS,
        }));
    }

    let token = new_token();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into pulse_sessions (id, token_hash, app_id, user_id) values ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(hash_token(&token))
    .bind(app_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    // Apps without Pulse are weighted by the days they were launched (METRICS §2).
    sqlx::query(
        "insert into user_app_usage (user_id, app_id, days_launched, last_launch_day)
         values ($1, $2, 1, current_date)
         on conflict (user_id, app_id) do update set
           days_launched = user_app_usage.days_launched
             + case when user_app_usage.last_launch_day = current_date then 0 else 1 end,
           last_launch_day = current_date",
    )
    .bind(user.id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(Launched {
        token: Some(token),
        pulse_url,
        heartbeat_seconds: HEARTBEAT_SECONDS,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBody {
    token: String,
}

fn unknown() -> ApiError {
    ApiError::invalid(
        "token",
        "unknown or expired Pulse token; relaunch the game from Slop",
    )
}

async fn start(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> ApiResult<StatusCode> {
    // Starting twice is harmless (a retry); a token can't be started late or after it ended.
    let updated = sqlx::query(
        "update pulse_sessions set started_at = coalesce(started_at, now()), last_beat_at = coalesce(last_beat_at, now())
         where token_hash = $1 and ended_at is null and issued_at > now() - $2
           and (started_at is null or last_beat_at > now() - $3)",
    )
    .bind(hash_token(&body.token))
    .bind(START_WINDOW)
    .bind(SESSION_TIMEOUT)
    .execute(&state.db)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(unknown());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn heartbeat(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, Uuid, Uuid, OffsetDateTime)> = sqlx::query_as(
        "select id, app_id, user_id, last_beat_at from pulse_sessions
         where token_hash = $1 and started_at is not null and ended_at is null and last_beat_at > now() - $2
         for update",
    )
    .bind(hash_token(&body.token))
    .bind(SESSION_TIMEOUT)
    .fetch_optional(&mut *tx)
    .await?;
    let (id, app_id, user_id, last) = row.ok_or_else(unknown)?;
    let gap = OffsetDateTime::now_utc() - last;
    if gap < MIN_BEAT_GAP {
        // Too soon: accepted, but it adds nothing.
        return Ok(StatusCode::NO_CONTENT);
    }
    let credit = gap.min(MAX_BEAT_CREDIT).whole_seconds();
    sqlx::query("update pulse_sessions set last_beat_at = now(), beats = beats + 1 where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "update user_app_usage set playtime_seconds = playtime_seconds + $3, last_played_at = now()
         where user_id = $1 and app_id = $2",
    )
    .bind(user_id)
    .bind(app_id)
    .bind(credit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn end(State(state): State<AppState>, Json(body): Json<TokenBody>) -> ApiResult<StatusCode> {
    sqlx::query(
        "update pulse_sessions set ended_at = now() where token_hash = $1 and ended_at is null",
    )
    .bind(hash_token(&body.token))
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DayPeak {
    /// YYYY-MM-DD (UTC).
    pub day: String,
    pub peak: i32,
}

#[derive(Serialize)]
pub struct Players {
    pub playing_now: i64,
    pub peak_this_month: i64,
    /// The busiest moment of each of the last 30 days.
    pub daily_peaks: Vec<DayPeak>,
}

/// Distinct players with a recent heartbeat, per app.
const LIVE_PLAYERS_SQL: &str = "select app_id, count(distinct user_id)::int as players from pulse_sessions
     where started_at is not null and ended_at is null and last_beat_at > now() - $1 group by app_id";

async fn players(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Players>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    Ok(Json(player_stats(&state.db, app_id).await?))
}

/// Players now (live heartbeats), this month's peak and each day's peak for 30 days.
pub async fn player_stats(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Players> {
    let playing_now: i64 = sqlx::query_scalar(
        "select count(distinct user_id) from pulse_sessions
         where app_id = $1 and started_at is not null and ended_at is null and last_beat_at > now() - $2",
    )
    .bind(app_id)
    .bind(LIVE_WINDOW)
    .fetch_one(db)
    .await?;
    let month_peak: Option<i32> = sqlx::query_scalar(
        "select max(players) from pulse_counts where app_id = $1 and bucket >= date_trunc('month', now())",
    )
    .bind(app_id)
    .fetch_one(db)
    .await?;
    let daily_peaks = sqlx::query_as(
        "select to_char((bucket at time zone 'UTC')::date, 'YYYY-MM-DD') as day, max(players) as peak from pulse_counts
         where app_id = $1 and bucket > now() - interval '30 days'
         group by 1 order by 1",
    )
    .bind(app_id)
    .fetch_all(db)
    .await?;
    Ok(Players {
        playing_now,
        peak_this_month: i64::from(month_peak.unwrap_or(0)).max(playing_now),
        daily_peaks,
    })
}

/// Records the current player count of every app into its 5-minute bucket. Idempotent, so
/// several API instances can run it.
pub async fn sample(db: &sqlx::PgPool) -> anyhow::Result<u64> {
    let sql = format!(
        "insert into pulse_counts (app_id, bucket, players)
         select app_id, to_timestamp(floor(extract(epoch from now()) / {BUCKET_SECONDS}) * {BUCKET_SECONDS}), players
         from ({LIVE_PLAYERS_SQL}) live
         on conflict (app_id, bucket) do update set players = greatest(pulse_counts.players, excluded.players)"
    );
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(LIVE_WINDOW)
        .execute(db)
        .await?
        .rows_affected())
}

/// Deletes raw sessions after a day (SECURITY §10). Counts and playtime are kept.
pub async fn delete_old_sessions(db: &sqlx::PgPool) -> anyhow::Result<u64> {
    Ok(sqlx::query(
        "delete from pulse_sessions where issued_at < now() - interval '24 hours'
           and (last_beat_at is null or last_beat_at < now() - $1 or ended_at is not null)",
    )
    .bind(SESSION_TIMEOUT)
    .execute(db)
    .await?
    .rows_affected())
}

/// Runs forever: samples player counts every minute (each bucket keeps its highest sample).
pub async fn run_sampler(state: AppState) {
    loop {
        if let Err(err) = sample(&state.db).await {
            tracing::error!(error = ?err, "pulse sampling failed");
        }
        tokio::time::sleep(StdDuration::from_secs(60)).await;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    share_playtime: Option<bool>,
    /// "Recommendations based on my activity" (foryou.rs).
    personalize: Option<bool>,
    /// "Show my votes and reviews on my profile" (reviews.rs).
    profile_public: Option<bool>,
}

/// `PATCH /v1/me`: the user's own settings.
pub(crate) async fn update_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Settings>,
) -> ApiResult<StatusCode> {
    if let Some(share) = body.share_playtime {
        sqlx::query("update users set share_playtime = $2 where id = $1")
            .bind(user.id)
            .bind(share)
            .execute(&state.db)
            .await?;
    }
    if let Some(on) = body.personalize {
        sqlx::query("update users set personalize = $2 where id = $1")
            .bind(user.id)
            .bind(on)
            .execute(&state.db)
            .await?;
    }
    if let Some(on) = body.profile_public {
        sqlx::query("update users set profile_public = $2 where id = $1")
            .bind(user.id)
            .bind(on)
            .execute(&state.db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
