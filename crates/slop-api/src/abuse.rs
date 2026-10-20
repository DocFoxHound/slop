//! Abuse checks (SECURITY §11, METRICS §5). Every hour Slop looks for patterns a person should
//! check, records each once (`abuse_signals`), and the admins see them in the queue and as an
//! alert (ops.rs). Nothing is changed automatically: a reviewer decides, through the normal
//! moderation tools, or dismisses it with a note.
//!
//! - **Vote bursts:** an app getting many votes in a day from accounts made in the last week.
//! - **Review bombs:** an app's 👎 votes in a day far above its usual.
//! - **Vote swings:** many players changing their vote the same way on one app in a day.
//! - **Lookalike names:** an app named almost exactly like another maker's app in the store
//!   (ignoring case, spacing, punctuation and look-alike characters such as 0/o, 1/l).
//! - **Rate limits:** accounts that keep hitting the per-account limits (limits.rs).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin_web::AdminUser,
    error::{ApiError, ApiResult},
    staff::Perm,
};

const EVERY: std::time::Duration = std::time::Duration::from_secs(3600);
/// Votes in a day from new accounts that make a burst (and at least half that day's votes).
const BURST_MIN: i64 = 10;
/// A day's 👎 at least this many, and this many times the usual daily number.
const BOMB_MIN: i64 = 10;
const BOMB_FACTOR: f64 = 5.0;
/// Changes of vote in one direction in a day.
const SWING_MIN: i64 = 10;
/// Times an account hit a limit in a day.
const RATE_LIMIT_HITS: u32 = 20;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/abuse/{id}/dismiss", post(dismiss))
}

/// A title reduced to what a reader would see: lowercase letters and digits, look-alike
/// characters folded together.
pub fn fold(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            let c = match c {
                '0' => 'o',
                '1' | '!' | '|' | 'i' => 'l',
                '3' => 'e',
                '4' | '@' => 'a',
                '5' | '$' => 's',
                '7' => 't',
                '8' => 'b',
                'а' => 'a',
                'е' => 'e',
                'о' => 'o',
                'р' => 'p',
                'с' => 'c',
                'х' => 'x',
                'у' => 'y',
                other => other,
            };
            c.is_alphanumeric().then_some(c)
        })
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Whether two titles would be taken for each other.
pub fn looks_like(a: &str, b: &str) -> bool {
    let (a, b) = (fold(a), fold(b));
    if a.len() < 4 || b.len() < 4 {
        return a == b && !a.is_empty();
    }
    a == b || edit_distance(&a, &b) <= 1
}

struct Found {
    kind: &'static str,
    key: String,
    subject_kind: &'static str,
    subject_id: Uuid,
    label: String,
    detail: Value,
}

async fn record(db: &sqlx::PgPool, f: &Found) -> sqlx::Result<bool> {
    let n = sqlx::query(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, $2, $3, $4, $5, $6, $7) on conflict (key) do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(f.kind)
    .bind(&f.key)
    .bind(f.subject_kind)
    .bind(f.subject_id)
    .bind(&f.label)
    .bind(&f.detail)
    .execute(db)
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// One round of checks. Returns how many new signals it raised.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let db = &state.db;
    let day = OffsetDateTime::now_utc().date().to_string();
    let mut found = Vec::new();

    let bursts: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        "select a.id, a.title,
                count(*) filter (where u.created_at > now() - interval '7 days'),
                count(*)
         from vote_events e join apps a on a.id = e.app_id join users u on u.id = e.user_id
         where e.created_at > now() - interval '24 hours' and e.vote <> 0
         group by a.id, a.title",
    )
    .fetch_all(db)
    .await?;
    for (id, title, new, all) in bursts {
        if new >= BURST_MIN && new * 2 >= all {
            found.push(Found {
                kind: "vote_burst",
                key: format!("vote_burst:{id}:{day}"),
                subject_kind: "app",
                subject_id: id,
                label: title,
                detail: json!({ "votes_from_new_accounts": new, "votes": all, "window": "24 hours" }),
            });
        }
    }

    let bombs: Vec<(Uuid, String, i64, f64)> = sqlx::query_as(
        "select a.id, a.title,
                count(*) filter (where e.created_at > now() - interval '24 hours'),
                (count(*) filter (where e.created_at <= now() - interval '24 hours'))::float8 / 30
         from vote_events e join apps a on a.id = e.app_id
         where e.vote = -1 and e.created_at > now() - interval '31 days'
         group by a.id, a.title",
    )
    .fetch_all(db)
    .await?;
    for (id, title, today, usual) in bombs {
        if today >= BOMB_MIN && (today as f64) >= BOMB_FACTOR * usual.max(1.0) {
            found.push(Found {
                kind: "review_bomb",
                key: format!("review_bomb:{id}:{day}"),
                subject_kind: "app",
                subject_id: id,
                label: title,
                detail: json!({ "slop_votes_today": today, "usual_per_day": (usual * 10.0).round() / 10.0 }),
            });
        }
    }

    let swings: Vec<(Uuid, String, i16, i64)> = sqlx::query_as(
        "select a.id, a.title, e.vote, count(distinct e.user_id)
         from vote_events e join apps a on a.id = e.app_id
         where e.created_at > now() - interval '24 hours' and e.prev_vote is not null
           and e.vote <> 0 and e.vote <> e.prev_vote
         group by a.id, a.title, e.vote",
    )
    .fetch_all(db)
    .await?;
    for (id, title, to, n) in swings {
        if n >= SWING_MIN {
            found.push(Found {
                kind: "vote_swing",
                key: format!("vote_swing:{id}:{to}:{day}"),
                subject_kind: "app",
                subject_id: id,
                label: title,
                detail: json!({ "changed_to": if to == 1 { "not_slop" } else { "slop" }, "players": n, "window": "24 hours" }),
            });
        }
    }

    // New or renamed apps against what's in the store, by other makers.
    let fresh: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "select id, maker_id, title from apps where updated_at > now() - interval '25 hours' and parent_app_id is null",
    )
    .fetch_all(db)
    .await?;
    if !fresh.is_empty() {
        let listed: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
            "select id, maker_id, title, slug from apps where status = 'listed' and parent_app_id is null",
        )
        .fetch_all(db)
        .await?;
        for (id, maker, title) in &fresh {
            if let Some((other_id, _, other_title, other_slug)) = listed
                .iter()
                .find(|(oid, om, ot, _)| oid != id && om != maker && looks_like(title, ot))
            {
                found.push(Found {
                    kind: "lookalike_name",
                    key: format!("lookalike_name:{id}:{other_id}"),
                    subject_kind: "app",
                    subject_id: *id,
                    label: title.clone(),
                    detail: json!({ "looks_like": other_title, "other_app": other_slug }),
                });
            }
        }
    }

    for (user, hits) in state.account_limiter.frequent(RATE_LIMIT_HITS) {
        let name: Option<String> =
            sqlx::query_scalar("select display_name from users where id = $1")
                .bind(user)
                .fetch_optional(db)
                .await?;
        if let Some(name) = name {
            found.push(Found {
                kind: "rate_limited",
                key: format!("rate_limited:{user}:{day}"),
                subject_kind: "account",
                subject_id: user,
                label: name,
                detail: json!({ "limit_hits_today": hits }),
            });
        }
    }

    let mut raised = 0;
    for f in &found {
        if record(db, f).await? {
            raised += 1;
        }
    }
    Ok(raised)
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        match run_once(&state).await {
            Ok(n) if n > 0 => tracing::warn!(signals = n, "abuse checks raised signals"),
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "abuse checks"),
        }
    }
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Signal {
    id: Uuid,
    kind: String,
    subject_kind: String,
    subject_id: Uuid,
    subject_label: String,
    /// The app's address, for apps.
    slug: Option<String>,
    detail: Value,
    #[serde(with = "time::serde::rfc3339")]
    detected_at: OffsetDateTime,
}

/// Open signals, newest first, for the admin queue.
pub async fn open(db: &sqlx::PgPool) -> sqlx::Result<Vec<Signal>> {
    sqlx::query_as(
        "select s.id, s.kind, s.subject_kind, s.subject_id, s.subject_label,
                (select a.slug from apps a where s.subject_kind = 'app' and a.id = s.subject_id) as slug,
                s.detail, s.detected_at
         from abuse_signals s where s.status = 'open' order by s.detected_at desc limit 100",
    )
    .fetch_all(db)
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dismiss {
    /// What the reviewer found, for the record.
    note: String,
    /// true when it was acted on (through the moderation tools), false when it's nothing.
    #[serde(default)]
    acted: bool,
}

async fn dismiss(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Dismiss>,
) -> ApiResult<StatusCode> {
    admin.need(Perm::Moderate)?;
    let note = b.note.trim();
    if note.is_empty() || note.chars().count() > 1000 {
        return Err(ApiError::invalid(
            "note",
            "say what you found, in up to 1000 characters",
        ));
    }
    let mut tx = state.db.begin().await?;
    let n = sqlx::query(
        "update abuse_signals set status = $2, decided_by = $3, decision_note = $4, decided_at = now()
         where id = $1 and status = 'open'",
    )
    .bind(id)
    .bind(if b.acted { "actioned" } else { "dismissed" })
    .bind(&admin.actor)
    .bind(note)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "abuse.close",
        &id.to_string(),
        json!({ "acted": b.acted, "note": note }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookalikes() {
        assert!(looks_like("Minecraft", "M1necraft"));
        assert!(looks_like("Stardew Valley", "stardew-valley!"));
        assert!(looks_like("Stardew Valley", "Stardew Valey"));
        assert!(looks_like("Rоblox", "Roblox"), "Cyrillic o");
        assert!(!looks_like("Cozy Tower", "Crazy Tower"));
        assert!(!looks_like("Pixel Arena", "Pixel Arena 2 Deluxe"));
        assert!(!looks_like("Go", "Gp"), "short names must match exactly");
    }
}
