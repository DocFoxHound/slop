// SPDX-License-Identifier: AGPL-3.0-or-later
//! Chat on this device (DESIGN §19, SECURITY §8.5): end-to-end encrypted with MLS, so Slop's
//! server (`slop-api`'s `chat.rs`) only relays what this module encrypts.
//!
//! - **This device** has its own signing key, kept with its MLS state in a local file encrypted
//!   with a key from the OS secure store. The user's first device stands alone; later ones are
//!   approved here on an existing device after comparing a short code.
//! - **Each chat is an MLS group** of the devices of everyone in it. Apps keep the group in step
//!   with the chat's members and their devices (`reconcile`): any member's app can add or remove
//!   devices, but only devices whose approvals check out back to the user's first device.
//! - **Messages** are JSON (`content`), with a franking key so a reported message can be checked.
//!   Pictures, GIFs, videos and files are encrypted attachments; links get previews made here.
//! - **History** stays on the device. A newly approved device gets a copy from the approving one
//!   (through the user's own "self" chat), and an encrypted backup (on by default) restores it
//!   with the recovery code when every device is lost.

pub mod content;
pub mod links;
pub mod mls;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::Mutex as StdMutex,
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use self::content::{AttachmentRef, Body, Content, LinkPreview};
use crate::{
    Error, Result,
    api::{ApiClient, RawBody, RawResponse},
};

const DEVICE_HEADER: &str = "x-slop-chat-device";
/// Messages kept per chat on the device.
const KEEP_MESSAGES: usize = 5000;
/// Key packages kept uploaded.
const KEY_PACKAGES: usize = 20;
/// How long a member who isn't the chat's first device waits before fixing its devices itself.
const FIX_AFTER_SECS: i64 = 45;
/// How often the backup is refreshed while something changed.
const BACKUP_EVERY_SECS: i64 = 10 * 60;
/// What an approving device signs (must match the server's `chat::APPROVAL_CONTEXT`).
const APPROVAL_CONTEXT: &str = "slop-chat-device-approval:v1";

/// Where secrets live: the OS secure store in the app, memory in tests.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Option<Vec<u8>>;
    /// False when the store isn't available.
    fn set(&self, name: &str, value: &[u8]) -> bool;
}

#[derive(Default)]
pub struct MemorySecrets(StdMutex<HashMap<String, Vec<u8>>>);

impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(name).cloned()
    }
    fn set(&self, name: &str, value: &[u8]) -> bool {
        self.0.lock().unwrap().insert(name.into(), value.into());
        true
    }
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The code a device shows and its approver compares (same as the server's `device_code`).
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

fn approval_message(user: Uuid, device: Uuid, key: &[u8]) -> Vec<u8> {
    format!("{APPROVAL_CONTEXT}:{user}:{device}:{}", hex::encode(key)).into_bytes()
}

// ---------------------------------------------------------------------------------------------
// What's kept on the device

#[derive(Serialize, Deserialize, Clone)]
struct DeviceState {
    id: Uuid,
    name: String,
    public_key: String,
    status: String,
}

#[derive(Serialize, Deserialize, Default)]
struct State {
    user: Uuid,
    device: Option<DeviceState>,
    /// OpenMLS's storage: keys and every group's state, base64 pairs.
    mls: Vec<(String, String)>,
    cursor: i64,
    roster: String,
    conversations: BTreeMap<Uuid, LocalConversation>,
    /// Each person's first-device key as first seen (trust on first use); a change is shown.
    roots: HashMap<Uuid, String>,
    backup_key: Option<String>,
    backups: bool,
    backup_dirty: bool,
    last_backup_at: i64,
    /// Chats whose devices are out of step, since when.
    out_of_sync: HashMap<Uuid, i64>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct LocalConversation {
    /// A group chat's name (inside the encryption).
    name: Option<String>,
    /// This device reads messages after this sequence number (when it joined).
    joined_after: i64,
    /// Removed from the chat's encryption (left it, or this device was removed).
    left: bool,
    messages: Vec<Message>,
    /// Unreadable messages, counted rather than shown one by one.
    #[serde(default)]
    undecryptable: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Message {
    id: Uuid,
    seq: i64,
    sender: Uuid,
    at: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    reply_to: Option<Uuid>,
    #[serde(default)]
    attachments: Vec<AttachmentRef>,
    #[serde(default)]
    preview: Option<LinkPreview>,
    /// A line from Slop itself (someone's security code changed, a rename).
    #[serde(default)]
    notice: bool,
    #[serde(default)]
    reactions: BTreeMap<String, BTreeSet<Uuid>>,
    #[serde(default)]
    edited: bool,
    #[serde(default)]
    deleted: bool,
    /// Someone else's message as decrypted, for reporting it (franking).
    #[serde(default)]
    plaintext: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// What the server says

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    /// Their profile picture, served by Slop; None: the letter.
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct ServerConversation {
    id: Uuid,
    kind: String,
    team_slug: Option<String>,
    team_name: Option<String>,
    members: Vec<Person>,
    epoch: i64,
    unread: i64,
    muted: bool,
    created_by: Option<Uuid>,
    last_message_at: Option<String>,
    #[serde(default)]
    closed: bool,
}

#[derive(Deserialize, Clone, Debug)]
struct ServerDevice {
    id: Uuid,
    user_id: Uuid,
    status: String,
    signature_key: String,
    approved_by: Option<Uuid>,
    approval_sig: Option<String>,
}

#[derive(Deserialize)]
struct Synced {
    events: Vec<Event>,
    welcomes: Vec<WelcomeIn>,
    next: i64,
    roster: String,
    key_packages: i64,
    pending_devices: i64,
    #[serde(default)]
    typing: Vec<TypingIn>,
}

#[derive(Deserialize)]
struct TypingIn {
    conversation_id: Uuid,
    user_id: Uuid,
}

#[derive(Deserialize)]
struct Event {
    seq: i64,
    conversation_id: Uuid,
    sender_device_id: Option<Uuid>,
    sender_user_id: Option<Uuid>,
    body: String,
    created_at: String,
}

#[derive(Deserialize)]
struct WelcomeIn {
    conversation_id: Uuid,
    welcome: String,
    after_seq: i64,
}

// ---------------------------------------------------------------------------------------------
// What the UI sees

#[derive(Serialize, Debug, Clone)]
pub struct Status {
    /// none (not set up on this device), pending (waiting for approval), active, or revoked.
    pub state: String,
    pub device_id: Option<Uuid>,
    /// This device's code, to compare when approving it.
    pub code: Option<String>,
    /// The user's devices waiting for approval (shown on an active device).
    pub pending: Vec<PendingDevice>,
    pub devices: Vec<DeviceInfo>,
    pub backups: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PendingDevice {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceInfo {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub last_seen_at: String,
    #[serde(default)]
    pub this_device: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct ConversationView {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub members: Vec<Person>,
    pub team_slug: Option<String>,
    pub unread: i64,
    pub muted: bool,
    /// The encryption is set up on this device (else it waits to be added).
    pub ready: bool,
    /// You can't message the other person any more.
    pub closed: bool,
    pub last: Option<String>,
    pub last_at: Option<String>,
    pub owner: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct MessageView {
    pub id: Uuid,
    pub seq: i64,
    pub sender: Person,
    pub mine: bool,
    pub at: String,
    pub text: String,
    pub reply_to: Option<ReplyView>,
    pub attachments: Vec<AttachmentView>,
    pub preview: Option<PreviewView>,
    pub reactions: Vec<ReactionView>,
    pub edited: bool,
    pub deleted: bool,
    pub notice: bool,
    /// Warnings shown with someone else's message (off-platform payment requests).
    pub warnings: Vec<String>,
    pub reportable: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct ReplyView {
    pub id: Uuid,
    pub sender: String,
    pub text: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct AttachmentView {
    pub id: Uuid,
    pub mime: String,
    pub name: String,
    pub size: u64,
    pub source_url: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct PreviewView {
    pub url: String,
    pub site: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<AttachmentView>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ReactionView {
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
    pub names: Vec<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct SecurityCode {
    pub user: Uuid,
    pub name: String,
    pub code: String,
}

/// What changed in a sync, for the UI to reload.
#[derive(Serialize, Debug, Clone, Default)]
pub struct Changes {
    pub conversations: Vec<Uuid>,
    pub list: bool,
    pub devices: bool,
    /// Who is typing in which chat right now (shown for a few seconds).
    pub typing: Vec<TypingView>,
    /// New messages from other people, for notifications. Muted chats are left out.
    pub new_messages: Vec<NewMessage>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TypingView {
    pub conversation: Uuid,
    pub user: Uuid,
    pub name: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct NewMessage {
    pub conversation: Uuid,
    /// The chat's name ("Mia" for a direct chat, the group's or team's name).
    pub title: String,
    pub sender: String,
    /// A short line to show: the text, or "Sent a picture".
    pub text: String,
}

/// A file to send.
pub struct OutgoingFile {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

fn attachment_view(a: &AttachmentRef) -> AttachmentView {
    AttachmentView {
        id: a.id,
        mime: a.mime.clone(),
        name: a.name.clone(),
        size: a.size,
        source_url: a.source_url.clone(),
    }
}

/// Words that often come with asking to be paid outside Slop, where Slop can't help.
const PAYMENT_WORDS: &[&str] = &[
    "paypal",
    "venmo",
    "cash app",
    "cashapp",
    "zelle",
    "gift card",
    "giftcard",
    "bitcoin",
    "crypto wallet",
    "usdt",
    "wire transfer",
    "western union",
    "moneygram",
    "iban",
    "bank details",
    "send money",
    "pay me",
];

pub fn payment_warning(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    PAYMENT_WORDS.iter().any(|w| lower.contains(w)).then(|| {
        "Careful with payments outside Slop: Slop can't help if something goes wrong. Never pay for something you haven't received, and never send gift cards."
            .to_string()
    })
}

// ---------------------------------------------------------------------------------------------
// The engine

struct Inner {
    state: State,
    provider: mls::Provider,
    /// The server's chat list, refreshed on each sync.
    server: HashMap<Uuid, ServerConversation>,
}

pub struct Chat {
    api: ApiClient,
    path: PathBuf,
    key: [u8; 32],
    inner: Mutex<Inner>,
}

fn b64d(s: &str) -> Result<Vec<u8>> {
    B64.decode(s)
        .map_err(|_| Error::Verify("bad base64 from the server".into()))
}

fn decode<T: serde::de::DeserializeOwned>(what: &str, v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::Verify(format!("{what}: {e}")))
}

impl Chat {
    /// Opens this user's chat on this device: the local state, encrypted with a key from the
    /// secure store (made the first time).
    pub fn open(
        api: ApiClient,
        dir: PathBuf,
        secrets: &dyn SecretStore,
        user: Uuid,
    ) -> Result<Self> {
        let key_name = format!("chat-store:{user}");
        let key: [u8; 32] = match secrets.get(&key_name).and_then(|k| k.try_into().ok()) {
            Some(k) => k,
            None => {
                let k = content::random::<32>();
                if !secrets.set(&key_name, &k) {
                    return Err(Error::Install(
                        "chat needs your system's secure storage (Keychain, Credential Manager or a Secret Service such as GNOME Keyring), and it isn't available".into(),
                    ));
                }
                k
            }
        };
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("chat-{user}.bin"));
        let state: State = match std::fs::read(&path) {
            Ok(blob) => serde_json::from_slice(&content::open_blob(&key, &blob)?)
                .map_err(|e| Error::Verify(format!("chat state: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State {
                user,
                backups: true,
                ..Default::default()
            },
            Err(e) => return Err(e.into()),
        };
        let values = state
            .mls
            .iter()
            .filter_map(|(k, v)| Some((B64.decode(k).ok()?, B64.decode(v).ok()?)))
            .collect();
        Ok(Self {
            api,
            path,
            key,
            inner: Mutex::new(Inner {
                state,
                provider: mls::Provider::from_values(values),
                server: HashMap::new(),
            }),
        })
    }

    fn save(&self, inner: &mut Inner) -> Result<()> {
        inner.state.mls = inner
            .provider
            .values()
            .into_iter()
            .map(|(k, v)| (B64.encode(k), B64.encode(v)))
            .collect();
        let json = serde_json::to_vec(&inner.state).map_err(|e| Error::Verify(e.to_string()))?;
        let blob = content::seal_blob(&self.key, &json)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, blob)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn identity(inner: &Inner) -> Result<mls::Identity> {
        let device = inner
            .state
            .device
            .as_ref()
            .ok_or_else(|| Error::Install("set up chat on this device first".into()))?;
        mls::Identity::load(
            &inner.provider,
            &b64d(&device.public_key)?,
            inner.state.user,
            device.id,
        )
    }

    fn device_id(inner: &Inner) -> Result<Uuid> {
        inner
            .state
            .device
            .as_ref()
            .filter(|d| d.status == "active")
            .map(|d| d.id)
            .ok_or_else(|| Error::Install("chat isn't approved on this device yet".into()))
    }

    async fn call(
        &self,
        device: Option<Uuid>,
        method: Method,
        path: &str,
        body: RawBody,
    ) -> Result<RawResponse> {
        let headers: Vec<(&'static str, String)> = device
            .map(|d| vec![(DEVICE_HEADER, d.to_string())])
            .unwrap_or_default();
        self.api.request_raw(method, path, body, &headers).await
    }

    async fn call_ok(
        &self,
        device: Option<Uuid>,
        method: Method,
        path: &str,
        body: RawBody,
    ) -> Result<Value> {
        let res = self.call(device, method, path, body).await?;
        if !res.ok() {
            return Err(res.error());
        }
        Ok(res.json())
    }

    // -----------------------------------------------------------------------------------------
    // Setting up and approving devices

    pub async fn status(&self) -> Result<Status> {
        let devices: Vec<Value> = decode(
            "devices",
            self.call_ok(None, Method::GET, "/v1/chat/devices", RawBody::None)
                .await?,
        )?;
        let mut inner = self.inner.lock().await;
        // Approved elsewhere, removed, or replaced by another device starting over.
        if let Some(d) = inner.state.device.clone() {
            let status = devices
                .iter()
                .find(|v| v["id"] == json!(d.id))
                .and_then(|v| v["status"].as_str())
                .unwrap_or("revoked")
                .to_string();
            if status != d.status {
                if let Some(dev) = inner.state.device.as_mut() {
                    dev.status = status;
                }
                self.save(&mut inner)?;
            }
        }
        let this = inner.state.device.as_ref().map(|d| d.id);
        let mut pending = Vec::new();
        let mut list = Vec::new();
        for v in &devices {
            let id: Uuid = serde_json::from_value(v["id"].clone()).unwrap_or_default();
            let name = v["name"].as_str().unwrap_or("").to_string();
            let status = v["status"].as_str().unwrap_or("").to_string();
            if status == "pending" && Some(id) != this {
                pending.push(PendingDevice {
                    id,
                    name: name.clone(),
                    code: v["code"].as_str().unwrap_or("").to_string(),
                });
            }
            list.push(DeviceInfo {
                id,
                name,
                status,
                last_seen_at: v["last_seen_at"].as_str().unwrap_or("").to_string(),
                this_device: Some(id) == this,
            });
        }
        let d = inner.state.device.as_ref();
        Ok(Status {
            state: d.map(|d| d.status.clone()).unwrap_or_else(|| "none".into()),
            device_id: d.map(|d| d.id),
            code: d
                .and_then(|d| b64d(&d.public_key).ok())
                .map(|k| device_code(&k)),
            pending,
            devices: list,
            backups: inner.state.backups && inner.state.backup_key.is_some(),
        })
    }

    /// Sets chat up on this device: a new signing key and key packages, registered with Slop.
    /// The user's first device is ready at once; later ones wait for approval.
    pub async fn setup(&self, name: &str) -> Result<Status> {
        {
            let mut inner = self.inner.lock().await;
            if inner
                .state
                .device
                .as_ref()
                .is_some_and(|d| d.status != "revoked")
            {
                drop(inner);
                return self.status().await;
            }
            // A removed device starts again with fresh keys and no chats.
            inner.provider = mls::Provider::default();
            inner.state.conversations.clear();
            inner.state.cursor = 0;
            inner.state.roster.clear();
            let id = Uuid::now_v7();
            let signer = mls::Identity::generate(&inner.provider)?;
            let identity = mls::Identity::named(signer, inner.state.user, id);
            let mut packages = Vec::new();
            for _ in 0..KEY_PACKAGES {
                packages.push(B64.encode(identity.key_package(&inner.provider, false)?));
            }
            let last_resort = B64.encode(identity.key_package(&inner.provider, true)?);
            let public = identity.public_key();
            let res = self
                .call_ok(
                    None,
                    Method::POST,
                    "/v1/chat/devices",
                    RawBody::Json(json!({
                        "id": id, "name": name, "signature_key": B64.encode(&public),
                        "key_packages": packages, "last_resort": last_resort,
                    })),
                )
                .await?;
            inner.state.device = Some(DeviceState {
                id,
                name: name.to_string(),
                public_key: B64.encode(&public),
                status: res["status"].as_str().unwrap_or("pending").to_string(),
            });
            self.save(&mut inner)?;
        }
        self.status().await
    }

    /// Approves another of the user's devices after the user compared its code.
    pub async fn approve(&self, device: Uuid, code: &str) -> Result<()> {
        let (me, identity, user) = {
            let inner = self.inner.lock().await;
            (
                Self::device_id(&inner)?,
                Self::identity(&inner)?,
                inner.state.user,
            )
        };
        let devices = self
            .call_ok(None, Method::GET, "/v1/chat/devices", RawBody::None)
            .await?;
        let d = devices
            .as_array()
            .and_then(|a| {
                a.iter()
                    .find(|v| v["id"] == json!(device) && v["status"] == "pending")
            })
            .ok_or_else(|| Error::Install("that device isn't waiting for approval".into()))?;
        let key = b64d(d["signature_key"].as_str().unwrap_or(""))?;
        let normalize = |c: &str| {
            c.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_uppercase()
        };
        if normalize(&device_code(&key)) != normalize(code) {
            return Err(Error::Verify(
                "the codes don't match: don't approve that device".into(),
            ));
        }
        let signature = identity.sign(&approval_message(user, device, &key))?;
        self.call_ok(
            None,
            Method::POST,
            &format!("/v1/chat/devices/{device}/approve"),
            RawBody::Json(json!({ "by": me, "signature": B64.encode(signature) })),
        )
        .await?;
        Ok(())
    }

    /// Every other device lost: this one becomes the user's only device (needs a recent
    /// sign-in). Its chats are set up again by the people in them when they're next online.
    pub async fn start_over(&self) -> Result<()> {
        let device = {
            let inner = self.inner.lock().await;
            inner
                .state
                .device
                .clone()
                .ok_or_else(|| Error::Install("set up chat on this device first".into()))?
        };
        self.call_ok(
            None,
            Method::POST,
            &format!("/v1/chat/devices/{}/start-over", device.id),
            RawBody::None,
        )
        .await?;
        let mut inner = self.inner.lock().await;
        if let Some(d) = inner.state.device.as_mut() {
            d.status = "active".into();
        }
        self.save(&mut inner)
    }

    pub async fn remove_device(&self, device: Uuid) -> Result<()> {
        self.call_ok(
            None,
            Method::DELETE,
            &format!("/v1/chat/devices/{device}"),
            RawBody::None,
        )
        .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Syncing

    /// Fetches and processes what's new, waiting up to `wait` seconds for something to arrive,
    /// then keeps every chat's devices in step. Call it in a loop.
    pub async fn sync(&self, wait: u64) -> Result<Changes> {
        let (device, after) = {
            let inner = self.inner.lock().await;
            match inner.state.device.as_ref() {
                Some(d) if d.status == "active" => (d.id, inner.state.cursor),
                _ => return Ok(Changes::default()),
            }
        };
        let res = self
            .call(
                Some(device),
                Method::GET,
                &format!("/v1/chat/sync?after={after}&wait={wait}"),
                RawBody::None,
            )
            .await?;
        if res.status == 409 {
            // Removed or not approved any more.
            let mut inner = self.inner.lock().await;
            if let Some(d) = inner.state.device.as_mut() {
                d.status = "revoked".into();
            }
            self.save(&mut inner)?;
            return Ok(Changes {
                devices: true,
                ..Default::default()
            });
        }
        if !res.ok() {
            return Err(res.error());
        }
        let synced: Synced =
            serde_json::from_slice(&res.bytes).map_err(|e| Error::Verify(format!("sync: {e}")))?;
        // Names and mute settings for notifications, the first time round.
        if self.inner.lock().await.server.is_empty() {
            self.refresh_list().await?;
        }
        let mut changes = Changes {
            devices: synced.pending_devices > 0,
            ..Default::default()
        };
        let mut device_syncs = Vec::new();
        let roster_changed;
        {
            let mut inner = self.inner.lock().await;
            if inner.state.cursor != after {
                // Another call processed this range meanwhile.
                return Ok(changes);
            }
            for w in &synced.welcomes {
                if mls::load_group(&inner.provider, w.conversation_id)?.is_some() {
                    continue;
                }
                match mls::join(&inner.provider, &b64d(&w.welcome)?) {
                    Ok(_) => {
                        let conv = inner
                            .state
                            .conversations
                            .entry(w.conversation_id)
                            .or_default();
                        conv.joined_after = w.after_seq;
                        conv.left = false;
                        changes.list = true;
                    }
                    Err(e) => tracing::warn!(error = %e, "chat welcome"),
                }
            }
            for e in &synced.events {
                if e.sender_device_id == Some(device) {
                    continue;
                }
                let Some(mut group) = mls::load_group(&inner.provider, e.conversation_id)? else {
                    continue;
                };
                let joined_after = inner
                    .state
                    .conversations
                    .get(&e.conversation_id)
                    .map(|c| c.joined_after)
                    .unwrap_or(0);
                if e.seq <= joined_after {
                    continue;
                }
                let body = b64d(&e.body)?;
                match mls::process(&inner.provider, &mut group, &body) {
                    Ok(mls::Incoming::Commit { removed_me }) => {
                        if removed_me {
                            mls::delete_group(&inner.provider, e.conversation_id)?;
                            inner
                                .state
                                .conversations
                                .entry(e.conversation_id)
                                .or_default()
                                .left = true;
                        }
                        changes.list = true;
                    }
                    Ok(mls::Incoming::Application {
                        user, plaintext, ..
                    }) => {
                        // The server's sender must be the one MLS authenticated.
                        if e.sender_user_id != Some(user) {
                            tracing::warn!("chat message whose sender doesn't match");
                            continue;
                        }
                        match Content::parse(&plaintext) {
                            Ok(content) => {
                                let raw = String::from_utf8_lossy(&plaintext).to_string();
                                if let Body::Text {
                                    text, attachments, ..
                                } = &content.body
                                    && let Some(c) = inner.server.get(&e.conversation_id)
                                    && !c.muted
                                    && c.kind != "self"
                                {
                                    let me = inner.state.user;
                                    let local = inner.state.conversations.get(&e.conversation_id);
                                    changes.new_messages.push(NewMessage {
                                        conversation: e.conversation_id,
                                        title: title(c, local, me),
                                        sender: c
                                            .members
                                            .iter()
                                            .find(|m| m.id == user)
                                            .map(|m| m.name.clone())
                                            .unwrap_or_else(|| "Someone".into()),
                                        text: if !text.trim().is_empty() {
                                            text.chars().take(140).collect()
                                        } else if attachments
                                            .iter()
                                            .any(|a| a.mime.starts_with("image/"))
                                        {
                                            "Sent a picture".into()
                                        } else {
                                            "Sent a file".into()
                                        },
                                    });
                                }
                                if let Some(sync) = apply(
                                    &mut inner.state,
                                    e.conversation_id,
                                    e.seq,
                                    user,
                                    &e.created_at,
                                    content,
                                    Some(raw),
                                ) {
                                    device_syncs.push(sync);
                                }
                                changes.conversations.push(e.conversation_id);
                            }
                            Err(err) => tracing::warn!(error = %err, "chat message"),
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, seq = e.seq, "couldn't decrypt a chat message");
                        inner
                            .state
                            .conversations
                            .entry(e.conversation_id)
                            .or_default()
                            .undecryptable += 1;
                        changes.conversations.push(e.conversation_id);
                    }
                }
            }
            inner.state.cursor = synced.next;
            if !changes.conversations.is_empty() {
                inner.state.backup_dirty = true;
            }
            roster_changed = inner.state.roster != synced.roster;
            inner.state.roster = synced.roster.clone();
            self.save(&mut inner)?;
        }

        // From the user's own other devices: the backup key, and history for a new device.
        for sync in device_syncs {
            if let Err(e) = self.receive_device_sync(sync).await {
                tracing::warn!(error = %e, "chat device sync");
            }
        }
        if roster_changed || !synced.welcomes.is_empty() || !synced.events.is_empty() {
            changes.list = true;
        }
        self.reconcile_all(roster_changed).await?;
        if (synced.key_packages as usize) < KEY_PACKAGES / 2 {
            self.top_up_key_packages(device).await?;
        }
        if let Err(e) = self.maybe_backup().await {
            tracing::warn!(error = %e, "chat backup");
        }
        {
            let inner = self.inner.lock().await;
            changes.typing = synced
                .typing
                .iter()
                .map(|t| TypingView {
                    conversation: t.conversation_id,
                    user: t.user_id,
                    name: inner
                        .server
                        .get(&t.conversation_id)
                        .and_then(|c| c.members.iter().find(|m| m.id == t.user_id))
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| "Someone".into()),
                })
                .collect();
        }
        changes.conversations.sort();
        changes.conversations.dedup();
        Ok(changes)
    }

    /// Tells the others in a chat this user is typing (or stopped). Not encrypted, not stored:
    /// it says nothing but "typing", for a few seconds.
    pub async fn typing(&self, conversation: Uuid, typing: bool) -> Result<()> {
        let device = Self::device_id(&*self.inner.lock().await)?;
        self.call_ok(
            Some(device),
            Method::POST,
            &format!("/v1/chat/conversations/{conversation}/typing"),
            RawBody::Json(json!({ "typing": typing })),
        )
        .await?;
        Ok(())
    }

    async fn top_up_key_packages(&self, device: Uuid) -> Result<()> {
        let packages = {
            let mut inner = self.inner.lock().await;
            let identity = Self::identity(&inner)?;
            let mut packages = Vec::new();
            for _ in 0..KEY_PACKAGES {
                packages.push(B64.encode(identity.key_package(&inner.provider, false)?));
            }
            self.save(&mut inner)?;
            packages
        };
        self.call_ok(
            None,
            Method::POST,
            &format!("/v1/chat/devices/{device}/key-packages"),
            RawBody::Json(json!({ "key_packages": packages })),
        )
        .await?;
        Ok(())
    }

    async fn refresh_list(&self) -> Result<Vec<ServerConversation>> {
        let list: Vec<ServerConversation> = decode(
            "chats",
            self.call_ok(None, Method::GET, "/v1/chat/conversations", RawBody::None)
                .await?,
        )?;
        let mut inner = self.inner.lock().await;
        inner.server = list.iter().map(|c| (c.id, c.clone())).collect();
        Ok(list)
    }

    async fn reconcile_all(&self, force: bool) -> Result<()> {
        let mut list = self.refresh_list().await?;
        // The self chat keeps the user's devices together (history for new devices).
        if !list.iter().any(|c| c.kind == "self") {
            let _ = self
                .call_ok(
                    None,
                    Method::POST,
                    "/v1/chat/conversations",
                    RawBody::Json(json!({ "kind": "self" })),
                )
                .await;
            list = self.refresh_list().await?;
        }
        for c in &list {
            let needs = {
                let inner = self.inner.lock().await;
                force
                    || mls::load_group(&inner.provider, c.id)?.is_none()
                    || inner.state.out_of_sync.contains_key(&c.id)
            };
            if needs && let Err(e) = self.reconcile(c).await {
                tracing::warn!(error = %e, conversation = %c.id, "chat devices");
            }
        }
        Ok(())
    }

    /// Checks devices' approvals back to each user's first device, and notes when someone's
    /// first device (their security code) changed. Returns the active devices that check out.
    fn verify_devices(inner: &mut Inner, devices: &[ServerDevice]) -> Vec<(Uuid, Uuid, Vec<u8>)> {
        let mut by_user: HashMap<Uuid, Vec<&ServerDevice>> = HashMap::new();
        for d in devices {
            by_user.entry(d.user_id).or_default().push(d);
        }
        let mut out = Vec::new();
        for (user, devs) in by_user {
            let mut root_key: Option<Vec<u8>> = None;
            for d in devs.iter().filter(|d| d.status == "active") {
                let Ok(key) = b64d(&d.signature_key) else {
                    continue;
                };
                let mut current: &ServerDevice = d;
                let mut current_key = key.clone();
                let mut ok = false;
                for _ in 0..32 {
                    let Some(by) = current.approved_by else {
                        ok = true;
                        root_key = Some(current_key.clone());
                        break;
                    };
                    let Some(approver) = devs.iter().find(|x| x.id == by) else {
                        break;
                    };
                    let (Ok(approver_key), Some(Ok(sig))) = (
                        b64d(&approver.signature_key),
                        current.approval_sig.as_deref().map(b64d),
                    ) else {
                        break;
                    };
                    if !mls::verify_signature(
                        &inner.provider,
                        &approver_key,
                        &approval_message(user, current.id, &current_key),
                        &sig,
                    ) {
                        break;
                    }
                    current = approver;
                    current_key = approver_key;
                }
                if ok {
                    out.push((user, d.id, key));
                } else {
                    tracing::warn!(%user, device = %d.id, "chat device whose approval doesn't check out");
                }
            }
            let Some(root) = root_key.map(|r| B64.encode(r)) else {
                continue;
            };
            match inner.state.roots.get(&user) {
                Some(pinned) if *pinned != root => {
                    inner.state.roots.insert(user, root);
                    if user == inner.state.user {
                        continue;
                    }
                    let ids: Vec<Uuid> = inner
                        .server
                        .values()
                        .filter(|c| c.members.iter().any(|m| m.id == user))
                        .map(|c| c.id)
                        .collect();
                    let name = inner
                        .server
                        .values()
                        .flat_map(|c| c.members.iter())
                        .find(|m| m.id == user)
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| "Someone".into());
                    for id in ids {
                        push_notice(
                            &mut inner.state,
                            id,
                            format!(
                                "{name}'s security code changed: they set chat up again on a new device. Compare codes if you want to be sure it's them."
                            ),
                        );
                    }
                }
                Some(_) => {}
                None => {
                    inner.state.roots.insert(user, root);
                }
            }
        }
        out
    }

    /// Keeps one chat's encryption in step with its members' devices: creates the group if no
    /// one has, adds new devices and removes old ones.
    async fn reconcile(&self, conv: &ServerConversation) -> Result<()> {
        let devices: Vec<ServerDevice> = decode(
            "devices",
            self.call_ok(
                None,
                Method::GET,
                &format!("/v1/chat/conversations/{}/devices", conv.id),
                RawBody::None,
            )
            .await?,
        )?;
        let (me, desired, add, remove, creating, epoch) = {
            let mut inner = self.inner.lock().await;
            let me = Self::device_id(&inner)?;
            let identity = Self::identity(&inner)?;
            let desired = Self::verify_devices(&mut inner, &devices);
            let (group, creating) = match mls::load_group(&inner.provider, conv.id)? {
                Some(g) => (g, false),
                None if conv.epoch == 0 => (
                    mls::create_group(&inner.provider, &identity, conv.id)?,
                    true,
                ),
                None => {
                    // Someone else set it up: this device waits for its Welcome.
                    self.save(&mut inner)?;
                    return Ok(());
                }
            };
            let actual = mls::members(&group);
            let remove: Vec<_> = actual
                .iter()
                .filter(|m| {
                    m.device != me
                        && !desired.iter().any(|(u, d, k)| {
                            *u == m.user && *d == m.device && *k == m.signature_key
                        })
                })
                .map(|m| m.index)
                .collect();
            let add: Vec<Uuid> = desired
                .iter()
                .filter(|(_, d, _)| *d != me && !actual.iter().any(|m| m.device == *d))
                .map(|(_, d, _)| *d)
                .collect();
            if !creating && remove.is_empty() && add.is_empty() {
                inner.state.out_of_sync.remove(&conv.id);
                self.save(&mut inner)?;
                return Ok(());
            }
            // One device fixes it, so commits don't race: the group's first device at once,
            // others if it's still not fixed a little later.
            if !creating {
                let first = actual.iter().map(|m| m.device).min();
                let since = *inner.state.out_of_sync.entry(conv.id).or_insert_with(now);
                if first != Some(me) && now() - since < FIX_AFTER_SECS {
                    self.save(&mut inner)?;
                    return Ok(());
                }
            }
            let epoch = mls::epoch(&group);
            self.save(&mut inner)?;
            (me, desired, add, remove, creating, epoch)
        };
        let claimed: Vec<Value> = if add.is_empty() {
            vec![]
        } else {
            decode(
                "key packages",
                self.call_ok(
                    Some(me),
                    Method::POST,
                    &format!("/v1/chat/conversations/{}/key-packages", conv.id),
                    RawBody::Json(json!({ "devices": add })),
                )
                .await?,
            )?
        };
        let (commit, welcome_for) = {
            let mut inner = self.inner.lock().await;
            let identity = Self::identity(&inner)?;
            let mut group = mls::load_group(&inner.provider, conv.id)?
                .ok_or_else(|| Error::Verify("the chat's group went missing".into()))?;
            if mls::epoch(&group) != epoch {
                // Something arrived meanwhile; look again next time.
                return Ok(());
            }
            let mut packages = Vec::new();
            let mut welcome_for = Vec::new();
            for c in &claimed {
                if let (Some(d), Some(kp)) = (
                    c["device_id"].as_str().and_then(|d| d.parse::<Uuid>().ok()),
                    c["key_package"].as_str(),
                ) {
                    packages.push(b64d(kp)?);
                    welcome_for.push(d);
                }
            }
            if !creating && packages.is_empty() && remove.is_empty() {
                return Ok(());
            }
            let commit = match mls::change_members(
                &inner.provider,
                &identity,
                &mut group,
                &packages,
                &remove,
            ) {
                Ok(c) => c,
                Err(e) => {
                    if creating {
                        mls::delete_group(&inner.provider, conv.id)?;
                    } else {
                        let _ = mls::clear_pending(&inner.provider, &mut group);
                    }
                    self.save(&mut inner)?;
                    return Err(e);
                }
            };
            self.save(&mut inner)?;
            (commit, welcome_for)
        };
        let welcomes: Vec<Value> = match &commit.welcome {
            Some(w) => welcome_for
                .iter()
                .map(|d| json!({ "device_id": d, "welcome": B64.encode(w) }))
                .collect(),
            None => vec![],
        };
        let res = self
            .call(
                Some(me),
                Method::POST,
                &format!("/v1/chat/conversations/{}/commit", conv.id),
                RawBody::Json(json!({
                    "epoch": epoch, "commit": B64.encode(&commit.commit), "welcomes": welcomes,
                })),
            )
            .await?;
        let own_new = {
            let mut inner = self.inner.lock().await;
            let mut group = mls::load_group(&inner.provider, conv.id)?
                .ok_or_else(|| Error::Verify("the chat's group went missing".into()))?;
            if !res.ok() {
                // Someone else's commit won: drop ours and catch up on the next sync.
                if creating {
                    mls::delete_group(&inner.provider, conv.id)?;
                } else {
                    mls::clear_pending(&inner.provider, &mut group)?;
                }
                self.save(&mut inner)?;
                return if res.status == 409 {
                    Ok(())
                } else {
                    Err(res.error())
                };
            }
            mls::merge_pending(&inner.provider, &mut group)?;
            let seq = res.json()["seq"].as_i64().unwrap_or(0);
            let user = inner.state.user;
            let conv_state = inner.state.conversations.entry(conv.id).or_default();
            if creating {
                conv_state.joined_after = seq;
            }
            conv_state.left = false;
            inner.state.out_of_sync.remove(&conv.id);
            self.save(&mut inner)?;
            welcome_for
                .iter()
                .any(|d| desired.iter().any(|(u, dd, _)| dd == d && *u == user))
        };
        // A new device of the user's own: send it the backup key and a copy of the history.
        if conv.kind == "self" && own_new {
            // Boxed: sending can sync, which can reconcile again.
            Box::pin(self.send_device_sync(conv.id)).await?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Chats

    pub async fn conversations(&self) -> Result<Vec<ConversationView>> {
        let list = self.refresh_list().await?;
        let inner = self.inner.lock().await;
        let me = inner.state.user;
        let mut out = Vec::new();
        for c in list.into_iter().filter(|c| c.kind != "self") {
            let local = inner.state.conversations.get(&c.id);
            let last = local
                .and_then(|l| l.messages.iter().rev().find(|m| !m.notice))
                .map(|m| {
                    if m.deleted {
                        "Message deleted".to_string()
                    } else if !m.text.is_empty() {
                        m.text.chars().take(80).collect()
                    } else if !m.attachments.is_empty() {
                        "📎 Attachment".into()
                    } else {
                        String::new()
                    }
                });
            out.push(ConversationView {
                title: title(&c, local, me),
                id: c.id,
                kind: c.kind.clone(),
                members: c.members.clone(),
                team_slug: c.team_slug.clone(),
                unread: c.unread,
                muted: c.muted,
                ready: mls::load_group(&inner.provider, c.id)?.is_some(),
                closed: c.closed,
                last,
                last_at: c.last_message_at.clone(),
                owner: c.created_by == Some(me),
            });
        }
        Ok(out)
    }

    async fn start(&self, body: Value) -> Result<Uuid> {
        let c: ServerConversation = decode(
            "chat",
            self.call_ok(
                None,
                Method::POST,
                "/v1/chat/conversations",
                RawBody::Json(body),
            )
            .await?,
        )?;
        self.inner.lock().await.server.insert(c.id, c.clone());
        if let Err(e) = self.reconcile(&c).await {
            tracing::warn!(error = %e, "setting up a chat");
        }
        Ok(c.id)
    }

    /// A direct chat with someone (by profile link or ID): the existing one, or a new one.
    pub async fn start_direct(&self, user: &str) -> Result<Uuid> {
        self.start(json!({ "kind": "direct", "user": user })).await
    }

    pub async fn start_group(&self, users: &[String], name: Option<&str>) -> Result<Uuid> {
        let id = self
            .start(json!({ "kind": "group", "users": users }))
            .await?;
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            self.send_content(
                id,
                Body::Rename {
                    name: name.chars().take(80).collect(),
                },
            )
            .await?;
        }
        Ok(id)
    }

    pub async fn start_team(&self, team: &str) -> Result<Uuid> {
        self.start(json!({ "kind": "team", "team": team })).await
    }

    pub async fn add_people(&self, conversation: Uuid, users: &[String]) -> Result<()> {
        self.call_ok(
            None,
            Method::POST,
            &format!("/v1/chat/conversations/{conversation}/members"),
            RawBody::Json(json!({ "users": users })),
        )
        .await?;
        self.reconcile_all(true).await
    }

    pub async fn leave(&self, conversation: Uuid) -> Result<()> {
        let user = self.inner.lock().await.state.user;
        self.call_ok(
            None,
            Method::DELETE,
            &format!("/v1/chat/conversations/{conversation}/members/{user}"),
            RawBody::None,
        )
        .await?;
        let mut inner = self.inner.lock().await;
        mls::delete_group(&inner.provider, conversation)?;
        inner
            .state
            .conversations
            .entry(conversation)
            .or_default()
            .left = true;
        self.save(&mut inner)
    }

    pub async fn remove_person(&self, conversation: Uuid, user: Uuid) -> Result<()> {
        self.call_ok(
            None,
            Method::DELETE,
            &format!("/v1/chat/conversations/{conversation}/members/{user}"),
            RawBody::None,
        )
        .await?;
        self.reconcile_all(true).await
    }

    pub async fn hide(&self, conversation: Uuid) -> Result<()> {
        self.call_ok(
            None,
            Method::POST,
            &format!("/v1/chat/conversations/{conversation}/hide"),
            RawBody::None,
        )
        .await?;
        Ok(())
    }

    pub async fn set_muted(&self, conversation: Uuid, muted: bool) -> Result<()> {
        self.call_ok(
            None,
            Method::PUT,
            &format!("/v1/chat/conversations/{conversation}/mute"),
            RawBody::Json(json!({ "muted": muted })),
        )
        .await?;
        Ok(())
    }

    pub async fn mark_read(&self, conversation: Uuid) -> Result<()> {
        let seq = {
            let inner = self.inner.lock().await;
            inner
                .state
                .conversations
                .get(&conversation)
                .and_then(|c| c.messages.iter().map(|m| m.seq).max())
                .unwrap_or(0)
        };
        if seq > 0 {
            self.call_ok(
                None,
                Method::POST,
                &format!("/v1/chat/conversations/{conversation}/read"),
                RawBody::Json(json!({ "seq": seq })),
            )
            .await?;
        }
        Ok(())
    }

    /// Messages in a chat, oldest first: the latest `limit`, or those before `before`.
    pub async fn messages(
        &self,
        conversation: Uuid,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageView>> {
        let inner = self.inner.lock().await;
        let me = inner.state.user;
        let Some(local) = inner.state.conversations.get(&conversation) else {
            return Ok(vec![]);
        };
        let names: HashMap<Uuid, String> = inner
            .server
            .get(&conversation)
            .map(|c| c.members.iter().map(|m| (m.id, m.name.clone())).collect())
            .unwrap_or_default();
        let name = |id: &Uuid| -> String {
            if *id == me {
                "You".into()
            } else {
                names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "Someone who left".into())
            }
        };
        let all: Vec<&Message> = local
            .messages
            .iter()
            .filter(|m| before.is_none_or(|b| m.seq < b))
            .collect();
        let start = all.len().saturating_sub(limit);
        let mut out: Vec<MessageView> = all[start..]
            .iter()
            .map(|m| MessageView {
                id: m.id,
                seq: m.seq,
                sender: Person {
                    id: m.sender,
                    name: name(&m.sender),
                    avatar_url: None,
                },
                mine: m.sender == me,
                at: m.at.clone(),
                text: if m.deleted {
                    String::new()
                } else {
                    m.text.clone()
                },
                reply_to: m.reply_to.and_then(|r| {
                    local
                        .messages
                        .iter()
                        .find(|x| x.id == r)
                        .map(|x| ReplyView {
                            id: x.id,
                            sender: name(&x.sender),
                            text: if x.deleted {
                                "Message deleted".into()
                            } else if x.text.is_empty() {
                                "📎 Attachment".into()
                            } else {
                                x.text.chars().take(140).collect()
                            },
                        })
                }),
                attachments: if m.deleted {
                    vec![]
                } else {
                    m.attachments.iter().map(attachment_view).collect()
                },
                preview: m
                    .preview
                    .as_ref()
                    .filter(|_| !m.deleted)
                    .map(|p| PreviewView {
                        url: p.url.clone(),
                        site: p.site.clone(),
                        title: p.title.clone(),
                        description: p.description.clone(),
                        image: p.image.as_ref().map(attachment_view),
                    }),
                reactions: m
                    .reactions
                    .iter()
                    .filter(|(_, who)| !who.is_empty())
                    .map(|(emoji, who)| ReactionView {
                        emoji: emoji.clone(),
                        count: who.len(),
                        mine: who.contains(&me),
                        names: who.iter().map(&name).collect(),
                    })
                    .collect(),
                edited: m.edited,
                deleted: m.deleted,
                notice: m.notice,
                warnings: if m.sender != me && !m.notice {
                    payment_warning(&m.text).into_iter().collect()
                } else {
                    vec![]
                },
                reportable: m.sender != me && m.plaintext.is_some() && !m.notice,
            })
            .collect();
        if local.undecryptable > 0 && before.is_none() {
            out.push(MessageView {
                id: Uuid::nil(),
                seq: i64::MAX,
                sender: Person {
                    id: Uuid::nil(),
                    name: String::new(),
                    avatar_url: None,
                },
                mine: false,
                at: String::new(),
                text: format!(
                    "{} message{} couldn't be decrypted on this device.",
                    local.undecryptable,
                    if local.undecryptable == 1 { "" } else { "s" }
                ),
                reply_to: None,
                attachments: vec![],
                preview: None,
                reactions: vec![],
                edited: false,
                deleted: false,
                notice: true,
                warnings: vec![],
                reportable: false,
            });
        }
        Ok(out)
    }

    /// Sends a text message with any files, and a link preview (or the linked meme itself)
    /// when `link_preview` is on and the text has a link.
    pub async fn send(
        &self,
        conversation: Uuid,
        text: &str,
        reply_to: Option<Uuid>,
        files: Vec<OutgoingFile>,
        link_preview: bool,
    ) -> Result<Uuid> {
        let text = text.trim().to_string();
        if text.is_empty() && files.is_empty() {
            return Err(Error::Install("write something first".into()));
        }
        if text.chars().count() > 4000 {
            return Err(Error::Install(
                "messages are 4,000 characters at most".into(),
            ));
        }
        let mut attachments = Vec::new();
        for f in files {
            attachments.push(
                self.upload(conversation, &f.bytes, &f.mime, &f.name, None)
                    .await?,
            );
        }
        let mut preview = None;
        if link_preview && let Some(url) = links::first_link(&text) {
            match links::fetch(&url).await {
                Ok(links::Fetched::Media { mime, bytes, name }) => {
                    if let Ok(a) = self
                        .upload(conversation, &bytes, &mime, &name, Some(url.to_string()))
                        .await
                    {
                        attachments.push(a);
                    }
                }
                Ok(links::Fetched::Page(p)) if p.title.is_some() || p.description.is_some() => {
                    let image = match &p.image_url {
                        Some(u) => match links::fetch_image(u).await {
                            Ok((mime, bytes)) => self
                                .upload(conversation, &bytes, &mime, "preview", None)
                                .await
                                .ok(),
                            Err(_) => None,
                        },
                        None => None,
                    };
                    preview = Some(LinkPreview {
                        site: url
                            .host_str()
                            .unwrap_or("")
                            .trim_start_matches("www.")
                            .to_string(),
                        url: url.to_string(),
                        title: p.title,
                        description: p.description,
                        image,
                    });
                }
                _ => {}
            }
        }
        self.send_content(
            conversation,
            Body::Text {
                text,
                reply_to,
                attachments,
                preview,
            },
        )
        .await
    }

    pub async fn react(
        &self,
        conversation: Uuid,
        target: Uuid,
        emoji: &str,
        on: bool,
    ) -> Result<()> {
        if emoji.is_empty() || emoji.chars().count() > 8 {
            return Err(Error::Install("pick one emoji".into()));
        }
        self.send_content(
            conversation,
            Body::Reaction {
                target,
                emoji: emoji.into(),
                on,
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn edit(&self, conversation: Uuid, target: Uuid, text: &str) -> Result<()> {
        self.own_message(conversation, target).await?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > 4000 {
            return Err(Error::Install("1 to 4,000 characters".into()));
        }
        self.send_content(
            conversation,
            Body::Edit {
                target,
                text: text.into(),
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn delete(&self, conversation: Uuid, target: Uuid) -> Result<()> {
        self.own_message(conversation, target).await?;
        self.send_content(conversation, Body::Delete { target })
            .await
            .map(|_| ())
    }

    pub async fn rename(&self, conversation: Uuid, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(Error::Install("1 to 80 characters".into()));
        }
        self.send_content(conversation, Body::Rename { name: name.into() })
            .await
            .map(|_| ())
    }

    async fn own_message(&self, conversation: Uuid, target: Uuid) -> Result<()> {
        let inner = self.inner.lock().await;
        let me = inner.state.user;
        let ok = inner
            .state
            .conversations
            .get(&conversation)
            .and_then(|c| c.messages.iter().find(|m| m.id == target))
            .is_some_and(|m| m.sender == me && !m.deleted);
        if ok {
            Ok(())
        } else {
            Err(Error::Install(
                "you can only change your own messages".into(),
            ))
        }
    }

    async fn upload(
        &self,
        conversation: Uuid,
        bytes: &[u8],
        mime: &str,
        name: &str,
        source_url: Option<String>,
    ) -> Result<AttachmentRef> {
        if bytes.is_empty() || bytes.len() > 15 * 1024 * 1024 {
            return Err(Error::Install(format!("{name}: files are 15 MB at most")));
        }
        let device = Self::device_id(&*self.inner.lock().await)?;
        let mime = if mime.is_empty() {
            "application/octet-stream"
        } else {
            mime
        };
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
            .take(120)
            .collect();
        let (ciphertext, mut a) = content::encrypt_attachment(bytes, mime, &name)?;
        let res = self
            .call_ok(
                Some(device),
                Method::POST,
                &format!("/v1/chat/conversations/{conversation}/attachments"),
                RawBody::Bytes(ciphertext),
            )
            .await?;
        a.id = decode("attachment", res["id"].clone())?;
        a.source_url = source_url;
        Ok(a)
    }

    /// An attachment's bytes, decrypted, and its type.
    pub async fn attachment(
        &self,
        conversation: Uuid,
        message: Uuid,
        attachment: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        let a = {
            let inner = self.inner.lock().await;
            inner
                .state
                .conversations
                .get(&conversation)
                .and_then(|c| c.messages.iter().find(|m| m.id == message))
                .and_then(|m| {
                    m.attachments
                        .iter()
                        .chain(m.preview.as_ref().and_then(|p| p.image.as_ref()))
                        .find(|a| a.id == attachment)
                        .cloned()
                })
                .ok_or_else(|| Error::Install("no such attachment".into()))?
        };
        let res = self
            .call(
                None,
                Method::GET,
                &format!("/v1/chat/attachments/{}", a.id),
                RawBody::None,
            )
            .await?;
        if res.status == 404 {
            return Err(Error::Install(
                "this attachment is no longer available (they're kept for 30 days)".into(),
            ));
        }
        if !res.ok() {
            return Err(res.error());
        }
        Ok((a.mime.clone(), content::decrypt_attachment(&a, &res.bytes)?))
    }

    /// Encrypts and sends one message, catching up and retrying if the chat moved on.
    async fn send_content(&self, conversation: Uuid, body: Body) -> Result<Uuid> {
        let content = Content::new(body);
        let (plaintext, tag) = content.seal()?;
        for attempt in 0..3 {
            let prepared = {
                let mut inner = self.inner.lock().await;
                let device = Self::device_id(&inner)?;
                let identity = Self::identity(&inner)?;
                match mls::load_group(&inner.provider, conversation)? {
                    Some(mut group) => {
                        let epoch = mls::epoch(&group);
                        let ciphertext =
                            mls::encrypt(&inner.provider, &identity, &mut group, &plaintext)?;
                        self.save(&mut inner)?;
                        Some((device, epoch, ciphertext))
                    }
                    None => None,
                }
            };
            let Some((device, epoch, ciphertext)) = prepared else {
                if attempt == 0 {
                    // Not set up on this device yet: try now.
                    self.reconcile_all(true).await?;
                    continue;
                }
                return Err(Error::Install(
                    "this chat is still being set up on this device; try again in a moment".into(),
                ));
            };
            let res = self
                .call(
                    Some(device),
                    Method::POST,
                    &format!("/v1/chat/conversations/{conversation}/messages"),
                    RawBody::Json(json!({
                        "epoch": epoch, "body": B64.encode(&ciphertext), "franking_tag": B64.encode(tag),
                    })),
                )
                .await?;
            if res.status == 409 && res.json()["error"]["code"] == "stale_epoch" {
                self.sync(0).await?;
                continue;
            }
            if !res.ok() {
                return Err(res.error());
            }
            let seq = res.json()["seq"].as_i64().unwrap_or(0);
            let mut inner = self.inner.lock().await;
            let me = inner.state.user;
            apply(
                &mut inner.state,
                conversation,
                seq,
                me,
                &now_rfc3339(),
                content.clone(),
                None,
            );
            inner.state.backup_dirty = true;
            self.save(&mut inner)?;
            return Ok(content.id);
        }
        Err(Error::Install(
            "the chat kept changing while sending; try again".into(),
        ))
    }

    // -----------------------------------------------------------------------------------------
    // Safety: security codes and reports

    /// Everyone's security codes in a chat, to compare in person or on a call.
    pub async fn security_codes(&self, conversation: Uuid) -> Result<Vec<SecurityCode>> {
        let inner = self.inner.lock().await;
        let c = inner
            .server
            .get(&conversation)
            .ok_or_else(|| Error::Install("no such chat".into()))?;
        Ok(c.members
            .iter()
            .filter_map(|m| {
                let root = b64d(inner.state.roots.get(&m.id)?).ok()?;
                Some(SecurityCode {
                    user: m.id,
                    name: if m.id == inner.state.user {
                        "You".into()
                    } else {
                        m.name.clone()
                    },
                    code: content::security_code(m.id, &root),
                })
            })
            .collect())
    }

    /// Reports someone's message to Slop's moderators, with its decrypted text and franking key
    /// so Slop can check it's genuine (only this message is revealed).
    pub async fn report(
        &self,
        conversation: Uuid,
        message: Uuid,
        reason: &str,
        details: &str,
    ) -> Result<Value> {
        let (seq, plaintext) = {
            let inner = self.inner.lock().await;
            let m = inner
                .state
                .conversations
                .get(&conversation)
                .and_then(|c| c.messages.iter().find(|m| m.id == message))
                .ok_or_else(|| Error::Install("no such message".into()))?;
            (
                m.seq,
                m.plaintext.clone().ok_or_else(|| {
                    Error::Install("only other people's messages can be reported".into())
                })?,
            )
        };
        let content = Content::parse(plaintext.as_bytes())?;
        self.call_ok(
            None,
            Method::POST,
            "/v1/reports",
            RawBody::Json(json!({
                "kind": "chat_message", "subject": conversation.to_string(), "reason": reason,
                "details": details, "good_faith": true,
                "chat": {
                    "conversation_id": conversation, "seq": seq, "plaintext": plaintext,
                    "franking_key": content.fk,
                },
            })),
        )
        .await
    }

    // -----------------------------------------------------------------------------------------
    // History for new devices, and backups

    fn history(state: &State, except: Option<Uuid>) -> Result<Vec<u8>> {
        let history: BTreeMap<Uuid, LocalConversation> = state
            .conversations
            .iter()
            .filter(|(id, _)| Some(**id) != except)
            .map(|(id, c)| {
                (
                    *id,
                    LocalConversation {
                        left: false,
                        joined_after: 0,
                        undecryptable: 0,
                        ..c.clone()
                    },
                )
            })
            .collect();
        serde_json::to_vec(&history).map_err(|e| Error::Verify(e.to_string()))
    }

    async fn send_device_sync(&self, self_chat: Uuid) -> Result<()> {
        let (history, backup_key) = {
            let inner = self.inner.lock().await;
            (
                Self::history(&inner.state, Some(self_chat))?,
                inner.state.backup_key.clone(),
            )
        };
        let attachment = self
            .upload(
                self_chat,
                &history,
                "application/x-slop-chat-history",
                "history",
                None,
            )
            .await?;
        self.send_content(
            self_chat,
            Body::DeviceSync {
                backup_key,
                history: Some(attachment),
            },
        )
        .await?;
        Ok(())
    }

    async fn receive_device_sync(&self, sync: DeviceSyncIn) -> Result<()> {
        if let Some(key) = sync.backup_key {
            let mut inner = self.inner.lock().await;
            inner.state.backup_key = Some(key);
            inner.state.backups = true;
            self.save(&mut inner)?;
        }
        if let Some(a) = sync.history {
            let res = self
                .call(
                    None,
                    Method::GET,
                    &format!("/v1/chat/attachments/{}", a.id),
                    RawBody::None,
                )
                .await?;
            if res.ok() {
                let bytes = content::decrypt_attachment(&a, &res.bytes)?;
                let history: BTreeMap<Uuid, LocalConversation> =
                    serde_json::from_slice(&bytes).map_err(|e| Error::Verify(e.to_string()))?;
                let mut inner = self.inner.lock().await;
                merge_history(&mut inner.state, history);
                self.save(&mut inner)?;
            }
        }
        Ok(())
    }

    /// Turns the encrypted backup on (returning the new recovery code, shown once) or off
    /// (deleting the copy at Slop).
    pub async fn set_backups(&self, on: bool) -> Result<Option<String>> {
        if !on {
            let res = self
                .call(None, Method::DELETE, "/v1/chat/backup", RawBody::None)
                .await?;
            if !res.ok() {
                return Err(res.error());
            }
            let mut inner = self.inner.lock().await;
            inner.state.backups = false;
            inner.state.backup_key = None;
            self.save(&mut inner)?;
            return Ok(None);
        }
        let code = content::new_recovery_code();
        let self_chat = {
            let mut inner = self.inner.lock().await;
            let key = content::backup_key(&code, inner.state.user)?;
            inner.state.backup_key = Some(B64.encode(key));
            inner.state.backups = true;
            inner.state.backup_dirty = true;
            inner.state.last_backup_at = 0;
            self.save(&mut inner)?;
            inner
                .server
                .values()
                .find(|c| c.kind == "self")
                .map(|c| c.id)
        };
        self.maybe_backup().await?;
        // The user's other devices get the new key.
        if let Some(id) = self_chat {
            let key = self.inner.lock().await.state.backup_key.clone();
            let _ = self
                .send_content(
                    id,
                    Body::DeviceSync {
                        backup_key: key,
                        history: None,
                    },
                )
                .await;
        }
        Ok(Some(code))
    }

    async fn maybe_backup(&self) -> Result<()> {
        let (key, blob, device) = {
            let inner = self.inner.lock().await;
            let due = inner.state.backups
                && inner.state.backup_dirty
                && now() - inner.state.last_backup_at >= BACKUP_EVERY_SECS;
            let Some(key) = inner.state.backup_key.clone().filter(|_| due) else {
                return Ok(());
            };
            (
                key,
                Self::history(&inner.state, None)?,
                Self::device_id(&inner)?,
            )
        };
        let key: [u8; 32] = b64d(&key)?
            .try_into()
            .map_err(|_| Error::Verify("bad backup key".into()))?;
        let sealed = content::seal_blob(&key, &blob)?;
        self.call_ok(
            Some(device),
            Method::PUT,
            "/v1/chat/backup",
            RawBody::Bytes(sealed),
        )
        .await?;
        let mut inner = self.inner.lock().await;
        inner.state.backup_dirty = false;
        inner.state.last_backup_at = now();
        self.save(&mut inner)
    }

    /// The key for save sync (save_sync.rs), from the backup key the user's devices share.
    /// None until this device has it (chat set up here, with backups on).
    pub async fn save_sync_key(&self) -> Option<[u8; 32]> {
        let key = self.inner.lock().await.state.backup_key.clone()?;
        let bytes: [u8; 32] = B64.decode(key).ok()?.try_into().ok()?;
        Some(crate::save_sync::derive_key(&bytes))
    }

    /// Restores history from the backup with the recovery code (after starting over).
    pub async fn restore_backup(&self, recovery_code: &str) -> Result<usize> {
        let user = self.inner.lock().await.state.user;
        let key = content::backup_key(recovery_code, user)?;
        let res = self
            .call(None, Method::GET, "/v1/chat/backup", RawBody::None)
            .await?;
        if res.status == 404 {
            return Err(Error::Install("there's no backup to restore".into()));
        }
        if !res.ok() {
            return Err(res.error());
        }
        let bytes = content::open_blob(&key, &res.bytes)
            .map_err(|_| Error::Verify("that recovery code doesn't open the backup".into()))?;
        let history: BTreeMap<Uuid, LocalConversation> =
            serde_json::from_slice(&bytes).map_err(|e| Error::Verify(e.to_string()))?;
        let count = history.values().map(|c| c.messages.len()).sum();
        let mut inner = self.inner.lock().await;
        merge_history(&mut inner.state, history);
        inner.state.backup_key = Some(B64.encode(key));
        inner.state.backups = true;
        self.save(&mut inner)?;
        Ok(count)
    }
}

struct DeviceSyncIn {
    backup_key: Option<String>,
    history: Option<AttachmentRef>,
}

fn title(c: &ServerConversation, local: Option<&LocalConversation>, me: Uuid) -> String {
    match c.kind.as_str() {
        "team" => c.team_name.clone().unwrap_or_else(|| "Team chat".into()),
        "self" => "Your devices".into(),
        "direct" => c
            .members
            .iter()
            .find(|m| m.id != me)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Chat".into()),
        _ => local.and_then(|l| l.name.clone()).unwrap_or_else(|| {
            let names: Vec<&str> = c
                .members
                .iter()
                .filter(|m| m.id != me)
                .map(|m| m.name.as_str())
                .take(4)
                .collect();
            if names.is_empty() {
                "Group chat".into()
            } else {
                names.join(", ")
            }
        }),
    }
}

fn push_notice(state: &mut State, conversation: Uuid, text: String) {
    let conv = state.conversations.entry(conversation).or_default();
    let seq = conv.messages.last().map(|m| m.seq).unwrap_or(0);
    conv.messages.push(Message {
        id: Uuid::now_v7(),
        seq,
        sender: Uuid::nil(),
        at: now_rfc3339(),
        text,
        reply_to: None,
        attachments: vec![],
        preview: None,
        notice: true,
        reactions: BTreeMap::new(),
        edited: false,
        deleted: false,
        plaintext: None,
    });
}

/// Applies one decrypted message to the history. Returns a device sync to handle (self chat).
fn apply(
    state: &mut State,
    conversation: Uuid,
    seq: i64,
    sender: Uuid,
    at: &str,
    content: Content,
    plaintext: Option<String>,
) -> Option<DeviceSyncIn> {
    let me = state.user;
    let conv = state.conversations.entry(conversation).or_default();
    match content.body {
        Body::Text {
            text,
            reply_to,
            attachments,
            preview,
        } => {
            if conv.messages.iter().any(|m| m.id == content.id) {
                return None;
            }
            conv.messages.push(Message {
                id: content.id,
                seq,
                sender,
                at: at.to_string(),
                text: text.chars().take(4000).collect(),
                reply_to,
                attachments,
                preview,
                notice: false,
                reactions: BTreeMap::new(),
                edited: false,
                deleted: false,
                plaintext,
            });
            if conv.messages.len() > KEEP_MESSAGES {
                let extra = conv.messages.len() - KEEP_MESSAGES;
                conv.messages.drain(..extra);
            }
        }
        Body::Reaction { target, emoji, on } => {
            if let Some(m) = conv.messages.iter_mut().find(|m| m.id == target) {
                let who = m.reactions.entry(emoji).or_default();
                if on {
                    who.insert(sender);
                } else {
                    who.remove(&sender);
                }
            }
        }
        Body::Edit { target, text } => {
            if let Some(m) = conv
                .messages
                .iter_mut()
                .find(|m| m.id == target && m.sender == sender)
            {
                m.text = text.chars().take(4000).collect();
                m.edited = true;
                m.preview = None;
            }
        }
        Body::Delete { target } => {
            if let Some(m) = conv
                .messages
                .iter_mut()
                .find(|m| m.id == target && m.sender == sender)
            {
                m.deleted = true;
                m.text.clear();
                m.attachments.clear();
                m.preview = None;
                m.plaintext = None;
            }
        }
        Body::Rename { name } => {
            conv.name = Some(name.clone());
            let who = if sender == me { "You" } else { "Someone" };
            push_notice(
                state,
                conversation,
                format!("{who} named the group “{name}”"),
            );
        }
        Body::DeviceSync {
            backup_key,
            history,
        } => {
            // Only from the user's own devices.
            if sender == me {
                return Some(DeviceSyncIn {
                    backup_key,
                    history,
                });
            }
        }
    }
    None
}

fn merge_history(state: &mut State, history: BTreeMap<Uuid, LocalConversation>) {
    for (id, theirs) in history {
        let ours = state.conversations.entry(id).or_default();
        if ours.name.is_none() {
            ours.name = theirs.name;
        }
        for m in theirs.messages {
            if !ours.messages.iter().any(|x| x.id == m.id) {
                ours.messages.push(m);
            }
        }
        ours.messages
            .sort_by(|a, b| a.seq.cmp(&b.seq).then(a.at.cmp(&b.at)));
    }
}

/// Chat's secrets in the OS secure store (SECURITY §4.2). Unavailable stores refuse, and chat
/// says it needs one, rather than keeping keys in a plain file.
#[cfg(feature = "os-keyring")]
pub struct KeyringSecrets {
    service: String,
}

#[cfg(feature = "os-keyring")]
impl KeyringSecrets {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }
}

#[cfg(feature = "os-keyring")]
impl SecretStore for KeyringSecrets {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        let value = keyring::Entry::new(&self.service, name)
            .ok()?
            .get_password()
            .ok()?;
        B64.decode(value).ok()
    }
    fn set(&self, name: &str, value: &[u8]) -> bool {
        keyring::Entry::new(&self.service, name)
            .and_then(|e| e.set_password(&B64.encode(value)))
            .inspect_err(
                |e| tracing::warn!(error = %e, "couldn't save a chat key to the OS secure store"),
            )
            .is_ok()
    }
}
