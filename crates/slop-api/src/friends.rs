// SPDX-License-Identifier: AGPL-3.0-or-later
//! Friends, blocks and who may message whom (DESIGN §19).
//!
//! - **Friends:** one side asks (from the other's profile), the other accepts. Either can remove
//!   the friendship.
//! - **Connections** are friends, members of the same player team, and the two sides of a People
//!   job. Only connections can message each other, unless someone allows messages from anyone.
//! - **The UK under-18 view:** only connections, and "anyone" is locked off (PRIV-6).
//! - **Blocking** someone ends the friendship, refuses their messages and friend requests, and
//!   hides them from the blocker's chats.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/friends", get(list).post(request))
        .route("/me/friends/{id}", delete(remove))
        .route("/me/friends/{id}/accept", post(accept))
        .route("/me/blocks", get(blocks))
        .route("/me/blocks/{id}", put(block).delete(unblock))
        .route("/me/chat-settings", get(settings).put(save_settings))
        .route("/users/{id}/connection", get(connection))
}

/// SQL: whether user `$a` is in the UK under-18 view (no age check exists yet, AGE-6).
fn under18(user: &str) -> String {
    format!(
        "coalesce((select cp.under18_view_default from users uu join country_policies cp on cp.country_code = uu.country_code
                   where uu.id = {user}), true)"
    )
}

/// Whether two people are connected: friends, in the same player team, or the two sides of a
/// People job.
pub async fn connected(conn: &mut PgConnection, a: Uuid, b: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from friends where user_a = least($1, $2) and user_b = greatest($1, $2))
             or exists (select 1 from team_members x join team_members y on y.team_id = x.team_id
                        join teams t on t.id = x.team_id
                        where x.user_id = $1 and y.user_id = $2 and t.kind = 'player')
             or exists (select 1 from jobs j where (j.client_id = $1 and j.creative_id = $2)
                                                or (j.client_id = $2 and j.creative_id = $1))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(conn)
    .await
}

/// Whether either has blocked the other.
pub async fn blocked(conn: &mut PgConnection, a: Uuid, b: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from user_blocks where (user_id = $1 and blocked_id = $2)
                                                  or (user_id = $2 and blocked_id = $1))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(conn)
    .await
}

/// Whether `from` may start a chat with `to`, or add them to one: not blocked, an open account,
/// and connected, or `to` takes messages from anyone (never in either's under-18 view).
pub async fn may_message(conn: &mut PgConnection, from: Uuid, to: Uuid) -> sqlx::Result<bool> {
    if from == to || blocked(conn, from, to).await? {
        return Ok(false);
    }
    let open: bool = sqlx::query_scalar(
        "select exists (select 1 from users where id = $1 and closed_at is null and suspended_at is null)",
    )
    .bind(to)
    .fetch_one(&mut *conn)
    .await?;
    if !open {
        return Ok(false);
    }
    if connected(conn, from, to).await? {
        return Ok(true);
    }
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select u.chat_from_anyone and not {} and not {} from users u where u.id = $2",
        under18("$1"),
        under18("$2")
    )))
    .bind(from)
    .bind(to)
    .fetch_one(conn)
    .await
}

/// A user from a profile link (…/u/{id}) or an ID.
pub fn parse_user(value: &str) -> ApiResult<Uuid> {
    let value = value.trim().trim_end_matches('/');
    let id = value.rsplit('/').next().unwrap_or(value);
    id.parse()
        .map_err(|_| ApiError::invalid("user", "paste their profile link"))
}

#[derive(Serialize, sqlx::FromRow)]
struct Person {
    id: Uuid,
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    since: OffsetDateTime,
}

#[derive(Serialize)]
struct Friends {
    friends: Vec<Person>,
    /// Asking to be your friend.
    incoming: Vec<Person>,
    /// You asked; waiting for them.
    outgoing: Vec<Person>,
}

async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Friends>> {
    let friends = sqlx::query_as(
        "select u.id, u.display_name as name, f.since from friends f
         join users u on u.id = case when f.user_a = $1 then f.user_b else f.user_a end
         where $1 in (f.user_a, f.user_b) and u.closed_at is null order by lower(u.display_name)",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let incoming = sqlx::query_as(
        "select u.id, u.display_name as name, r.created_at as since from friend_requests r
         join users u on u.id = r.from_id where r.to_id = $1 and u.closed_at is null order by r.created_at desc",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let outgoing = sqlx::query_as(
        "select u.id, u.display_name as name, r.created_at as since from friend_requests r
         join users u on u.id = r.to_id where r.from_id = $1 and u.closed_at is null order by r.created_at desc",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Friends {
        friends,
        incoming,
        outgoing,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestBody {
    /// A profile link or user ID.
    user: String,
}

/// Friend requests a day, so they can't be used to spam.
const REQUESTS_PER_DAY: i64 = 30;

#[derive(Serialize)]
struct Requested {
    /// "requested", or "friends" when they had already asked you.
    status: &'static str,
}

async fn request(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<RequestBody>,
) -> ApiResult<Json<Requested>> {
    crate::limits::check(&state, user.id, crate::limits::Action::FriendRequest)?;
    let to = parse_user(&b.user)?;
    if to == user.id {
        return Err(ApiError::invalid("user", "that's you"));
    }
    let mut tx = state.db.begin().await?;
    let name: Option<String> =
        sqlx::query_scalar("select display_name from users where id = $1 and closed_at is null")
            .bind(to)
            .fetch_optional(&mut *tx)
            .await?;
    // Blocked either way looks the same as no such person.
    if name.is_none() || blocked(&mut tx, user.id, to).await? {
        return Err(ApiError::NotFound);
    }
    let theirs = sqlx::query("delete from friend_requests where from_id = $1 and to_id = $2")
        .bind(to)
        .bind(user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if theirs > 0 {
        make_friends(&mut tx, user.id, to).await?;
        tell(
            &mut tx,
            to,
            &format!("{} accepted your friend request", user.display_name),
            user.id,
        )
        .await?;
        tx.commit().await?;
        return Ok(Json(Requested { status: "friends" }));
    }
    let already: bool = sqlx::query_scalar(
        "select exists (select 1 from friends where user_a = least($1, $2) and user_b = greatest($1, $2))",
    )
    .bind(user.id)
    .bind(to)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Ok(Json(Requested { status: "friends" }));
    }
    let today: i64 = sqlx::query_scalar(
        "select count(*) from friend_requests where from_id = $1 and created_at > now() - interval '1 day'",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if today >= REQUESTS_PER_DAY {
        return Err(ApiError::Conflict(
            "that's a lot of friend requests for one day; try again tomorrow".into(),
        ));
    }
    let new = sqlx::query(
        "insert into friend_requests (from_id, to_id) values ($1, $2) on conflict do nothing",
    )
    .bind(user.id)
    .bind(to)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if new > 0 {
        tell(
            &mut tx,
            to,
            &format!("{} wants to be friends", user.display_name),
            user.id,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(Requested {
        status: "requested",
    }))
}

async fn tell(conn: &mut PgConnection, to: Uuid, title: &str, from: Uuid) -> sqlx::Result<()> {
    notify::send(
        conn,
        Notice {
            user_id: to,
            category: "friends",
            title: title.to_string(),
            body: "Friends can message each other in the Slop app.".into(),
            link: format!("/friends#{from}"),
            collapse_key: Some(format!("friend:{from}")),
        },
    )
    .await
}

async fn make_friends(conn: &mut PgConnection, a: Uuid, b: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        "insert into friends (user_a, user_b) values (least($1, $2), greatest($1, $2)) on conflict do nothing",
    )
    .bind(a)
    .bind(b)
    .execute(conn)
    .await?;
    Ok(())
}

async fn accept(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(from): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let found = sqlx::query("delete from friend_requests where from_id = $1 and to_id = $2")
        .bind(from)
        .bind(user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if found == 0 {
        return Err(ApiError::NotFound);
    }
    make_friends(&mut tx, user.id, from).await?;
    tell(
        &mut tx,
        from,
        &format!("{} accepted your friend request", user.display_name),
        user.id,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a friend, declines their request, or cancels yours.
async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(other): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    unfriend(&mut tx, user.id, other).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfriend(conn: &mut PgConnection, a: Uuid, b: Uuid) -> sqlx::Result<()> {
    sqlx::query("delete from friends where user_a = least($1, $2) and user_b = greatest($1, $2)")
        .bind(a)
        .bind(b)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "delete from friend_requests where (from_id = $1 and to_id = $2) or (from_id = $2 and to_id = $1)",
    )
    .bind(a)
    .bind(b)
    .execute(conn)
    .await?;
    Ok(())
}

async fn blocks(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<Person>>> {
    Ok(Json(
        sqlx::query_as(
            "select u.id, u.display_name as name, b.created_at as since from user_blocks b
             join users u on u.id = b.blocked_id where b.user_id = $1 order by b.created_at desc",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn block(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(other): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if other == user.id {
        return Err(ApiError::invalid("user", "that's you"));
    }
    let mut tx = state.db.begin().await?;
    let exists: bool = sqlx::query_scalar("select exists (select 1 from users where id = $1)")
        .bind(other)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(ApiError::NotFound);
    }
    sqlx::query(
        "insert into user_blocks (user_id, blocked_id) values ($1, $2) on conflict do nothing",
    )
    .bind(user.id)
    .bind(other)
    .execute(&mut *tx)
    .await?;
    unfriend(&mut tx, user.id, other).await?;
    // Their direct chat disappears from the blocker's list (chat.rs leaves blocked people out).
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(other): Path<Uuid>,
) -> ApiResult<StatusCode> {
    sqlx::query("delete from user_blocks where user_id = $1 and blocked_id = $2")
        .bind(user.id)
        .bind(other)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, Deserialize)]
struct ChatSettings {
    /// Accept messages from people you aren't connected to.
    from_anyone: bool,
    /// The UK under-18 view: "anyone" is locked off.
    #[serde(default)]
    locked: bool,
}

async fn load_settings(db: &PgPool, user: Uuid) -> sqlx::Result<ChatSettings> {
    let (from_anyone, locked): (bool, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select chat_from_anyone, {} from users where id = $1",
        under18("$1")
    )))
    .bind(user)
    .fetch_one(db)
    .await?;
    Ok(ChatSettings {
        from_anyone: from_anyone && !locked,
        locked,
    })
}

async fn settings(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<ChatSettings>> {
    Ok(Json(load_settings(&state.db, user.id).await?))
}

async fn save_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<ChatSettings>,
) -> ApiResult<Json<ChatSettings>> {
    let current = load_settings(&state.db, user.id).await?;
    if b.from_anyone && current.locked {
        return Err(ApiError::Conflict(
            "in the under-18 view only friends, teammates and People job partners can message you"
                .into(),
        ));
    }
    sqlx::query("update users set chat_from_anyone = $2 where id = $1")
        .bind(user.id)
        .bind(b.from_anyone)
        .execute(&state.db)
        .await?;
    Ok(Json(load_settings(&state.db, user.id).await?))
}

#[derive(Serialize)]
struct Connection {
    /// friends, requested (you asked), incoming (they asked), or none.
    friend: &'static str,
    blocked: bool,
    /// Whether you may message them.
    can_message: bool,
}

/// The friend and message buttons on someone's profile.
async fn connection(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(other): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    let mut conn = state.db.acquire().await?;
    let (friends, requested, incoming, blocked_them): (bool, bool, bool, bool) = sqlx::query_as(
        "select exists (select 1 from friends where user_a = least($1, $2) and user_b = greatest($1, $2)),
                exists (select 1 from friend_requests where from_id = $1 and to_id = $2),
                exists (select 1 from friend_requests where from_id = $2 and to_id = $1),
                exists (select 1 from user_blocks where user_id = $1 and blocked_id = $2)",
    )
    .bind(user.id)
    .bind(other)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(Connection {
        friend: if friends {
            "friends"
        } else if requested {
            "requested"
        } else if incoming {
            "incoming"
        } else {
            "none"
        },
        blocked: blocked_them,
        can_message: may_message(&mut conn, user.id, other).await?,
    }))
}
