// SPDX-License-Identifier: AGPL-3.0-or-later
//! Phone and browser notifications through web push (DESIGN §19).
//!
//! SLAPorSLOP has no phone app, so phones get notifications through the website: Android browsers
//! directly, iPhones once SLAPorSLOP is added to the home screen. A push carries **no content**: it
//! only wakes the phone, whose service worker (`web/public/sw.js`) fetches the newest
//! notification from SLAPorSLOP with the user's own sign-in and shows it. The push service (Google,
//! Apple, Mozilla, Microsoft) learns only that something happened. Chat notices are already
//! content-free ("New message from Mia").
//!
//! Pushes are signed with SLAPorSLOP's VAPID key (RFC 8292). Only the big push services' addresses are
//! accepted, so a subscription can't make SLAPorSLOP's server call anywhere else.

use std::time::Duration as StdDuration;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    config::PushConfig,
    error::{ApiError, ApiResult},
};

/// Subscriptions one account may have (phones and browsers).
const MAX_PER_USER: i64 = 10;
/// The push services browsers use. Endpoints elsewhere are refused.
pub const PUSH_HOSTS: &[&str] = &[
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "push.services.mozilla.com",
    "web.push.apple.com",
    "api.push.apple.com",
    "wns2-par02p.notify.windows.com",
];
/// Host suffixes push services use for regional servers.
const PUSH_SUFFIXES: &[&str] = &[".notify.windows.com", ".push.apple.com"];

pub fn routes() -> Router<AppState> {
    Router::new().route("/push/key", get(key)).route(
        "/me/push",
        axum::routing::post(subscribe).delete(unsubscribe),
    )
}

/// A VAPID key: the private scalar, base64url. `slop-api keys generate-vapid` makes one.
pub fn parse_key(b64: &str) -> anyhow::Result<SigningKey> {
    let bytes = URL_SAFE_NO_PAD.decode(b64.trim())?;
    Ok(SigningKey::from_slice(&bytes)?)
}

/// The public key browsers subscribe with: the uncompressed point, base64url.
pub fn public_key(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes())
}

pub fn generate() -> (String, String) {
    let key = SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    (URL_SAFE_NO_PAD.encode(key.to_bytes()), public_key(&key))
}

/// The VAPID `Authorization` header for a push to `endpoint` (RFC 8292): an ES256 JWT for the
/// push service's origin, valid 12 hours.
pub fn authorization(config: &PushConfig, endpoint: &Url, now: i64) -> String {
    let origin = endpoint.origin().ascii_serialization();
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(
        json!({ "aud": origin, "exp": now + 12 * 3600, "sub": config.subject }).to_string(),
    );
    let signing_input = format!("{header}.{claims}");
    let signature: Signature = config.key.sign(signing_input.as_bytes());
    let jwt = format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );
    format!("vapid t={jwt}, k={}", public_key(&config.key))
}

fn allowed(config: &PushConfig, endpoint: &str) -> Option<Url> {
    let url = Url::parse(endpoint).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let known = PUSH_HOSTS.contains(&host.as_str())
        || PUSH_SUFFIXES.iter().any(|s| host.ends_with(s))
        || config.extra_hosts.contains(&host);
    let scheme_ok = url.scheme() == "https" || (config.allow_http && url.scheme() == "http");
    (known && scheme_ok && url.username().is_empty()).then_some(url)
}

#[derive(Serialize)]
struct Key {
    public_key: String,
}

async fn key(State(state): State<AppState>) -> ApiResult<Json<Key>> {
    let config = state.config.push.as_ref().ok_or(ApiError::Unavailable(
        "notifications on phones aren't switched on on this server",
    ))?;
    Ok(Json(Key {
        public_key: public_key(&config.key),
    }))
}

#[derive(Deserialize)]
struct Subscription {
    endpoint: String,
    /// The browser's encryption keys. Pushes carry no data, so they aren't kept.
    #[serde(default)]
    #[allow(dead_code)]
    keys: Option<serde_json::Value>,
}

async fn subscribe(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Subscription>,
) -> ApiResult<StatusCode> {
    let config = state.config.push.as_ref().ok_or(ApiError::Unavailable(
        "notifications on phones aren't switched on on this server",
    ))?;
    let endpoint = allowed(config, &b.endpoint)
        .ok_or_else(|| ApiError::invalid("endpoint", "not a push service SLAPorSLOP knows"))?;
    let mut tx = state.db.begin().await?;
    // The account's others: one it already has, posted again, doesn't count twice.
    let count: i64 = sqlx::query_scalar(
        "select count(*) from push_subscriptions where user_id = $1 and endpoint <> $2",
    )
    .bind(user.id)
    .bind(endpoint.as_str())
    .fetch_one(&mut *tx)
    .await?;
    if count >= MAX_PER_USER {
        // The oldest goes, so a new phone always works.
        sqlx::query(
            "delete from push_subscriptions where id = (
                 select id from push_subscriptions where user_id = $1 and endpoint <> $2
                 order by created_at limit 1)",
        )
        .bind(user.id)
        .bind(endpoint.as_str())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "insert into push_subscriptions (id, user_id, endpoint) values ($1, $2, $3)
         on conflict (endpoint) do update set user_id = excluded.user_id, failures = 0",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(endpoint.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

async fn unsubscribe(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Unsubscribe>,
) -> ApiResult<StatusCode> {
    sqlx::query("delete from push_subscriptions where user_id = $1 and endpoint = $2")
        .bind(user.id)
        .bind(&b.endpoint)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Pushes each new notification to its user's phones and browsers. Returns how many pushes
/// were sent.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let Some(config) = &state.config.push else {
        return Ok(0);
    };
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "update notifications n set pushed_at = now()
         where n.id in (select n2.id from notifications n2
                        where n2.pushed_at is null and n2.read_at is null
                          and n2.created_at > now() - interval '10 minutes'
                          and exists (select 1 from push_subscriptions s where s.user_id = n2.user_id)
                        limit 200)
         returning n.id, n.user_id",
    )
    .fetch_all(&state.db)
    .await?;
    let mut users: Vec<Uuid> = due.iter().map(|(_, u)| *u).collect();
    users.sort();
    users.dedup();
    let mut sent = 0;
    for user in users {
        let subs: Vec<(Uuid, String)> =
            sqlx::query_as("select id, endpoint from push_subscriptions where user_id = $1")
                .bind(user)
                .fetch_all(&state.db)
                .await?;
        for (id, endpoint) in subs {
            let Some(url) = allowed(config, &endpoint) else {
                continue;
            };
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let res = state
                .http
                .post(url.clone())
                .header("authorization", authorization(config, &url, now))
                .header("ttl", "3600")
                .header("urgency", "normal")
                .header("content-length", "0")
                .timeout(StdDuration::from_secs(10))
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => {
                    sent += 1;
                    sqlx::query(
                        "update push_subscriptions set last_pushed_at = now(), failures = 0 where id = $1",
                    )
                    .bind(id)
                    .execute(&state.db)
                    .await?;
                }
                // Gone: the user switched it off in the browser, or uninstalled.
                Ok(r) if r.status() == StatusCode::NOT_FOUND || r.status() == StatusCode::GONE => {
                    sqlx::query("delete from push_subscriptions where id = $1")
                        .bind(id)
                        .execute(&state.db)
                        .await?;
                }
                other => {
                    if let Err(e) = &other {
                        tracing::warn!(error = %e, "push");
                    }
                    sqlx::query(
                        "update push_subscriptions set failures = failures + 1 where id = $1",
                    )
                    .bind(id)
                    .execute(&state.db)
                    .await?;
                    sqlx::query("delete from push_subscriptions where id = $1 and failures >= 20")
                        .bind(id)
                        .execute(&state.db)
                        .await?;
                }
            }
        }
    }
    Ok(sent)
}

pub async fn run(state: AppState) {
    if state.config.push.is_none() {
        return;
    }
    loop {
        if let Err(e) = run_once(&state).await {
            tracing::error!(error = %e, "push notifications");
        }
        tokio::time::sleep(StdDuration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};

    #[test]
    fn vapid_tokens_are_signed_for_the_push_service() {
        let (private, public) = generate();
        let config = PushConfig {
            key: parse_key(&private).unwrap(),
            subject: "mailto:ops@slop.example".into(),
            extra_hosts: vec![],
            allow_http: false,
        };
        assert_eq!(public_key(&config.key), public);
        let endpoint = Url::parse("https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let header = authorization(&config, &endpoint, 1_000);
        let (t, k) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(k, public);
        let parts: Vec<&str> = t.split('.').collect();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["exp"], 1_000 + 12 * 3600);
        let key = VerifyingKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(k).unwrap()).unwrap();
        let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        key.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .unwrap();
    }

    #[test]
    fn only_push_services_are_called() {
        let config = PushConfig {
            key: parse_key(&generate().0).unwrap(),
            subject: "mailto:a@b".into(),
            extra_hosts: vec![],
            allow_http: false,
        };
        for ok in [
            "https://fcm.googleapis.com/fcm/send/x",
            "https://web.push.apple.com/abc",
            "https://db5p.notify.windows.com/w/?token=x",
            "https://updates.push.services.mozilla.com/wpush/v2/x",
        ] {
            assert!(allowed(&config, ok).is_some(), "{ok}");
        }
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://evil.example/x",
            "https://fcm.googleapis.com.evil.example/x",
            "https://127.0.0.1/x",
        ] {
            assert!(allowed(&config, bad).is_none(), "{bad}");
        }
    }
}
