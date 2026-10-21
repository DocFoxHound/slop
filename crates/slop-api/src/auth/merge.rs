// SPDX-License-Identifier: AGPL-3.0-or-later
//! Combining two Slop accounts (DESIGN §4).
//!
//! When someone links a sign-in that already has its own Slop account, signing in to it proves
//! they control that account too. After they confirm, everything moves into the account they're
//! signed in to: apps, library, playtime, votes and reviews, friends, chats, teams, People jobs,
//! donations, notifications, reports and decisions. Where both had the same thing (both voted on
//! an app, both in a team), the signed-in account's stays. The other account is closed and points
//! at this one; its sign-ins now open this account.
//!
//! Not combined, so nothing is lost or mixed up:
//! - two People pages, or two Stripe accounts for donations: the merge waits until one goes;
//! - a suspended or closed account: never combined (it would undo the decision);
//! - chat history, which only exists on the devices: the other account's chat devices are
//!   removed, and its chats continue on this account's devices from now on.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use serde_json::json;
use sqlx::{PgConnection, Row};
use time::Duration;
use uuid::Uuid;

use super::session::{CurrentUser, removal_cookie};
use crate::{
    AppState,
    error::{ApiError, ApiResult},
    tokens::hash_token,
};

pub const COOKIE: &str = "slop_combine";
pub const CONFIRM_WITHIN: Duration = Duration::minutes(15);

pub fn routes() -> Router<AppState> {
    Router::new().route("/auth/combine", get(preview).post(confirm).delete(cancel))
}

#[derive(Serialize)]
struct Preview {
    provider: String,
    /// The other account, as it would come in.
    name: String,
    apps: i64,
    library: i64,
    votes: i64,
    friends: i64,
    teams: i64,
    jobs: i64,
    chats: i64,
    /// Why it can't be combined, if it can't.
    blocked: Option<String>,
}

async fn pending(
    state: &AppState,
    jar: &CookieJar,
    user: &CurrentUser,
) -> ApiResult<(Uuid, String, String)> {
    let token = jar.get(COOKIE).ok_or(ApiError::NotFound)?;
    let row: Option<(Uuid, Uuid, Uuid, String, String)> = sqlx::query_as(
        "select into_user_id, from_user_id, session_id, provider, provider_subject from pending_merges
         where token_hash = $1 and expires_at > now()",
    )
    .bind(hash_token(token.value()))
    .fetch_optional(&state.db)
    .await?;
    match row {
        Some((into, from, session, provider, subject))
            if into == user.id && session == user.session_id =>
        {
            Ok((from, provider, subject))
        }
        _ => Err(ApiError::NotFound),
    }
}

/// Why two accounts can't be combined, if they can't.
async fn blocked(conn: &mut PgConnection, into: Uuid, from: Uuid) -> sqlx::Result<Option<String>> {
    let (closed, suspended, people, stripe, admin): (bool, bool, bool, bool, bool) =
        sqlx::query_as(
            "select bool_or(closed_at is not null and id = $2),
                bool_or(suspended_at is not null),
                (select count(*) = 2 from creatives where user_id in ($1, $2)),
                (select count(*) = 2 from donation_accounts where user_id in ($1, $2)),
                bool_or(is_admin and id = $2)
         from users where id in ($1, $2)",
        )
        .bind(into)
        .bind(from)
        .fetch_one(conn)
        .await?;
    Ok(if closed {
        Some("that account is closed".into())
    } else if suspended {
        Some(
            "one of the accounts is suspended by a moderation decision, so they can't be combined"
                .into(),
        )
    } else if people {
        Some("both accounts have a People page. Close one of them first (or ask support)".into())
    } else if stripe {
        Some("both accounts receive donations through Stripe. Disconnect one first".into())
    } else if admin {
        Some("staff accounts can't be combined into another".into())
    } else {
        None
    })
}

async fn preview(
    State(state): State<AppState>,
    jar: CookieJar,
    user: CurrentUser,
) -> ApiResult<Json<Preview>> {
    let (from, provider, _) = pending(&state, &jar, &user).await?;
    let mut conn = state.db.acquire().await?;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "select u.display_name,
                (select count(*) from apps where maker_id = $1) as apps,
                (select count(*) from library_entries where user_id = $1) as library,
                (select count(*) from votes where user_id = $1) as votes,
                (select count(*) from friends where $1 in (user_a, user_b)) as friends,
                (select count(*) from team_members where user_id = $1) as teams,
                (select count(*) from jobs where $1 in (client_id, creative_id)) as jobs,
                (select count(*) from ({}) p where p.user_id = $1) as chats
         from users u where u.id = $1",
        crate::chat::PARTICIPANTS
    )))
    .bind(from)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(Preview {
        provider,
        name: row.get("display_name"),
        apps: row.get("apps"),
        library: row.get("library"),
        votes: row.get("votes"),
        friends: row.get("friends"),
        teams: row.get("teams"),
        jobs: row.get("jobs"),
        chats: row.get("chats"),
        blocked: blocked(&mut conn, user.id, from).await?,
    }))
}

async fn cancel(
    State(state): State<AppState>,
    jar: CookieJar,
) -> ApiResult<(StatusCode, CookieJar)> {
    if let Some(token) = jar.get(COOKIE) {
        sqlx::query("delete from pending_merges where token_hash = $1")
            .bind(hash_token(token.value()))
            .execute(&state.db)
            .await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        jar.remove(removal_cookie(&state.config, COOKIE)),
    ))
}

async fn confirm(
    State(state): State<AppState>,
    jar: CookieJar,
    user: CurrentUser,
) -> ApiResult<(StatusCode, CookieJar)> {
    let (from, _, _) = pending(&state, &jar, &user).await?;
    let mut tx = state.db.begin().await?;
    if let Some(why) = blocked(&mut tx, user.id, from).await? {
        return Err(ApiError::Conflict(why));
    }
    combine(&mut tx, user.id, from)
        .await
        .map_err(ApiError::Internal)?;
    crate::admin::audit(
        &mut tx,
        &user.display_name,
        "account.combine",
        &user.id.to_string(),
        json!({ "from": from }),
    )
    .await?;
    sqlx::query("delete from pending_merges where from_user_id = $1 or into_user_id = $1")
        .bind(from)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::NO_CONTENT,
        jar.remove(removal_cookie(&state.config, COOKIE)),
    ))
}

/// Tables that name a user without a foreign key.
const UNREFERENCED: &[(&str, &str)] = &[("votes", "user_id"), ("vote_events", "user_id")];

/// Moves everything of `from` into `into`, then closes `from` (pointing at `into`). Where both
/// have the same thing, `into`'s stays.
pub async fn combine(tx: &mut PgConnection, into: Uuid, from: Uuid) -> anyhow::Result<()> {
    // Rows with rules of their own first.
    for sql in [
        // Sessions of the other account end; its sign-ins open this one from now on.
        "update sessions set revoked_at = now() where user_id = $2 and revoked_at is null",
        // Friends: each of theirs becomes ours, except ourselves.
        "insert into friends (user_a, user_b, since)
         select least($1, other), greatest($1, other), since from (
             select case when user_a = $2 then user_b else user_a end as other, since
             from friends where $2 in (user_a, user_b)) f
         where other <> $1 on conflict do nothing",
        "delete from friends where $2 in (user_a, user_b)",
        "insert into friend_requests (from_id, to_id, created_at)
         select case when from_id = $2 then $1 else from_id end, case when to_id = $2 then $1 else to_id end, created_at
         from friend_requests where $2 in (from_id, to_id) and $1 not in (from_id, to_id)
         on conflict do nothing",
        "delete from friend_requests where $2 in (from_id, to_id)",
        "insert into user_blocks (user_id, blocked_id, created_at)
         select case when user_id = $2 then $1 else user_id end, case when blocked_id = $2 then $1 else blocked_id end, created_at
         from user_blocks where $2 in (user_id, blocked_id) and $1 not in (user_id, blocked_id)
         on conflict do nothing",
        "delete from user_blocks where $2 in (user_id, blocked_id)",
        // Chat: the other account's devices go (their history is on those devices); its chats
        // continue on this account's devices. A direct chat with someone both chatted with keeps
        // this account's.
        "update chat_devices set status = 'revoked', revoked_at = now() where user_id = $2 and status <> 'revoked'",
        "delete from chat_key_packages k using chat_devices d where d.id = k.device_id and d.user_id = $2",
        "delete from chat_conversations where kind = 'self' and pair_key = $2::text",
        "delete from chat_conversations c where c.kind = 'direct' and c.pair_key like '%' || $2::text || '%'
           and exists (select 1 from chat_conversations d where d.kind = 'direct' and d.pair_key =
                 least($1::text, replace(replace(c.pair_key, $2::text, ''), ':', ''))
                 || ':' || greatest($1::text, replace(replace(c.pair_key, $2::text, ''), ':', '')))",
        "delete from chat_conversations c where c.kind = 'direct' and c.pair_key like '%' || $2::text || '%'
           and c.pair_key like '%' || $1::text || '%'",
        "update chat_conversations c set pair_key =
             least($1::text, replace(replace(c.pair_key, $2::text, ''), ':', ''))
             || ':' || greatest($1::text, replace(replace(c.pair_key, $2::text, ''), ':', ''))
         where c.kind = 'direct' and c.pair_key like '%' || $2::text || '%'",
        "delete from chat_backups where user_id = $2",
        "delete from push_subscriptions where user_id = $2",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(into)
            .bind(from)
            .execute(&mut *tx)
            .await?;
    }
    // A People page moves as a copy (other tables point at it by user), then the old one goes.
    let has_page: bool =
        sqlx::query_scalar("select exists (select 1 from creatives where user_id = $1)")
            .bind(from)
            .fetch_one(&mut *tx)
            .await?;
    if has_page {
        // The handle is unique: park the old row's on a placeholder while the copy takes it.
        let handle: String = sqlx::query_scalar("select handle from creatives where user_id = $1")
            .bind(from)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("update creatives set handle = 'moving-' || substr(md5(user_id::text), 1, 12) where user_id = $1")
            .bind(from)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "insert into creatives select (jsonb_populate_record(null::creatives,
                 to_jsonb(c) || jsonb_build_object('user_id', $1::uuid, 'handle', $3::text))).*
             from creatives c where c.user_id = $2",
        )
        .bind(into)
        .bind(from)
        .bind(&handle)
        .execute(&mut *tx)
        .await?;
        move_column_refs(tx, "creatives", "user_id", into, from).await?;
        sqlx::query("delete from creatives where user_id = $1")
            .bind(from)
            .execute(&mut *tx)
            .await?;
    }
    // Everything else that names the user.
    move_column_refs(tx, "users", "id", into, from).await?;
    for (table, column) in UNREFERENCED {
        move_rows(tx, table, column, into, from).await?;
    }
    sqlx::query(
        "update users set closed_at = now(), merged_into = $1, email = null, is_admin = false where id = $2",
    )
    .bind(into)
    .bind(from)
    .execute(&mut *tx)
    .await?;
    // The earlier 18+ confirmation counts for the combined account.
    sqlx::query(
        "update users set adult_confirmed_at = coalesce(adult_confirmed_at, (select adult_confirmed_at from users where id = $2))
         where id = $1",
    )
    .bind(into)
    .bind(from)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Moves every column that references `table(column)` from `from` to `into`.
async fn move_column_refs(
    tx: &mut PgConnection,
    table: &str,
    column: &str,
    into: Uuid,
    from: Uuid,
) -> anyhow::Result<()> {
    let refs: Vec<(String, String)> = sqlx::query_as(
        "select src.relname::text, a.attname::text
         from pg_constraint c
         join pg_class src on src.oid = c.conrelid
         join pg_class dst on dst.oid = c.confrelid
         join pg_attribute a on a.attrelid = c.conrelid and a.attnum = c.conkey[1]
         join pg_attribute b on b.attrelid = c.confrelid and b.attnum = c.confkey[1]
         join pg_namespace n on n.oid = src.relnamespace
         where c.contype = 'f' and dst.relname = $1 and b.attname = $2 and n.nspname = current_schema()
           and array_length(c.conkey, 1) = 1
         order by 1, 2",
    )
    .bind(table)
    .bind(column)
    .fetch_all(&mut *tx)
    .await?;
    for (t, c) in refs {
        // The users table's own references (merged_into) stay as they are.
        if t == "users" {
            continue;
        }
        move_rows(tx, &t, &c, into, from).await?;
    }
    Ok(())
}

fn ident(name: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "unexpected identifier {name:?}"
    );
    Ok(name)
}

/// Moves `table.column` from `from` to `into`. A row that would duplicate one `into` already has
/// (a unique key) is dropped, so `into`'s stays. Any other error stops the merge.
async fn move_rows(
    tx: &mut PgConnection,
    table: &str,
    column: &str,
    into: Uuid,
    from: Uuid,
) -> anyhow::Result<()> {
    let (t, c) = (ident(table)?, ident(column)?);
    sqlx::query("savepoint move_all").execute(&mut *tx).await?;
    let all = sqlx::query(sqlx::AssertSqlSafe(format!(
        "update {t} set {c} = $1 where {c} = $2"
    )))
    .bind(into)
    .bind(from)
    .execute(&mut *tx)
    .await;
    match all {
        Ok(_) => {
            sqlx::query("release savepoint move_all")
                .execute(&mut *tx)
                .await?;
            return Ok(());
        }
        Err(e) if crate::error::is_unique_violation(&e) => {
            sqlx::query("rollback to savepoint move_all")
                .execute(&mut *tx)
                .await?;
        }
        Err(e) => return Err(e.into()),
    }
    // One row at a time.
    let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select ctid::text from {t} where {c} = $1"
    )))
    .bind(from)
    .fetch_all(&mut *tx)
    .await?;
    for ctid in rows {
        sqlx::query("savepoint move_one").execute(&mut *tx).await?;
        let one = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {t} set {c} = $1 where ctid = $2::tid"
        )))
        .bind(into)
        .bind(&ctid)
        .execute(&mut *tx)
        .await;
        match one {
            Ok(_) => {
                sqlx::query("release savepoint move_one")
                    .execute(&mut *tx)
                    .await?;
            }
            Err(e) if crate::error::is_unique_violation(&e) => {
                sqlx::query("rollback to savepoint move_one")
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "delete from {t} where ctid = $1::tid"
                )))
                .bind(&ctid)
                .execute(&mut *tx)
                .await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
