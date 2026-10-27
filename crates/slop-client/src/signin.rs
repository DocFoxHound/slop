// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signing in through the system browser (SECURITY §4.1, RFC 8252 loopback redirect).
//!
//! 1. [`begin`] listens on a random port on 127.0.0.1 and asks the API for a sign-in request,
//!    sending only a PKCE challenge and that port.
//! 2. The caller opens [`PendingSignIn::approve_url`] in the system browser, where the user signs
//!    in on the website and approves this device.
//! 3. [`PendingSignIn::finish`] waits for the browser to arrive at the loopback address with a
//!    one-time code, checks the state, and swaps the code (plus the PKCE verifier) for tokens.

use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{
    Error, Result,
    api::{ApiClient, api_error},
    random_token,
};

pub struct PendingSignIn {
    pub approve_url: String,
    listener: TcpListener,
    state: String,
    verifier: String,
}

#[derive(Deserialize)]
struct Created {
    approve_url: String,
}

pub async fn begin(api: &ApiClient, device_name: &str) -> Result<PendingSignIn> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let state = random_token();
    let verifier = random_token();
    let res = api
        .request_anonymous(
            Method::POST,
            "/v1/auth/desktop/requests",
            Some(serde_json::json!({
                "code_challenge": slop_pkce_challenge(&verifier),
                "redirect_port": port,
                "state": state,
                "device_name": device_name,
            })),
        )
        .await?;
    if res.status != 200 {
        return Err(api_error(&res));
    }
    let created: Created =
        serde_json::from_value(res.body).map_err(|e| Error::Verify(e.to_string()))?;
    // Only ever open the configured SLAPorSLOP site in the browser.
    if !created
        .approve_url
        .starts_with(api.base().as_str().trim_end_matches('/'))
    {
        return Err(Error::Verify(
            "sign-in page isn't on the SLAPorSLOP site".into(),
        ));
    }
    Ok(PendingSignIn {
        approve_url: created.approve_url,
        listener,
        state,
        verifier,
    })
}

fn slop_pkce_challenge(verifier: &str) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>SLAPorSLOP</title>\
<body style=\"font:16px system-ui;background:#0d1015;color:#e7ecf2;display:grid;place-items:center;height:100vh;margin:0\">\
<div><h1 style=\"color:#a3e635\">You're signed in</h1><p>You can close this tab and go back to SLAPorSLOP.</p></div>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>SLAPorSLOP</title>\
<body style=\"font:16px system-ui;background:#0d1015;color:#e7ecf2;display:grid;place-items:center;height:100vh;margin:0\">\
<div><h1>Sign-in didn't work</h1><p>Go back to SLAPorSLOP and try again.</p></div>";

impl PendingSignIn {
    /// Waits for the browser to come back, then stores the new tokens in `api`.
    pub async fn finish(self, api: &ApiClient, timeout: Duration) -> Result<()> {
        let code = tokio::time::timeout(timeout, self.wait_for_code())
            .await
            .map_err(|_| Error::Install("sign-in timed out; please try again".into()))??;
        api.exchange_code(&code, &self.verifier).await
    }

    async fn wait_for_code(&self) -> Result<String> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            let Ok(Ok(target)) =
                tokio::time::timeout(Duration::from_secs(5), read_request_target(&mut stream))
                    .await
            else {
                continue;
            };
            let Some(query) = target.strip_prefix("/callback?") else {
                // Browsers also ask for /favicon.ico and the like.
                let _ = respond(&mut stream, "404 Not Found", "").await;
                continue;
            };
            let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
            let get = |k: &str| {
                params
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.clone())
            };
            match (get("code"), get("state")) {
                (Some(code), Some(state))
                    if constant_time_eq(state.as_bytes(), self.state.as_bytes()) =>
                {
                    let _ = respond(&mut stream, "200 OK", DONE_PAGE).await;
                    return Ok(code);
                }
                _ => {
                    // Not our sign-in: keep waiting for the real one.
                    let _ = respond(&mut stream, "400 Bad Request", FAILED_PAGE).await;
                }
            }
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Reads an HTTP request head and returns the request target of a GET.
async fn read_request_target(stream: &mut TcpStream) -> Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await?;
        if n == 0 || buf.len() > 8192 {
            return Err(Error::Install("bad sign-in callback".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Ok(target.to_string()),
        _ => Err(Error::Install("bad sign-in callback".into())),
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}
