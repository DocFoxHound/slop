//! Chat's delivery service (DESIGN §19, SECURITY §8.5).
//!
//! Messages are end-to-end encrypted with MLS (RFC 9420) in the Slop app (`slop-client`'s
//! `chat`), so Slop can't read them. This module only:
//!
//! - **keeps the devices list**: each device's public signing key and its public MLS key
//!   packages. A user's first device stands alone; later ones are approved by an existing device,
//!   which signs the new key. Slop checks that signature and can't approve a device itself, so a
//!   stolen Slop session can't add a device that reads chats. "Start over" (every device lost)
//!   needs a fresh sign-in and is shown to everyone the user chats with.
//! - **orders each chat's messages.** A commit (adding or removing devices) must build on the
//!   epoch Slop last saw and moves it on; a message must use the current epoch. So everyone
//!   processes the same sequence.
//! - **delivers**: encrypted messages, commits and Welcomes wait here until fetched, for 30
//!   days at most. A device long-polls `/chat/sync` for whatever is new.
//! - **decides who may chat**: members of a chat, and who may start one (`friends::may_message`).
//!
//! What Slop sees: who is in which chat, which device sent each message and when, and sizes.
//! Message franking lets a reported message be checked as genuine without Slop reading others.

use std::{collections::HashMap, sync::Arc, time::Duration as StdDuration};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    friends,
    notify::{self, Notice},
};

/// The longest encrypted message (pictures and files go as attachments).
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
/// The largest encrypted attachment.
pub const MAX_ATTACHMENT_BYTES: usize = 16 * 1024 * 1024;
/// Commits carry a Welcome for each added device; a big group's can be large.
pub const MAX_COMMIT_BODY: usize = 2 * 1024 * 1024;
/// People in a group chat.
pub const MAX_GROUP: i64 = 50;
/// How long a long-poll waits for something new.
const LONG_POLL: StdDuration = StdDuration::from_secs(25);
/// Messages, Welcomes and attachments are deleted after this long.
pub const KEEP_DAYS: i64 = 30;
/// Key packages a device keeps uploaded; it tops up below half.
pub const KEY_PACKAGES: i64 = 20;
/// What a device signs to approve another of the user's devices.
pub const APPROVAL_CONTEXT: &str = "slop-chat-device-approval:v1";

/// Wakes the long-polls of the users a change is for.
#[derive(Clone)]
pub struct Hub(
    broadcast::Sender<Arc<Vec<Uuid>>>,
    /// Who is typing where, until when. Kept in memory only, never stored: typing indicators are
    /// the one chat signal that isn't encrypted, and say nothing but "typing".
    Arc<std::sync::Mutex<HashMap<(Uuid, Uuid), tokio::time::Instant>>>,
);

/// How long "typing…" lasts after the last keystroke signal.
pub const TYPING_FOR: StdDuration = StdDuration::from_secs(6);

impl Default for Hub {
    fn default() -> Self {
        Self(broadcast::channel(1024).0, Arc::default())
    }
}

impl Hub {
    fn wake(&self, users: Vec<Uuid>) {
        let _ = self.0.send(Arc::new(users));
    }

    fn set_typing(&self, conversation: Uuid, user: Uuid, typing: bool) {
        let mut map = self.1.lock().unwrap();
        let now = tokio::time::Instant::now();
        map.retain(|_, until| *until > now);
        if typing {
            map.insert((conversation, user), now + TYPING_FOR);
        } else {
            map.remove(&(conversation, user));
        }
    }

    /// (conversation, user) pairs typing now, in these chats, except `me`.
    fn typing(&self, conversations: &[Uuid], me: Uuid) -> Vec<Typing> {
        let now = tokio::time::Instant::now();
        let mut out: Vec<Typing> = self
            .1
            .lock()
            .unwrap()
            .iter()
            .filter(|((c, u), until)| **until > now && *u != me && conversations.contains(c))
            .map(|((c, u), _)| Typing {
                conversation_id: *c,
                user_id: *u,
            })
            .collect();
        out.sort_by_key(|t| (t.conversation_id, t.user_id));
        out
    }
}

#[derive(Serialize, Clone, PartialEq, Eq)]
struct Typing {
    conversation_id: Uuid,
    user_id: Uuid,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/chat/devices", get(my_devices).post(register_device))
        .route("/chat/devices/{id}", delete(revoke_device))
        .route("/chat/devices/{id}/approve", post(approve_device))
        .route("/chat/devices/{id}/start-over", post(start_over))
        .route("/chat/devices/{id}/key-packages", post(add_key_packages))
        .route("/chat/conversations", get(conversations).post(start))
        .route("/chat/conversations/{id}", get(conversation))
        .route("/chat/conversations/{id}/devices", get(member_devices))
        .route(
            "/chat/conversations/{id}/key-packages",
            post(claim_key_packages),
        )
        .route("/chat/conversations/{id}/members", post(add_members))
        .route(
            "/chat/conversations/{id}/members/{user}",
            delete(remove_member),
        )
        .route("/chat/conversations/{id}/read", post(mark_read))
        .route("/chat/conversations/{id}/mute", put(mute))
        .route("/chat/conversations/{id}/hide", post(hide))
        .route("/chat/conversations/{id}/typing", post(typing))
        .route("/chat/attachments/{id}", get(attachment))
        .route("/chat/backup", get(backup).delete(delete_backup))
}

/// Routes with bodies larger than the API's usual 64 KiB: commits and messages.
pub fn message_routes() -> Router<AppState> {
    Router::new()
        .route("/chat/conversations/{id}/commit", post(commit))
        .route("/chat/conversations/{id}/messages", post(send))
}

/// Attachments and backups, up to 16 MiB.
pub fn upload_routes() -> Router<AppState> {
    Router::new()
        .route("/chat/conversations/{id}/attachments", post(upload))
        .route("/chat/backup", put(save_backup))
}

/// The long-poll, which outlives the API's usual 20-second timeout.
pub fn sync_routes() -> Router<AppState> {
    Router::new().route("/chat/sync", get(sync))
}

fn b64(field: &'static str, value: &str, max: usize) -> ApiResult<Vec<u8>> {
    let bytes = B64
        .decode(value)
        .map_err(|_| ApiError::invalid(field, "not base64"))?;
    if bytes.len() > max {
        return Err(ApiError::invalid(field, "too large"));
    }
    Ok(bytes)
}

/// Chat is in the Slop app only (SECURITY §8.5): devices register from desktop sign-ins.
async fn require_app(db: &PgPool, user: &CurrentUser) -> ApiResult<()> {
    let kind: String = sqlx::query_scalar("select kind from sessions where id = $1")
        .bind(user.session_id)
        .fetch_one(db)
        .await?;
    if kind != "desktop" {
        return Err(ApiError::Conflict(
            "chat is in the Slop app only, where messages are encrypted on your device".into(),
        ));
    }
    Ok(())
}

/// The calling device (`x-slop-chat-device`), which must be one of the user's active devices.
pub struct ChatDevice {
    pub id: Uuid,
    pub user: CurrentUser,
}

impl FromRequestParts<AppState> for ChatDevice {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let id: Uuid = parts
            .headers
            .get("x-slop-chat-device")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ApiError::invalid("device", "set up chat on this device first"))?;
        let active: bool = sqlx::query_scalar(
            "update chat_devices set last_seen_at = now() where id = $1 and user_id = $2 and status = 'active'
             returning true",
        )
        .bind(id)
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(false);
        if !active {
            return Err(ApiError::Conflict(
                "this device isn't approved for chat (or was removed)".into(),
            ));
        }
        Ok(Self { id, user })
    }
}

// ---------------------------------------------------------------------------------------------
// Devices

#[derive(Serialize, sqlx::FromRow)]
struct MyDevice {
    id: Uuid,
    name: String,
    status: String,
    approved_by: Option<Uuid>,
    /// The code shown on the device, to compare before approving it.
    #[sqlx(skip)]
    code: String,
    /// Its public signing key, which an approving device signs.
    #[serde(with = "b64_bytes")]
    signature_key: Vec<u8>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_seen_at: OffsetDateTime,
}

/// The short code a device shows and its approver compares: from the device's public key, so a
/// different key shows a different code.
pub fn device_code(signature_key: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let hash = Sha256::digest(signature_key);
    let mut bits = u64::from_be_bytes(hash[..8].try_into().unwrap());
    let mut code = String::new();
    for i in 0..9 {
        if i == 3 || i == 6 {
            code.push('-');
        }
        code.push(ALPHABET[(bits & 31) as usize] as char);
        bits >>= 5;
    }
    code
}

async fn my_devices(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<MyDevice>>> {
    let mut devices: Vec<MyDevice> = sqlx::query_as(
        "select id, name, status, approved_by, signature_key, created_at, last_seen_at from chat_devices
         where user_id = $1 and status <> 'revoked' order by created_at",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    for d in &mut devices {
        d.code = device_code(&d.signature_key);
    }
    Ok(Json(devices))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewDevice {
    /// Chosen by the device, since its MLS credential names it before Slop has seen it.
    id: Uuid,
    name: String,
    /// The device's MLS signing key (Ed25519), base64.
    signature_key: String,
    key_packages: Vec<String>,
    last_resort: String,
}

#[derive(Serialize)]
struct Registered {
    id: Uuid,
    /// active (the user's first device) or pending (an existing device approves it).
    status: String,
    code: String,
}

async fn register_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<NewDevice>,
) -> ApiResult<(StatusCode, Json<Registered>)> {
    require_app(&state.db, &user).await?;
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid("name", "1 to 60 characters"));
    }
    let key = b64("signature_key", &b.signature_key, 32)?;
    if key.len() != 32 {
        return Err(ApiError::invalid("signature_key", "an Ed25519 public key"));
    }
    if b.key_packages.len() > KEY_PACKAGES as usize {
        return Err(ApiError::invalid("key_packages", "too many"));
    }
    let mut tx = state.db.begin().await?;
    let pending: i64 = sqlx::query_scalar(
        "select count(*) from chat_devices where user_id = $1 and status = 'pending'",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if pending >= 5 {
        return Err(ApiError::Conflict(
            "too many devices are waiting for approval; approve or remove them first".into(),
        ));
    }
    let has_active: bool = sqlx::query_scalar(
        "select exists (select 1 from chat_devices where user_id = $1 and status = 'active')",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    let id = b.id;
    let status = if has_active { "pending" } else { "active" };
    let inserted = sqlx::query(
        "insert into chat_devices (id, user_id, name, signature_key, status, approved_at)
         values ($1, $2, $3, $4, $5, case when $5 = 'active' then now() end)",
    )
    .bind(id)
    .bind(user.id)
    .bind(name)
    .bind(&key)
    .bind(status)
    .execute(&mut *tx)
    .await;
    if let Err(err) = inserted {
        if crate::error::is_unique_violation(&err) {
            return Err(ApiError::Conflict(
                "that device or key is already registered".into(),
            ));
        }
        return Err(err.into());
    }
    store_key_packages(&mut tx, id, &b.key_packages, Some(&b.last_resort)).await?;
    tx.commit().await?;
    state.chat_hub.wake(vec![user.id]);
    Ok((
        StatusCode::CREATED,
        Json(Registered {
            id,
            status: status.into(),
            code: device_code(&key),
        }),
    ))
}

async fn store_key_packages(
    conn: &mut PgConnection,
    device: Uuid,
    packages: &[String],
    last_resort: Option<&str>,
) -> ApiResult<()> {
    for kp in packages {
        let bytes = b64("key_packages", kp, 4096)?;
        sqlx::query(
            "insert into chat_key_packages (id, device_id, key_package) values ($1, $2, $3)",
        )
        .bind(Uuid::now_v7())
        .bind(device)
        .bind(bytes)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(lr) = last_resort {
        let bytes = b64("last_resort", lr, 4096)?;
        sqlx::query("delete from chat_key_packages where device_id = $1 and last_resort")
            .bind(device)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "insert into chat_key_packages (id, device_id, key_package, last_resort) values ($1, $2, $3, true)",
        )
        .bind(Uuid::now_v7())
        .bind(device)
        .bind(bytes)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyPackagesBody {
    key_packages: Vec<String>,
    last_resort: Option<String>,
}

async fn add_key_packages(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<KeyPackagesBody>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let (mine, stored): (bool, i64) = sqlx::query_as(
        "select exists (select 1 from chat_devices where id = $1 and user_id = $2 and status <> 'revoked'),
                (select count(*) from chat_key_packages where device_id = $1 and not last_resort)",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if !mine {
        return Err(ApiError::NotFound);
    }
    if stored + b.key_packages.len() as i64 > KEY_PACKAGES * 2 {
        return Err(ApiError::invalid("key_packages", "too many"));
    }
    store_key_packages(&mut tx, id, &b.key_packages, b.last_resort.as_deref()).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    /// The approving device: one of the user's active devices.
    by: Uuid,
    /// Its Ed25519 signature over `approval_message`.
    signature: String,
}

/// What an approving device signs: the user, the new device and its key.
pub fn approval_message(user: Uuid, device: Uuid, signature_key: &[u8]) -> Vec<u8> {
    format!(
        "{APPROVAL_CONTEXT}:{user}:{device}:{}",
        hex::encode(signature_key)
    )
    .into_bytes()
}

async fn approve_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Approval>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let new_key: Option<Vec<u8>> = sqlx::query_scalar(
        "select signature_key from chat_devices where id = $1 and user_id = $2 and status = 'pending' for update",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let new_key = new_key.ok_or(ApiError::NotFound)?;
    let approver_key: Option<Vec<u8>> = sqlx::query_scalar(
        "select signature_key from chat_devices where id = $1 and user_id = $2 and status = 'active'",
    )
    .bind(b.by)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let approver_key = approver_key.ok_or(ApiError::NotFound)?;
    let signature = b64("signature", &b.signature, 64)?;
    if !verify(
        &approver_key,
        &approval_message(user.id, id, &new_key),
        &signature,
    ) {
        return Err(ApiError::invalid(
            "signature",
            "the approving device's signature doesn't check out",
        ));
    }
    sqlx::query(
        "update chat_devices set status = 'active', approved_by = $2, approval_sig = $3, approved_at = now()
         where id = $1",
    )
    .bind(id)
    .bind(b.by)
    .bind(&signature)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    wake_contacts(&state, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn verify(public: &[u8], message: &[u8], signature: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let (Ok(public), Ok(signature)) = (
        <[u8; 32]>::try_from(public),
        <[u8; 64]>::try_from(signature),
    ) else {
        return false;
    };
    VerifyingKey::from_bytes(&public)
        .and_then(|k| k.verify(message, &Signature::from_bytes(&signature)))
        .is_ok()
}

/// Every device lost: this pending device becomes the user's only one. It needs a fresh sign-in
/// (SECURITY §4.3), and the people they chat with see that their security code changed.
async fn start_over(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    crate::auth::session::require_recent_sign_in(&state.db, &user).await?;
    let mut tx = state.db.begin().await?;
    let pending: bool = sqlx::query_scalar(
        "select exists (select 1 from chat_devices where id = $1 and user_id = $2 and status = 'pending')",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if !pending {
        return Err(ApiError::NotFound);
    }
    sqlx::query(
        "update chat_devices set status = 'revoked', revoked_at = now() where user_id = $1 and id <> $2 and status <> 'revoked'",
    )
    .bind(user.id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update chat_devices set status = 'active', approved_by = null, approval_sig = null, approved_at = now()
         where id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("delete from chat_key_packages k using chat_devices d where d.id = k.device_id and d.user_id = $1 and d.status = 'revoked'")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    wake_contacts(&state, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "update chat_devices set status = 'revoked', revoked_at = now() where id = $1 and user_id = $2 and status <> 'revoked'",
    )
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query("delete from chat_key_packages where device_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from chat_welcomes where device_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Their other devices and the people they chat with remove it from every chat.
    wake_contacts(&state, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Everyone who shares a chat with the user (and the user), so their apps re-check devices.
async fn wake_contacts(state: &AppState, user: Uuid) -> sqlx::Result<()> {
    let mut users: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select distinct p2.user_id from ({PARTICIPANTS}) p1 join ({PARTICIPANTS}) p2
           on p2.conversation_id = p1.conversation_id where p1.user_id = $1"
    )))
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    users.push(user);
    state.chat_hub.wake(users);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Conversations

/// Everyone in every chat: direct, group and self chats from `chat_members`, team chats from
/// the team's members.
pub const PARTICIPANTS: &str = "select m.conversation_id, m.user_id from chat_members m
     union all
     select c.id, tm.user_id from chat_conversations c join team_members tm on tm.team_id = c.team_id
     where c.kind = 'team'";

async fn is_member(conn: &mut PgConnection, conversation: Uuid, user: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select exists (select 1 from ({PARTICIPANTS}) p where p.conversation_id = $1 and p.user_id = $2)"
    )))
    .bind(conversation)
    .bind(user)
    .fetch_one(conn)
    .await
}

async fn participants(conn: &mut PgConnection, conversation: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select p.user_id from ({PARTICIPANTS}) p where p.conversation_id = $1"
    )))
    .bind(conversation)
    .fetch_all(conn)
    .await
}

#[derive(Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct Member {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Conversation {
    pub id: Uuid,
    pub kind: String,
    /// Team chats: the team's address and name.
    team_slug: Option<String>,
    team_name: Option<String>,
    #[sqlx(json)]
    pub members: Vec<Member>,
    pub epoch: i64,
    /// The newest message's sequence number, to compare with `read_seq`.
    last_seq: i64,
    read_seq: i64,
    /// Messages from others since `read_seq`.
    unread: i64,
    muted: bool,
    created_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_message_at: Option<OffsetDateTime>,
    /// You can't message the other person any more (direct chats): blocked, or their account closed.
    #[sqlx(skip)]
    pub closed: bool,
}

const CONVERSATION_SQL: &str = "select c.id, c.kind, t.slug as team_slug, t.name as team_name, c.epoch, c.created_by, c.last_message_at,
        coalesce((select json_agg(json_build_object('id', u.id, 'name', u.display_name) order by u.display_name)
                  from ({PARTICIPANTS}) p join users u on u.id = p.user_id where p.conversation_id = c.id), '[]') as members,
        coalesce((select max(seq) from chat_messages m where m.conversation_id = c.id and m.kind = 'application'), 0) as last_seq,
        coalesce(s.read_seq, 0) as read_seq,
        (select count(*) from chat_messages m where m.conversation_id = c.id and m.kind = 'application'
           and m.seq > coalesce(s.read_seq, 0) and m.sender_user_id is distinct from $1) as unread,
        coalesce(s.muted, false) as muted
 from chat_conversations c
 left join teams t on t.id = c.team_id
 left join chat_member_state s on s.conversation_id = c.id and s.user_id = $1";

fn conversation_sql(filter: &str) -> String {
    format!(
        "{} where exists (select 1 from ({PARTICIPANTS}) p where p.conversation_id = c.id and p.user_id = $1) {filter}",
        CONVERSATION_SQL.replace("{PARTICIPANTS}", PARTICIPANTS)
    )
}

async fn with_closed(
    conn: &mut PgConnection,
    me: Uuid,
    mut c: Conversation,
) -> sqlx::Result<Conversation> {
    if c.kind == "direct"
        && let Some(other) = c.members.iter().find(|m| m.id != me)
    {
        c.closed = !friends::may_message(conn, me, other.id).await?;
    }
    Ok(c)
}

async fn conversations(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<Conversation>>> {
    let mut conn = state.db.acquire().await?;
    let list: Vec<Conversation> = sqlx::query_as(sqlx::AssertSqlSafe(conversation_sql(
        "and not exists (select 1 from chat_member_state h where h.conversation_id = c.id and h.user_id = $1
                           and h.hidden_at is not null and (c.last_message_at is null or c.last_message_at < h.hidden_at))
         order by coalesce(c.last_message_at, c.created_at) desc",
    )))
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(list.len());
    for c in list {
        // A blocked person's direct chat leaves the blocker's list.
        if c.kind == "direct"
            && let Some(other) = c.members.iter().find(|m| m.id != user.id)
            && sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from user_blocks where user_id = $1 and blocked_id = $2)",
            )
            .bind(user.id)
            .bind(other.id)
            .fetch_one(&mut *conn)
            .await?
        {
            continue;
        }
        out.push(with_closed(&mut conn, user.id, c).await?);
    }
    Ok(Json(out))
}

pub async fn load_conversation(
    conn: &mut PgConnection,
    user: Uuid,
    id: Uuid,
) -> ApiResult<Conversation> {
    let c: Conversation = sqlx::query_as(sqlx::AssertSqlSafe(conversation_sql("and c.id = $2")))
        .bind(user)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(with_closed(conn, user, c).await?)
}

async fn conversation(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Conversation>> {
    let mut conn = state.db.acquire().await?;
    Ok(Json(load_conversation(&mut conn, user.id, id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    /// direct, group, team or self.
    kind: String,
    /// direct: a profile link or ID.
    user: Option<String>,
    /// group: profile links or IDs.
    #[serde(default)]
    users: Vec<String>,
    /// team: the team's address.
    team: Option<String>,
}

/// Starts a chat, or returns the existing one (direct, team and self chats are one each). The
/// creating device then sets up the encryption with a first commit.
async fn start(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<StartBody>,
) -> ApiResult<(StatusCode, Json<Conversation>)> {
    require_app(&state.db, &user).await?;
    let mut tx = state.db.begin().await?;
    let id = Uuid::now_v7();
    let existing: Option<Uuid> = match b.kind.as_str() {
        "direct" => {
            let other = friends::parse_user(b.user.as_deref().unwrap_or(""))?;
            if !friends::may_message(&mut tx, user.id, other).await? {
                // Same answer whether they don't exist or don't take messages from you.
                return Err(ApiError::NotFound);
            }
            let pair = format!("{}:{}", user.id.min(other), user.id.max(other));
            let found: Option<Uuid> =
                sqlx::query_scalar("select id from chat_conversations where pair_key = $1")
                    .bind(&pair)
                    .fetch_optional(&mut *tx)
                    .await?;
            if found.is_none() {
                sqlx::query(
                    "insert into chat_conversations (id, kind, pair_key, created_by) values ($1, 'direct', $2, $3)",
                )
                .bind(id)
                .bind(&pair)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
                for u in [user.id, other] {
                    add_member(&mut tx, id, u, "member").await?;
                }
            }
            found
        }
        "self" => {
            let pair = user.id.to_string();
            let found: Option<Uuid> =
                sqlx::query_scalar("select id from chat_conversations where pair_key = $1")
                    .bind(&pair)
                    .fetch_optional(&mut *tx)
                    .await?;
            if found.is_none() {
                sqlx::query(
                    "insert into chat_conversations (id, kind, pair_key, created_by) values ($1, 'self', $2, $3)",
                )
                .bind(id)
                .bind(&pair)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
                add_member(&mut tx, id, user.id, "owner").await?;
            }
            found
        }
        "group" => {
            let mut others = Vec::new();
            for u in &b.users {
                let other = friends::parse_user(u)?;
                if other != user.id && !others.contains(&other) {
                    others.push(other);
                }
            }
            if others.is_empty() {
                return Err(ApiError::invalid("users", "add at least one person"));
            }
            if others.len() as i64 >= MAX_GROUP {
                return Err(ApiError::invalid("users", "50 people at most"));
            }
            for other in &others {
                if !friends::may_message(&mut tx, user.id, *other).await? {
                    return Err(ApiError::invalid(
                        "users",
                        "you can only add friends, teammates and People job partners (or people who take messages from anyone)",
                    ));
                }
            }
            sqlx::query(
                "insert into chat_conversations (id, kind, created_by) values ($1, 'group', $2)",
            )
            .bind(id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
            add_member(&mut tx, id, user.id, "owner").await?;
            for other in &others {
                add_member(&mut tx, id, *other, "member").await?;
            }
            None
        }
        "team" => {
            let slug = b.team.as_deref().unwrap_or("").to_ascii_lowercase();
            let team: Option<Uuid> = sqlx::query_scalar(
                "select t.id from teams t join team_members m on m.team_id = t.id
                 where t.slug = $1 and t.kind = 'player' and t.hidden_reason is null and m.user_id = $2",
            )
            .bind(&slug)
            .bind(user.id)
            .fetch_optional(&mut *tx)
            .await?;
            let team = team.ok_or(ApiError::NotFound)?;
            let found: Option<Uuid> =
                sqlx::query_scalar("select id from chat_conversations where team_id = $1")
                    .bind(team)
                    .fetch_optional(&mut *tx)
                    .await?;
            if found.is_none() {
                sqlx::query(
                    "insert into chat_conversations (id, kind, team_id, created_by) values ($1, 'team', $2, $3)",
                )
                .bind(id)
                .bind(team)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
            }
            found
        }
        _ => return Err(ApiError::invalid("kind", "direct, group, team or self")),
    };
    let id = existing.unwrap_or(id);
    sqlx::query(
        "update chat_member_state set hidden_at = null where conversation_id = $1 and user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    let c = load_conversation(&mut tx, user.id, id).await?;
    let everyone = participants(&mut tx, id).await?;
    tx.commit().await?;
    state.chat_hub.wake(everyone);
    Ok((
        if existing.is_some() {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(c),
    ))
}

async fn add_member(
    conn: &mut PgConnection,
    conversation: Uuid,
    user: Uuid,
    role: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into chat_members (conversation_id, user_id, role) values ($1, $2, $3) on conflict do nothing",
    )
    .bind(conversation)
    .bind(user)
    .bind(role)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddBody {
    users: Vec<String>,
}

/// Adds people to a group chat. Any member may add people they may message; their apps then add
/// the new people's devices to the encryption.
async fn add_members(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<AddBody>,
) -> ApiResult<Json<Conversation>> {
    let mut tx = state.db.begin().await?;
    let kind: Option<String> =
        sqlx::query_scalar("select kind from chat_conversations where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if kind.is_none() || !is_member(&mut tx, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    if kind.as_deref() != Some("group") {
        return Err(ApiError::Conflict(
            "people join group chats this way; team chats follow the team".into(),
        ));
    }
    let count: i64 =
        sqlx::query_scalar("select count(*) from chat_members where conversation_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if count + b.users.len() as i64 > MAX_GROUP {
        return Err(ApiError::invalid("users", "50 people at most"));
    }
    for u in &b.users {
        let other = friends::parse_user(u)?;
        if !friends::may_message(&mut tx, user.id, other).await? {
            return Err(ApiError::invalid(
                "users",
                "you can only add friends, teammates and People job partners (or people who take messages from anyone)",
            ));
        }
        add_member(&mut tx, id, other, "member").await?;
    }
    let c = load_conversation(&mut tx, user.id, id).await?;
    let everyone = participants(&mut tx, id).await?;
    tx.commit().await?;
    state.chat_hub.wake(everyone);
    Ok(Json(c))
}

/// Leaving a group chat (yourself), or its owner removing someone. Their devices are removed from
/// the encryption by whichever member's app is online next.
async fn remove_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, other)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let kind: Option<String> =
        sqlx::query_scalar("select kind from chat_conversations where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if kind.is_none() || !is_member(&mut tx, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    if kind.as_deref() != Some("group") {
        return Err(ApiError::Conflict(
            "only group chats can be left; hide a direct or team chat instead".into(),
        ));
    }
    if other != user.id {
        let owner: bool = sqlx::query_scalar(
            "select exists (select 1 from chat_members where conversation_id = $1 and user_id = $2 and role = 'owner')",
        )
        .bind(id)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
        if !owner {
            return Err(ApiError::Conflict(
                "only the person who started the group can remove people".into(),
            ));
        }
    }
    let everyone = participants(&mut tx, id).await?;
    sqlx::query("delete from chat_members where conversation_id = $1 and user_id = $2")
        .bind(id)
        .bind(other)
        .execute(&mut *tx)
        .await?;
    // The owner leaving hands the group to the longest-standing member.
    sqlx::query(
        "update chat_members set role = 'owner' where conversation_id = $1 and user_id = (
             select user_id from chat_members where conversation_id = $1 order by joined_at limit 1)
           and not exists (select 1 from chat_members where conversation_id = $1 and role = 'owner')",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let left: i64 =
        sqlx::query_scalar("select count(*) from chat_members where conversation_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if left == 0 {
        sqlx::query("delete from chat_conversations where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    state.chat_hub.wake(everyone);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadBody {
    seq: i64,
}

async fn state_row(conn: &mut PgConnection, conversation: Uuid, user: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        "insert into chat_member_state (conversation_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(conversation)
    .bind(user)
    .execute(conn)
    .await?;
    Ok(())
}

async fn mark_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<ReadBody>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    if !is_member(&mut tx, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    state_row(&mut tx, id, user.id).await?;
    sqlx::query(
        "update chat_member_state set read_seq = greatest(read_seq, $3) where conversation_id = $1 and user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(b.seq)
    .execute(&mut *tx)
    .await?;
    // The bell's "new messages" notice for this chat goes too.
    sqlx::query(
        "update notifications set read_at = now() where user_id = $1 and collapse_key = $2 and read_at is null",
    )
    .bind(user.id)
    .bind(format!("chat:{id}"))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // The user's other devices update their unread counts.
    state.chat_hub.wake(vec![user.id]);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MuteBody {
    muted: bool,
}

async fn mute(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<MuteBody>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    if !is_member(&mut tx, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    state_row(&mut tx, id, user.id).await?;
    sqlx::query(
        "update chat_member_state set muted = $3 where conversation_id = $1 and user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(b.muted)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypingBody {
    #[serde(default = "yes")]
    typing: bool,
}

fn yes() -> bool {
    true
}

/// "Typing…": shown to the others in the chat for a few seconds. Nothing is stored.
async fn typing(
    State(state): State<AppState>,
    device: ChatDevice,
    Path(id): Path<Uuid>,
    body: Option<Json<TypingBody>>,
) -> ApiResult<StatusCode> {
    let mut conn = state.db.acquire().await?;
    let everyone = participants(&mut conn, id).await?;
    if !everyone.contains(&device.user.id) {
        return Err(ApiError::NotFound);
    }
    let on = body.map(|b| b.typing).unwrap_or(true);
    state.chat_hub.set_typing(id, device.user.id, on);
    state.chat_hub.wake(everyone);
    Ok(StatusCode::NO_CONTENT)
}

/// Hides a chat from the list until a new message arrives.
async fn hide(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    if !is_member(&mut tx, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    state_row(&mut tx, id, user.id).await?;
    sqlx::query("update chat_member_state set hidden_at = now() where conversation_id = $1 and user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Devices in a chat, and their key packages

#[derive(Serialize, sqlx::FromRow)]
pub struct MemberDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    /// active or revoked: revoked ones are listed so approval chains can still be checked.
    pub status: String,
    #[serde(with = "b64_bytes")]
    pub signature_key: Vec<u8>,
    pub approved_by: Option<Uuid>,
    #[serde(with = "b64_option")]
    pub approval_sig: Option<Vec<u8>>,
}

mod b64_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    pub fn serialize<S: serde::Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(v))
    }
}

mod b64_option {
    use base64::{Engine, engine::general_purpose::STANDARD};
    pub fn serialize<S: serde::Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_str(&STANDARD.encode(v)),
            None => s.serialize_none(),
        }
    }
}

/// The devices of everyone in a chat: the active ones are what its encryption should include.
/// Revoked ones are listed too, so each app can follow every approval back to the user's first
/// device and check the signatures itself.
async fn member_devices(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<MemberDevice>>> {
    let mut conn = state.db.acquire().await?;
    if !is_member(&mut conn, id, user.id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select d.id, d.user_id, d.name, d.status, d.signature_key, d.approved_by, d.approval_sig from chat_devices d
             where d.status <> 'pending' and d.user_id in (select p.user_id from ({PARTICIPANTS}) p where p.conversation_id = $1)
             order by d.user_id, d.created_at"
        )))
        .bind(id)
        .fetch_all(&mut *conn)
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimBody {
    devices: Vec<Uuid>,
}

#[derive(Serialize)]
struct Claimed {
    device_id: Uuid,
    key_package: String,
}

/// Takes one key package for each device being added to a chat: only active devices of the
/// chat's members. The last-resort package is never used up.
async fn claim_key_packages(
    State(state): State<AppState>,
    device: ChatDevice,
    Path(id): Path<Uuid>,
    Json(b): Json<ClaimBody>,
) -> ApiResult<Json<Vec<Claimed>>> {
    if b.devices.len() > (MAX_GROUP * 4) as usize {
        return Err(ApiError::invalid("devices", "too many"));
    }
    let mut tx = state.db.begin().await?;
    if !is_member(&mut tx, id, device.user.id).await? {
        return Err(ApiError::NotFound);
    }
    let mut out = Vec::with_capacity(b.devices.len());
    for d in &b.devices {
        let allowed: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select exists (select 1 from chat_devices d where d.id = $1 and d.status = 'active'
                    and d.user_id in (select p.user_id from ({PARTICIPANTS}) p where p.conversation_id = $2))"
        )))
        .bind(d)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !allowed {
            continue;
        }
        let row: Option<(Uuid, Vec<u8>, bool)> = sqlx::query_as(
            "select id, key_package, last_resort from chat_key_packages where device_id = $1
             order by last_resort, created_at limit 1 for update skip locked",
        )
        .bind(d)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((kp_id, kp, last_resort)) = row else {
            continue;
        };
        if !last_resort {
            sqlx::query("delete from chat_key_packages where id = $1")
                .bind(kp_id)
                .execute(&mut *tx)
                .await?;
        }
        out.push(Claimed {
            device_id: *d,
            key_package: B64.encode(kp),
        });
    }
    tx.commit().await?;
    Ok(Json(out))
}

// ---------------------------------------------------------------------------------------------
// Commits and messages

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitBody {
    /// The epoch the commit builds on.
    epoch: i64,
    commit: String,
    #[serde(default)]
    welcomes: Vec<WelcomeOut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeOut {
    device_id: Uuid,
    welcome: String,
}

#[derive(Serialize)]
struct Accepted {
    seq: i64,
    epoch: i64,
}

fn stale(current: i64) -> impl IntoResponse {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({ "error": {
            "code": "stale_epoch",
            "message": "the chat moved on; fetch what's new and try again",
            "epoch": current,
        } })),
    )
}

/// A commit: adds or removes devices (or just updates keys). It must build on the epoch Slop
/// last saw, so concurrent commits can't fork the chat; the loser fetches and tries again.
async fn commit(
    State(state): State<AppState>,
    device: ChatDevice,
    Path(id): Path<Uuid>,
    Json(b): Json<CommitBody>,
) -> ApiResult<axum::response::Response> {
    let body = b64("commit", &b.commit, MAX_COMMIT_BODY)?;
    let mut tx = state.db.begin().await?;
    let epoch: Option<i64> =
        sqlx::query_scalar("select epoch from chat_conversations where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let epoch = epoch.ok_or(ApiError::NotFound)?;
    if !is_member(&mut tx, id, device.user.id).await? {
        return Err(ApiError::NotFound);
    }
    if epoch != b.epoch {
        return Ok(stale(epoch).into_response());
    }
    let seq: i64 = sqlx::query_scalar(
        "insert into chat_messages (conversation_id, sender_device_id, sender_user_id, kind, epoch, body)
         values ($1, $2, $3, 'commit', $4, $5) returning seq",
    )
    .bind(id)
    .bind(device.id)
    .bind(device.user.id)
    .bind(epoch)
    .bind(&body)
    .fetch_one(&mut *tx)
    .await?;
    for w in &b.welcomes {
        let welcome = b64("welcomes", &w.welcome, 256 * 1024)?;
        let allowed: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select exists (select 1 from chat_devices d where d.id = $1 and d.status = 'active'
                    and d.user_id in (select p.user_id from ({PARTICIPANTS}) p where p.conversation_id = $2))"
        )))
        .bind(w.device_id)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !allowed {
            return Err(ApiError::invalid(
                "welcomes",
                "a Welcome for a device that isn't in this chat",
            ));
        }
        sqlx::query(
            "insert into chat_welcomes (id, device_id, conversation_id, welcome, after_seq) values ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(w.device_id)
        .bind(id)
        .bind(welcome)
        .bind(seq)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("update chat_conversations set epoch = epoch + 1 where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let everyone = participants(&mut tx, id).await?;
    tx.commit().await?;
    state.chat_hub.wake(everyone);
    Ok(Json(Accepted {
        seq,
        epoch: epoch + 1,
    })
    .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageBody {
    epoch: i64,
    body: String,
    /// The sender's franking commitment, HMAC-SHA256(franking key, plaintext).
    franking_tag: String,
}

/// Messages sent per minute by one account before chat slows them down.
const MESSAGES_PER_MINUTE: i64 = 40;

async fn send(
    State(state): State<AppState>,
    device: ChatDevice,
    Path(id): Path<Uuid>,
    Json(b): Json<MessageBody>,
) -> ApiResult<axum::response::Response> {
    let body = b64("body", &b.body, MAX_MESSAGE_BYTES)?;
    let tag = b64("franking_tag", &b.franking_tag, 32)?;
    if tag.len() != 32 {
        return Err(ApiError::invalid("franking_tag", "32 bytes"));
    }
    let mut tx = state.db.begin().await?;
    let row: Option<(i64, String)> =
        sqlx::query_as("select epoch, kind from chat_conversations where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (epoch, kind) = row.ok_or(ApiError::NotFound)?;
    let everyone = participants(&mut tx, id).await?;
    if !everyone.contains(&device.user.id) {
        return Err(ApiError::NotFound);
    }
    // A direct chat stops when either blocks the other or can't message them any more.
    if kind == "direct"
        && let Some(other) = everyone.iter().find(|u| **u != device.user.id)
        && !friends::may_message(&mut tx, device.user.id, *other).await?
    {
        return Err(ApiError::Conflict(
            "you can't message this person any more".into(),
        ));
    }
    if epoch != b.epoch {
        return Ok(stale(epoch).into_response());
    }
    let recent: i64 = sqlx::query_scalar(
        "select count(*) from chat_messages where sender_user_id = $1 and kind = 'application'
         and created_at > now() - interval '1 minute'",
    )
    .bind(device.user.id)
    .fetch_one(&mut *tx)
    .await?;
    if recent >= MESSAGES_PER_MINUTE {
        return Err(ApiError::Conflict(
            "you're sending a lot of messages; wait a moment".into(),
        ));
    }
    let seq: i64 = sqlx::query_scalar(
        "insert into chat_messages (conversation_id, sender_device_id, sender_user_id, kind, epoch, body, franking_tag)
         values ($1, $2, $3, 'application', $4, $5, $6) returning seq",
    )
    .bind(id)
    .bind(device.id)
    .bind(device.user.id)
    .bind(epoch)
    .bind(&body)
    .bind(&tag)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("update chat_conversations set last_message_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // The sender has read their own message.
    state_row(&mut tx, id, device.user.id).await?;
    sqlx::query(
        "update chat_member_state set read_seq = $3, hidden_at = null where conversation_id = $1 and user_id = $2",
    )
    .bind(id)
    .bind(device.user.id)
    .bind(seq)
    .execute(&mut *tx)
    .await?;
    tell_away(&mut tx, id, &kind, &device.user, &everyone).await?;
    tx.commit().await?;
    state.chat_hub.set_typing(id, device.user.id, false);
    state.chat_hub.wake(everyone);
    Ok(Json(Accepted { seq, epoch }).into_response())
}

/// A bell notice (and, if they want, an email) for members whose apps haven't been online for a
/// few minutes, without what was said: Slop can't read it. One per chat until they read it.
async fn tell_away(
    conn: &mut PgConnection,
    conversation: Uuid,
    kind: &str,
    sender: &CurrentUser,
    everyone: &[Uuid],
) -> sqlx::Result<()> {
    for user in everyone {
        if *user == sender.id {
            continue;
        }
        let (away, muted): (bool, bool) = sqlx::query_as(
            "select not exists (select 1 from chat_devices where user_id = $1 and status = 'active'
                                and last_seen_at > now() - interval '3 minutes'),
                    coalesce((select muted from chat_member_state where conversation_id = $2 and user_id = $1), false)",
        )
        .bind(user)
        .bind(conversation)
        .fetch_one(&mut *conn)
        .await?;
        if !away || muted {
            continue;
        }
        let title = match kind {
            "direct" => format!("New message from {}", sender.display_name),
            "team" => format!(
                "New messages in your team chat from {}",
                sender.display_name
            ),
            _ => format!("New messages in a group chat from {}", sender.display_name),
        };
        notify::send(
            conn,
            Notice {
                user_id: *user,
                category: "chat",
                title,
                body: "Messages are end-to-end encrypted, so open the Slop app to read them."
                    .into(),
                link: format!("/chat/{conversation}"),
                collapse_key: Some(format!("chat:{conversation}")),
            },
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Sync

#[derive(Deserialize)]
struct SyncQuery {
    /// The last sequence number this device has.
    #[serde(default)]
    after: i64,
    /// Seconds to wait for something new (at most 25).
    #[serde(default)]
    wait: u64,
}

#[derive(Serialize, sqlx::FromRow)]
struct Event {
    seq: i64,
    conversation_id: Uuid,
    kind: String,
    sender_device_id: Option<Uuid>,
    sender_user_id: Option<Uuid>,
    epoch: i64,
    #[serde(with = "b64_bytes")]
    body: Vec<u8>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct Welcome {
    id: Uuid,
    conversation_id: Uuid,
    #[serde(with = "b64_bytes")]
    welcome: Vec<u8>,
    after_seq: i64,
}

#[derive(Serialize)]
struct Synced {
    events: Vec<Event>,
    welcomes: Vec<Welcome>,
    /// The highest sequence number so far, to pass back as `after`.
    next: i64,
    /// Changes when chats, their members or their devices change: the app re-checks who should
    /// be in each chat's encryption.
    roster: String,
    /// Key packages this device has left (it uploads more below half).
    key_packages: i64,
    /// Devices of yours waiting for approval.
    pending_devices: i64,
    /// Who is typing in your chats right now.
    typing: Vec<Typing>,
}

/// What's new for this device: messages and commits in its chats, and Welcomes to new ones.
/// With `wait`, holds the request until something arrives (long-polling).
async fn sync(
    State(state): State<AppState>,
    device: ChatDevice,
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<Synced>> {
    let wait = StdDuration::from_secs(q.wait.min(LONG_POLL.as_secs()));
    let mut rx = state.chat_hub.0.subscribe();
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let synced = sync_once(&state, &device, q.after).await?;
        if !synced.events.is_empty() || !synced.welcomes.is_empty() || wait.is_zero() {
            return Ok(Json(synced));
        }
        // Wait for a change for this user, then look again (and answer with the roster anyway
        // when the time is up, so the app notices member and device changes).
        let woke = loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(users)) if users.contains(&device.user.id) => break true,
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => break true,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break false,
            }
        };
        if !woke {
            return Ok(Json(sync_once(&state, &device, q.after).await?));
        }
        let again = sync_once(&state, &device, q.after).await?;
        if !again.events.is_empty()
            || !again.welcomes.is_empty()
            || again.roster != synced.roster
            || again.typing != synced.typing
        {
            return Ok(Json(again));
        }
    }
}

async fn sync_once(state: &AppState, device: &ChatDevice, after: i64) -> ApiResult<Synced> {
    let db = &state.db;
    let mine: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select p.conversation_id from ({PARTICIPANTS}) p where p.user_id = $1"
    )))
    .bind(device.user.id)
    .fetch_all(db)
    .await?;
    let typing = state.chat_hub.typing(&mine, device.user.id);
    let events: Vec<Event> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select m.seq, m.conversation_id, m.kind, m.sender_device_id, m.sender_user_id, m.epoch, m.body, m.created_at
         from chat_messages m
         where m.seq > $2 and m.conversation_id in (select p.conversation_id from ({PARTICIPANTS}) p where p.user_id = $1)
         order by m.seq limit 500"
    )))
    .bind(device.user.id)
    .bind(after)
    .fetch_all(db)
    .await?;
    let welcomes: Vec<Welcome> = sqlx::query_as(
        "select id, conversation_id, welcome, after_seq from chat_welcomes where device_id = $1 order by after_seq",
    )
    .bind(device.id)
    .fetch_all(db)
    .await?;
    let next = events.last().map(|e| e.seq).unwrap_or(after);
    let roster: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select md5(coalesce(string_agg(x, ',' order by x), ''))
         from (select p.conversation_id::text || ':' || p.user_id::text || ':' ||
                      coalesce((select string_agg(d.id::text, '|' order by d.id) from chat_devices d
                                where d.user_id = p.user_id and d.status = 'active'), '') as x
               from ({PARTICIPANTS}) p
               where p.conversation_id in (select q.conversation_id from ({PARTICIPANTS}) q where q.user_id = $1)) r"
    )))
    .bind(device.user.id)
    .fetch_one(db)
    .await?;
    let (key_packages, pending_devices): (i64, i64) = sqlx::query_as(
        "select (select count(*) from chat_key_packages where device_id = $1 and not last_resort),
                (select count(*) from chat_devices where user_id = $2 and status = 'pending')",
    )
    .bind(device.id)
    .bind(device.user.id)
    .fetch_one(db)
    .await?;
    // Welcomes are fetched once: the app keeps what it needs.
    if !welcomes.is_empty() {
        let ids: Vec<Uuid> = welcomes.iter().map(|w| w.id).collect();
        sqlx::query("delete from chat_welcomes where id = any($1)")
            .bind(&ids)
            .execute(db)
            .await?;
    }
    Ok(Synced {
        events,
        welcomes,
        next,
        roster,
        key_packages,
        pending_devices,
        typing,
    })
}

// ---------------------------------------------------------------------------------------------
// Attachments and backups

#[derive(Serialize)]
struct Uploaded {
    id: Uuid,
}

async fn upload(
    State(state): State<AppState>,
    device: ChatDevice,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Uploaded>)> {
    if body.is_empty() || body.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::invalid("body", "1 byte to 16 MB"));
    }
    let mut tx = state.db.begin().await?;
    if !is_member(&mut tx, id, device.user.id).await? {
        return Err(ApiError::NotFound);
    }
    let today: i64 = sqlx::query_scalar(
        "select coalesce(sum(size_bytes), 0)::int8 from chat_attachments where uploader_id = $1 and created_at > now() - interval '1 day'",
    )
    .bind(device.user.id)
    .fetch_one(&mut *tx)
    .await?;
    if today + body.len() as i64 > 500 * 1024 * 1024 {
        return Err(ApiError::Conflict(
            "that's a lot of attachments for one day; try again tomorrow".into(),
        ));
    }
    let attachment = Uuid::now_v7();
    sqlx::query(
        "insert into chat_attachments (id, conversation_id, uploader_id, size_bytes, body) values ($1, $2, $3, $4, $5)",
    )
    .bind(attachment)
    .bind(id)
    .bind(device.user.id)
    .bind(body.len() as i32)
    .bind(body.as_ref())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Uploaded { id: attachment })))
}

async fn attachment(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Response> {
    let row: Option<(Uuid, Vec<u8>)> =
        sqlx::query_as("select conversation_id, body from chat_attachments where id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (conversation, body) = row.ok_or(ApiError::NotFound)?;
    let mut conn = state.db.acquire().await?;
    if !is_member(&mut conn, conversation, user.id).await? {
        return Err(ApiError::NotFound);
    }
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response())
}

async fn save_backup(
    State(state): State<AppState>,
    device: ChatDevice,
    body: Bytes,
) -> ApiResult<StatusCode> {
    if body.is_empty() || body.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::invalid("body", "1 byte to 16 MB"));
    }
    sqlx::query(
        "insert into chat_backups (user_id, body) values ($1, $2)
         on conflict (user_id) do update set body = excluded.body, updated_at = now()",
    )
    .bind(device.user.id)
    .bind(body.as_ref())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn backup(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<axum::response::Response> {
    let body: Option<Vec<u8>> =
        sqlx::query_scalar("select body from chat_backups where user_id = $1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    let body = body.ok_or(ApiError::NotFound)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response())
}

async fn delete_backup(State(state): State<AppState>, user: CurrentUser) -> ApiResult<StatusCode> {
    sqlx::query("delete from chat_backups where user_id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Reports

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatEvidence {
    pub conversation_id: Uuid,
    pub seq: i64,
    /// The message exactly as decrypted.
    pub plaintext: String,
    /// The franking key inside it, base64.
    pub franking_key: String,
}

/// Checks a reported message against its franking tag (SECURITY §8.5): proof that its sender
/// really sent exactly this, without Slop reading any other message. Returns the sender and the
/// chat's kind.
pub async fn check_evidence(
    db: &PgPool,
    reporter: Uuid,
    e: &ChatEvidence,
) -> ApiResult<(Uuid, String, String)> {
    let mut conn = db.acquire().await?;
    if !is_member(&mut conn, e.conversation_id, reporter).await? {
        return Err(ApiError::NotFound);
    }
    type Row = (Option<Uuid>, Option<Vec<u8>>, String, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "select m.sender_user_id, m.franking_tag, c.kind, u.display_name from chat_messages m
         join chat_conversations c on c.id = m.conversation_id left join users u on u.id = m.sender_user_id
         where m.seq = $1 and m.conversation_id = $2 and m.kind = 'application'",
    )
    .bind(e.seq)
    .bind(e.conversation_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (Some(sender), Some(tag), kind, name) = row.ok_or_else(|| {
        ApiError::Conflict(
            "that message is older than 30 days, so Slop can't check it any more".into(),
        )
    })?
    else {
        return Err(ApiError::NotFound);
    };
    if sender == reporter {
        return Err(ApiError::invalid("subject", "that's your own message"));
    }
    let key = b64("franking_key", &e.franking_key, 64)?;
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
        .map_err(|_| ApiError::invalid("franking_key", "not a key"))?;
    mac.update(e.plaintext.as_bytes());
    if mac.verify_slice(&tag).is_err() {
        return Err(ApiError::invalid(
            "plaintext",
            "this doesn't match the message that was sent",
        ));
    }
    Ok((sender, kind, name.unwrap_or_else(|| "someone".into())))
}

// ---------------------------------------------------------------------------------------------
// Cleanup

/// Deletes messages, Welcomes and attachments older than 30 days, and key packages of revoked
/// devices (SECURITY §8.5: keep the minimum, delete it quickly).
pub async fn cleanup(db: &PgPool) -> sqlx::Result<u64> {
    let mut n = 0;
    for sql in [
        "delete from chat_messages where created_at < now() - make_interval(days => $1)",
        "delete from chat_welcomes where created_at < now() - make_interval(days => $1)",
        "delete from chat_attachments where created_at < now() - make_interval(days => $1)",
    ] {
        n += sqlx::query(sql)
            .bind(KEEP_DAYS as i32)
            .execute(db)
            .await?
            .rows_affected();
    }
    n += sqlx::query(
        "delete from chat_key_packages k using chat_devices d where d.id = k.device_id and d.status = 'revoked'",
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(n)
}

/// Pending approvals expire after a day.
pub async fn expire_pending(db: &PgPool) -> sqlx::Result<u64> {
    Ok(sqlx::query(
        "update chat_devices set status = 'revoked', revoked_at = now()
         where status = 'pending' and created_at < now() - interval '1 day'",
    )
    .execute(db)
    .await?
    .rows_affected())
}
