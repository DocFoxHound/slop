//! GitHub OAuth: authorization code flow with PKCE (SECURITY §4.1).
//!
//! Slop asks only for `user:email` and keeps the GitHub access token just long enough to read
//! the account ID, name and primary verified email. It's never stored.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use url::Url;

use crate::config::GithubConfig;

pub const SCOPE: &str = "user:email";
const USER_AGENT: &str = "slop-api";

#[derive(Debug)]
pub struct GithubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
}

pub fn authorize_url(cfg: &GithubConfig, redirect_uri: &str, state: &str, challenge: &str) -> Url {
    let mut url = cfg
        .oauth_base
        .join("login/oauth/authorize")
        .expect("valid GitHub OAuth base");
    url.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("allow_signup", "true");
    url
}

/// Exchanges the authorization code for an access token.
pub async fn exchange_code(
    http: &reqwest::Client,
    cfg: &GithubConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        error: Option<String>,
    }

    let response: TokenResponse = http
        .post(cfg.oauth_base.join("login/oauth/access_token")?)
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .context("GitHub token request")?
        .error_for_status()
        .context("GitHub token request")?
        .json()
        .await
        .context("GitHub token response")?;

    // GitHub reports a bad code with HTTP 200 and an `error` field.
    response.access_token.ok_or_else(|| {
        anyhow!(
            "GitHub token exchange failed: {}",
            response.error.unwrap_or_default()
        )
    })
}

pub async fn fetch_user(
    http: &reqwest::Client,
    cfg: &GithubConfig,
    token: &str,
) -> anyhow::Result<GithubUser> {
    #[derive(Deserialize)]
    struct User {
        id: i64,
        login: String,
        name: Option<String>,
    }

    let user: User = api_get(http, cfg, token, "user").await?;
    Ok(GithubUser {
        id: user.id,
        login: user.login,
        name: user.name,
    })
}

/// The account's primary email, only if GitHub has verified it.
pub async fn fetch_primary_email(
    http: &reqwest::Client,
    cfg: &GithubConfig,
    token: &str,
) -> anyhow::Result<Option<String>> {
    #[derive(Deserialize)]
    struct Email {
        email: String,
        primary: bool,
        verified: bool,
    }

    let emails: Vec<Email> = api_get(http, cfg, token, "user/emails").await?;
    Ok(emails
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email))
}

async fn api_get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    cfg: &GithubConfig,
    token: &str,
    path: &str,
) -> anyhow::Result<T> {
    http.get(cfg.api_base.join(path)?)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", USER_AGENT)
        .send()
        .await
        .with_context(|| format!("GitHub GET /{path}"))?
        .error_for_status()
        .with_context(|| format!("GitHub GET /{path}"))?
        .json()
        .await
        .with_context(|| format!("GitHub GET /{path} response"))
}
