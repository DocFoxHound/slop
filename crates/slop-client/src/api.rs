// SPDX-License-Identifier: AGPL-3.0-or-later
//! The API client. Tokens never leave Rust: the UI asks for a request and gets back the status
//! and JSON body (SECURITY §7: a compromised page can't read the user's tokens).

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{Error, Result};

/// Where the refresh token lives between runs. The desktop app uses the OS secure store
/// (SECURITY §4.2); tests use memory.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, refresh_token: &str);
    fn clear(&self);
}

#[derive(Default)]
pub struct MemoryTokenStore(Mutex<Option<String>>);

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, refresh_token: &str) {
        *self.0.lock().unwrap() = Some(refresh_token.to_string());
    }
    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Value,
}

#[derive(Debug, Clone)]
pub enum RawBody {
    None,
    Json(Value),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub bytes: Vec<u8>,
    /// SLAPorSLOP's own `x-slop-…` headers (lowercase names).
    pub slop_headers: Vec<(String, String)>,
}

impl RawResponse {
    /// One of SLAPorSLOP's own `x-slop-…` headers.
    pub fn header(&self, name: &str) -> Option<String> {
        self.slop_headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The API's error, for a non-2xx response.
    pub fn error(&self) -> Error {
        api_error(&ApiResponse {
            status: self.status,
            body: self.json(),
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

struct Inner {
    base: Url,
    http: reqwest::Client,
    store: Arc<dyn TokenStore>,
    access: Mutex<Option<(String, Instant)>>,
    /// Refresh tokens rotate, and reusing one revokes the session, so only one refresh may run
    /// at a time.
    refresh_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct ApiClient {
    inner: Arc<Inner>,
}

/// Paths the UI may call: API routes only, no tricks.
fn checked_path(path: &str) -> Result<&str> {
    let (route, _query) = path.split_once('?').unwrap_or((path, ""));
    let ok = route.starts_with("/v1/")
        && !route.contains("//")
        && !route.split('/').any(|s| s == "." || s == "..")
        && !path
            .chars()
            .any(|c| c.is_control() || c == '#' || c == '\\')
        && path.len() <= 2048;
    if ok {
        Ok(path)
    } else {
        Err(Error::Install(format!("refusing request path {path:?}")))
    }
}

impl ApiClient {
    pub fn new(base: Url, store: Arc<dyn TokenStore>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("slop-desktop/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                base,
                http,
                store,
                access: Mutex::new(None),
                refresh_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub fn base(&self) -> &Url {
        &self.inner.base
    }

    /// Whether a refresh token is stored. It may still turn out to be revoked.
    pub fn has_session(&self) -> bool {
        self.inner.store.load().is_some()
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.inner
            .base
            .join(checked_path(path)?)
            .map_err(|e| Error::Install(e.to_string()))
    }

    /// Calls the API as the signed-in user (or signed out, if there's no session).
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<ApiResponse> {
        let token = self.access_token().await?;
        let res = self
            .send(method.clone(), path, body.clone(), token.as_deref())
            .await?;
        if res.status == 401 && token.is_some() {
            // The access token was revoked or expired early: refresh once and retry.
            *self.inner.access.lock().unwrap() = None;
            let token = self.refresh(token.as_deref()).await?;
            return self.send(method, path, body, token.as_deref()).await;
        }
        Ok(res)
    }

    /// Calls the API as the signed-in user with extra headers and a JSON or raw body, returning
    /// the raw response (chat: the device header, encrypted attachments).
    pub async fn request_raw(
        &self,
        method: Method,
        path: &str,
        body: RawBody,
        headers: &[(&'static str, String)],
    ) -> Result<RawResponse> {
        let token = self.access_token().await?;
        let res = self
            .send_raw(
                method.clone(),
                path,
                body.clone(),
                headers,
                token.as_deref(),
            )
            .await?;
        if res.status == 401 && token.is_some() {
            *self.inner.access.lock().unwrap() = None;
            let token = self.refresh(token.as_deref()).await?;
            return self
                .send_raw(method, path, body, headers, token.as_deref())
                .await;
        }
        Ok(res)
    }

    async fn send_raw(
        &self,
        method: Method,
        path: &str,
        body: RawBody,
        headers: &[(&'static str, String)],
        token: Option<&str>,
    ) -> Result<RawResponse> {
        // Attachments can be big and connections slow.
        let mut req = self
            .inner
            .http
            .request(method, self.url(path)?)
            .timeout(Duration::from_secs(300));
        for (name, value) in headers {
            req = req.header(*name, value);
        }
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        req = match body {
            RawBody::None => req,
            RawBody::Json(v) => req.json(&v),
            RawBody::Bytes(b) => req
                .header("content-type", "application/octet-stream")
                .body(b),
        };
        let res = req.send().await?;
        let status = res.status().as_u16();
        let slop_headers = res
            .headers()
            .iter()
            .filter(|(n, _)| n.as_str().starts_with("x-slop-"))
            .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let bytes = res.bytes().await?.to_vec();
        Ok(RawResponse {
            status,
            bytes,
            slop_headers,
        })
    }

    /// Calls the API without credentials.
    pub async fn request_anonymous(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<ApiResponse> {
        self.send(method, path, body, None).await
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> Result<ApiResponse> {
        let mut req = self
            .inner
            .http
            .request(method, self.url(path)?)
            .header("accept", "application/json");
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let bytes = res.bytes().await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        Ok(ApiResponse { status, body })
    }

    /// GETs JSON and fails on any non-2xx status.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let res = self.request(Method::GET, path, None).await?;
        if !(200..300).contains(&res.status) {
            return Err(api_error(&res));
        }
        serde_json::from_value(res.body)
            .map_err(|e| Error::Verify(format!("unexpected response: {e}")))
    }

    /// A valid access token, refreshing if needed. `None` when signed out.
    async fn access_token(&self) -> Result<Option<String>> {
        if let Some((token, expires)) = self.inner.access.lock().unwrap().clone()
            && Instant::now() + Duration::from_secs(30) < expires
        {
            return Ok(Some(token));
        }
        if !self.has_session() {
            return Ok(None);
        }
        self.refresh(None).await
    }

    /// Swaps the stored refresh token for new tokens. `stale` is the access token the caller
    /// saw fail; if another task already refreshed past it, that result is reused.
    async fn refresh(&self, stale: Option<&str>) -> Result<Option<String>> {
        let _guard = self.inner.refresh_lock.lock().await;
        if let Some((token, expires)) = self.inner.access.lock().unwrap().clone()
            && Some(token.as_str()) != stale
            && Instant::now() + Duration::from_secs(30) < expires
        {
            return Ok(Some(token));
        }
        let Some(refresh_token) = self.inner.store.load() else {
            return Ok(None);
        };
        let res = self
            .send(
                Method::POST,
                "/v1/auth/desktop/token",
                Some(serde_json::json!({ "grant_type": "refresh_token", "refresh_token": refresh_token })),
                None,
            )
            .await?;
        if res.status == 401 {
            self.forget();
            return Ok(None);
        }
        if res.status != 200 {
            return Err(api_error(&res));
        }
        Ok(Some(self.store_tokens(res.body)?))
    }

    fn store_tokens(&self, body: Value) -> Result<String> {
        let tokens: TokenResponse = serde_json::from_value(body)
            .map_err(|e| Error::Verify(format!("bad token response: {e}")))?;
        self.inner.store.save(&tokens.refresh_token);
        let expires = Instant::now() + Duration::from_secs(tokens.expires_in);
        *self.inner.access.lock().unwrap() = Some((tokens.access_token.clone(), expires));
        Ok(tokens.access_token)
    }

    /// Finishes desktop sign-in: swaps the one-time code for tokens.
    pub(crate) async fn exchange_code(&self, code: &str, verifier: &str) -> Result<()> {
        let res = self
            .send(
                Method::POST,
                "/v1/auth/desktop/token",
                Some(serde_json::json!({
                    "grant_type": "authorization_code", "code": code, "code_verifier": verifier
                })),
                None,
            )
            .await?;
        if res.status != 200 {
            return Err(api_error(&res));
        }
        self.store_tokens(res.body)?;
        Ok(())
    }

    /// Signs this device out on the server (best effort) and forgets its tokens.
    pub async fn sign_out(&self) -> Result<()> {
        if self.has_session() {
            let _ = self.request(Method::POST, "/v1/auth/signout", None).await;
        }
        self.forget();
        Ok(())
    }

    fn forget(&self) {
        self.inner.store.clear();
        *self.inner.access.lock().unwrap() = None;
    }
}

pub(crate) fn api_error(res: &ApiResponse) -> Error {
    let err = &res.body["error"];
    Error::Api {
        status: res.status,
        code: err["code"].as_str().unwrap_or("http").to_string(),
        message: err["message"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("request failed ({})", res.status)),
    }
}

/// The refresh token in the OS secure store (SECURITY §4.2). If the store isn't available
/// (e.g. no Secret Service on a minimal Linux desktop), the token is kept in memory only and
/// the user signs in again next time; it's never written to a plain file.
#[cfg(feature = "os-keyring")]
pub struct KeyringTokenStore {
    entry: Option<keyring::Entry>,
    fallback: MemoryTokenStore,
}

#[cfg(feature = "os-keyring")]
impl KeyringTokenStore {
    pub fn new(service: &str) -> Self {
        let entry = keyring::Entry::new(service, "refresh-token")
            .inspect_err(|e| tracing::warn!(error = %e, "OS secure store unavailable; sign-in won't be remembered"))
            .ok();
        Self {
            entry,
            fallback: MemoryTokenStore::default(),
        }
    }
}

#[cfg(feature = "os-keyring")]
impl TokenStore for KeyringTokenStore {
    fn load(&self) -> Option<String> {
        match &self.entry {
            Some(entry) => entry.get_password().ok(),
            None => self.fallback.load(),
        }
    }
    fn save(&self, refresh_token: &str) {
        match &self.entry {
            Some(entry) => {
                if let Err(e) = entry.set_password(refresh_token) {
                    tracing::warn!(error = %e, "couldn't save the sign-in to the OS secure store");
                }
            }
            None => self.fallback.save(refresh_token),
        }
    }
    fn clear(&self) {
        match &self.entry {
            Some(entry) => {
                let _ = entry.delete_credential();
            }
            None => self.fallback.clear(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_api_paths_are_allowed() {
        for ok in ["/v1/apps", "/v1/apps?q=a%20b", "/v1/apps/x/manifest"] {
            assert!(checked_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "/healthz",
            "https://evil.example/v1/x",
            "//evil.example/v1",
            "/v1/../admin",
            "/v1/a\nb",
            "v1/apps",
            "/v1/a#b",
        ] {
            assert!(checked_path(bad).is_err(), "{bad}");
        }
    }
}
