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
