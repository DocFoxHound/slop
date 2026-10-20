//! A user's own account: downloading their data and closing the account (DESIGN §4;
//! LEGAL-GUIDANCE PRIV-3; METRICS §6).
//!
//! **Closing** (decided 2026-10-20) bans the sign-in and freezes what others rely on, rather
//! than erasing it:
//! - the sign-in is refused from then on (the provider account ID stays, only to refuse it);
//! - the email and personal settings are removed, and the library, playtime and live sessions
//!   deleted;
//! - votes are re-keyed to a random ID, so ratings stay honest but aren't linked to anyone;
//! - a creative's page is frozen: still readable (its reviews and credits are other people's
//!   record too) but it can't take new work, and it leaves the People directory;
//! - the maker's apps leave the store (installs keep working), since nobody can answer for them;
//! - agreements, job records, reviews and moderation records stay as they were.
//!
//! An account with an unfinished People job, or a payment it owes, can't be closed until that's
//! settled: the other side relies on it.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::{self, CurrentUser},
    error::{ApiError, ApiResult},
};

/// What the user types to confirm.
pub const CONFIRMATION: &str = "close my account";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/export", get(export))
        .route("/me/close", post(close))
}

/// Each part of the export: a name and a query returning one JSON value for user `$1`.
const EXPORT: &[(&str, &str)] = &[
    (
        "account",
        "select to_jsonb(u) - 'is_admin' from users u where u.id = $1",
    ),
    (
        "sign_ins",
        "select coalesce(jsonb_agg(jsonb_build_object('provider', provider, 'account_id', provider_subject,
                'linked_at', created_at, 'last_sign_in_at', last_sign_in_at)), '[]')
         from identities where user_id = $1",
    ),
    (
        "sessions",
        "select coalesce(jsonb_agg(jsonb_build_object('kind', kind, 'device', user_agent, 'signed_in_at', created_at,
                'last_seen_at', last_seen_at, 'expires_at', expires_at, 'signed_out_at', revoked_at)
                order by created_at), '[]')
         from sessions where user_id = $1",
    ),
    (
        "agreements_accepted",
        "select coalesce(jsonb_agg(jsonb_build_object('document', kind, 'version', version, 'accepted_at', accepted_at)
                order by accepted_at), '[]')
         from legal_acceptances where user_id = $1",
    ),
    (
        "library",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(l) - 'user_id' - 'app_id')), '[]')
         from library_entries l join apps a on a.id = l.app_id where l.user_id = $1",
    ),
    (
        "interested",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug, 'since', i.created_at)), '[]')
         from interests i join apps a on a.id = i.app_id where i.user_id = $1",
    ),
    (
        "notifications",
        "select coalesce(jsonb_agg(to_jsonb(n) - 'user_id' order by n.created_at), '[]')
         from notifications n where n.user_id = $1",
    ),
    (
        "playtime",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(p) - 'user_id' - 'app_id')), '[]')
         from user_app_usage p join apps a on a.id = p.app_id where p.user_id = $1",
    ),
    (
        "votes",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(v) - 'user_id' - 'app_id')), '[]')
         from votes v join apps a on a.id = v.app_id where v.user_id = $1",
    ),
    (
        "vote_history",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(v) - 'user_id' - 'app_id')
                order by v.created_at), '[]')
         from vote_events v join apps a on a.id = v.app_id where v.user_id = $1",
    ),
    (
        "apps",
        "select coalesce(jsonb_agg(to_jsonb(a) - 'maker_id' order by a.created_at), '[]')
         from apps a where a.maker_id = $1",
    ),
    (
        "people_profile",
        "select to_jsonb(c) - 'user_id' from creatives c where c.user_id = $1",
    ),
    (
        "jobs",
        "select coalesce(jsonb_agg(to_jsonb(j)
                || jsonb_build_object(
                     'events', (select coalesce(jsonb_agg(to_jsonb(e) - 'job_id' order by e.created_at), '[]')
                                from job_events e where e.job_id = j.id),
                     'payments', (select coalesce(jsonb_agg(to_jsonb(p) - 'job_id'), '[]')
                                  from job_payments p where p.job_id = j.id),
                     'changes', (select coalesce(jsonb_agg(to_jsonb(c) - 'job_id' order by c.number), '[]')
                                 from job_changes c where c.job_id = j.id),
                     'messages', (select coalesce(jsonb_agg(to_jsonb(m) - 'job_id' order by m.created_at), '[]')
                                  from job_messages m where m.job_id = j.id),
                     'problems', (select coalesce(jsonb_agg(to_jsonb(p) - 'job_id'
                                     || jsonb_build_object('proposals',
                                          (select coalesce(jsonb_agg(to_jsonb(x) - 'problem_id'), '[]')
                                           from job_problem_proposals x where x.problem_id = p.id))), '[]')
                                  from job_problems p where p.job_id = j.id))
                order by j.created_at), '[]')
         from jobs j where $1 in (j.client_id, j.creative_id)",
    ),
    (
        "donations",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(d) - 'donor_id' - 'app_id')
                order by d.created_at), '[]')
         from donations d join apps a on a.id = d.app_id where d.donor_id = $1",
    ),
    (
        "server_donations",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(s) - 'donor_id' - 'app_id')
                order by s.created_at), '[]')
         from server_donations s join apps a on a.id = s.app_id where s.donor_id = $1",
    ),
    (
        "donation_account",
        "select to_jsonb(d) - 'user_id' from donation_accounts d where d.user_id = $1",
    ),
    (
        "app_reviews",
        "select coalesce(jsonb_agg(jsonb_build_object('app', a.slug) || (to_jsonb(r) - 'user_id' - 'app_id')
                order by r.created_at), '[]')
         from app_reviews r join apps a on a.id = r.app_id where r.user_id = $1",
    ),
    (
        "reviews_written",
        "select coalesce(jsonb_agg(to_jsonb(r) - 'client_id'), '[]') from creative_reviews r where r.client_id = $1",
    ),
    (
        "reviews_received",
        "select coalesce(jsonb_agg(to_jsonb(r) - 'creative_id'), '[]') from creative_reviews r where r.creative_id = $1",
    ),
    (
        "reports_sent",
        "select coalesce(jsonb_agg(jsonb_build_object('reference', reference, 'about', subject_label,
                'reason', reason, 'details', details, 'status', status, 'outcome', outcome,
                'sent_at', created_at, 'decided_at', decided_at) order by created_at), '[]')
         from reports where reporter_id = $1",
    ),
    (
        "moderation_decisions",
        "select coalesce(jsonb_agg(to_jsonb(d) - 'user_id' - 'decided_by' - 'appeal_decided_by'
                order by created_at), '[]')
         from moderation_decisions d where d.user_id = $1",
    ),
];

/// Everything linked to the account, as one JSON file (GDPR Arts. 15 and 20).
async fn export(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    let mut out = serde_json::Map::new();
    out.insert(
        "about".into(),
        json!("Everything Slop holds that's linked to your account, downloaded from your account page. \
               Reports other people made about you aren't included, to protect them; ask us if you need them."),
    );
    out.insert(
        "exported_at".into(),
        json!(
            time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        ),
    );
    for (name, sql) in EXPORT {
        let value: Option<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(*sql))
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
        out.insert((*name).into(), value.unwrap_or(Value::Null));
    }
    let body = serde_json::to_string_pretty(&Value::Object(out)).map_err(anyhow::Error::from)?;
    let mut res = body.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"slop-data.json\""),
    );
    Ok(res)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseBody {
    confirm: String,
}

/// What stops an account being closed now, in words for the user.
pub async fn blockers(db: &sqlx::PgPool, user_id: Uuid) -> sqlx::Result<Vec<String>> {
    let open_jobs: i64 = sqlx::query_scalar(
        "select count(*) from jobs where $1 in (client_id, creative_id)
           and status not in ('completed', 'declined', 'cancelled', 'withdrawn')",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let owed: i64 = sqlx::query_scalar(
        "select count(*) from job_payments p join jobs j on j.id = p.job_id
         where j.client_id = $1 and p.status = 'due'",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let problems: i64 = sqlx::query_scalar(
        "select count(*) from job_problems p join jobs j on j.id = p.job_id
         where $1 in (j.client_id, j.creative_id) and p.status = 'open'",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let giving: i64 = sqlx::query_scalar(
        "select count(*) from server_donations where donor_id = $1 and status in ('pending', 'active')",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let receiving: i64 = sqlx::query_scalar(
        "select count(*) from server_donations s join apps a on a.id = s.app_id
         where a.maker_id = $1 and s.status in ('pending', 'active')",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let mut out = Vec::new();
    if giving > 0 {
        out.push("you give a monthly server donation: cancel it first".into());
    }
    if receiving > 0 {
        out.push(format!(
            "{receiving} people give monthly to your apps' servers: mark the servers shut down first, which cancels their donations"
        ));
    }
    if open_jobs > 0 {
        out.push(format!(
            "{open_jobs} People job{} still open: finish, cancel or withdraw {} first",
            if open_jobs == 1 { " is" } else { "s are" },
            if open_jobs == 1 { "it" } else { "them" }
        ));
    }
    if owed > 0 {
        out.push("you owe a payment on a finished People job: pay it first".into());
    }
    if problems > 0 {
        out.push("a problem on a People job is still open: settle or withdraw it first".into());
    }
    Ok(out)
}

async fn close(
    State(state): State<AppState>,
    user: CurrentUser,
    jar: CookieJar,
    Json(body): Json<CloseBody>,
) -> ApiResult<(StatusCode, CookieJar)> {
    if body.confirm.trim().to_lowercase() != CONFIRMATION {
        return Err(ApiError::invalid(
            "confirm",
            format!("type \"{CONFIRMATION}\" to confirm"),
        ));
    }
    let blocked = blockers(&state.db, user.id).await?;
    if !blocked.is_empty() {
        return Err(ApiError::Conflict(format!(
            "You can't close your account yet: {}.",
            blocked.join("; ")
        )));
    }
    close_account(&state.db, user.id, "the account's owner").await?;
    let jar = jar.remove(session::removal_cookie(
        &state.config,
        session::cookie_name(&state.config),
    ));
    Ok((StatusCode::NO_CONTENT, jar))
}

/// Closes an account (see the module comment). Returns the apps taken out of the store.
pub async fn close_account(
    db: &sqlx::PgPool,
    user_id: Uuid,
    actor: &str,
) -> anyhow::Result<Vec<String>> {
    let mut tx = db.begin().await?;
    let closed = sqlx::query(
        "update users set closed_at = now(), email = null, personalize = null, share_playtime = false,
                is_admin = false
         where id = $1 and closed_at is null",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    anyhow::ensure!(closed == 1, "that account is already closed");
    sqlx::query("update sessions set revoked_at = now() where user_id = $1 and revoked_at is null")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from admin_tokens where user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    // One random ID for all of them, so each app still counts this person's vote once.
    let anonymous = Uuid::new_v4();
    for table in ["votes", "vote_events"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {table} set user_id = $2 where user_id = $1"
        )))
        .bind(user_id)
        .bind(anonymous)
        .execute(&mut *tx)
        .await?;
    }
    for table in [
        "library_entries",
        "interests",
        "app_reviews",
        "user_app_usage",
        "pulse_sessions",
        "notifications",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "delete from {table} where user_id = $1"
        )))
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("update creatives set taking_work = false, updated_at = now() where user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let delisted: Vec<String> = sqlx::query_scalar(
        "update apps set status = 'delisted', updated_at = now()
         where maker_id = $1 and status = 'listed' returning slug",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        actor,
        "account.close",
        &user_id.to_string(),
        json!({ "apps_delisted": delisted }),
    )
    .await?;
    tx.commit().await?;
    Ok(delisted)
}
