//! Slop's GitHub App (decided 2026-10-19).
//!
//! Makers install the App on their repository. GitHub then:
//!
//! - **tells Slop when a release is published** (a webhook), so the release watcher checks that
//!   app straight away instead of polling it every few minutes;
//! - **gives that repository its own API allowance**: requests about it use a short-lived
//!   installation token instead of Slop's shared `GITHUB_TOKEN`, so Slop's request budget grows
//!   with the number of makers instead of running out.
//!
//! The App only needs read access to a repository's contents and metadata. Without it configured
//! (GITHUB_APP_*), everything works as before on the shared token.

use std::collections::HashMap;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use time::{Duration, OffsetDateTime};
use url::Url;

use crate::{
    AppState,
    config::GithubAppConfig,
    error::{ApiError, ApiResult},
};

/// The webhook can carry a long release description, so it gets its own body limit.
pub const MAX_WEBHOOK_BYTES: usize = 5 * 1024 * 1024;

pub fn webhook_routes() -> Router<AppState> {
    Router::new().route("/github/webhook", post(webhook))
}

/// Installation tokens by installation ID, with when they expire. GitHub's last an hour.
pub type TokenCache = tokio::sync::Mutex<HashMap<i64, (String, OffsetDateTime)>>;

/// The repository (`owner/repo`, lower case) a GitHub API URL is about, if any.
fn repo_of_api_url(base: &Url, url: &Url) -> Option<String> {
    let rest = url.as_str().strip_prefix(base.as_str())?;
    let mut parts = rest.split(['/', '?', '#']);
    if parts.next()? != "repos" {
        return None;
    }
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}").to_ascii_lowercase())
}

async fn installation_for(state: &AppState, repo_path: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar("select installation_id from github_installations where repo_path = $1")
        .bind(repo_path.to_ascii_lowercase())
        .fetch_optional(&state.db)
        .await
}

/// Whether the App is installed on this repository (`owner/repo`).
pub async fn installed(state: &AppState, repo_path: &str) -> sqlx::Result<bool> {
    Ok(installation_for(state, repo_path).await?.is_some())
}

/// The App's install page, where a maker picks repositories.
pub fn install_url(app: &GithubAppConfig) -> String {
    format!("https://github.com/apps/{}/installations/new", app.slug)
}

/// The token for a GitHub API request: the repository's installation token when the App is
/// installed there, else Slop's shared token. The flag says whether it's the shared one.
pub async fn auth_for(state: &AppState, url: &Url) -> Option<(String, bool)> {
    let cfg = &state.config.releases;
    if let (Some(app), Some(repo)) = (&cfg.github_app, repo_of_api_url(&cfg.github_api_base, url)) {
        match installation_for(state, &repo).await {
            Ok(Some(id)) => match installation_token(state, app, id).await {
                Ok(token) => return Some((token, false)),
                Err(err) => tracing::warn!(%repo, error = %err, "GitHub App token"),
            },
            Ok(None) => {}
            Err(err) => tracing::warn!(%repo, error = %err, "GitHub App lookup"),
        }
    }
    cfg.github_token.clone().map(|t| (t, true))
}

async fn installation_token(
    state: &AppState,
    app: &GithubAppConfig,
    id: i64,
) -> anyhow::Result<String> {
    let now = OffsetDateTime::now_utc();
    if let Some((token, expires)) = state.github_app_tokens.lock().await.get(&id)
        && *expires - now > Duration::minutes(5)
    {
        return Ok(token.clone());
    }

    // A JWT signed with the App's key proves the request is the App's (valid 10 minutes at most;
    // backdated a minute for clock drift).
    #[derive(Serialize)]
    struct Claims<'a> {
        iat: i64,
        exp: i64,
        iss: &'a str,
    }
    let unix = now.unix_timestamp();
    let jwt = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &Claims {
            iat: unix - 60,
            exp: unix + 9 * 60,
            iss: &app.app_id,
        },
        &app.private_key,
    )?;
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("app/installations/{id}/access_tokens"))?;
    let res = state
        .http
        .post(url)
        .bearer_auth(jwt)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "slop-api")
        .send()
        .await?;
    if res.status() == StatusCode::NOT_FOUND {
        // Uninstalled while Slop missed the webhook.
        forget_installation(state, id).await?;
        anyhow::bail!("installation {id} is gone");
    }
    #[derive(Deserialize)]
    struct Token {
        token: String,
        expires_at: String,
    }
    let token: Token = res.error_for_status()?.json().await?;
    let expires = OffsetDateTime::parse(
        &token.expires_at,
        &time::format_description::well_known::Rfc3339,
    )?;
    state
        .github_app_tokens
        .lock()
        .await
        .insert(id, (token.token.clone(), expires));
    Ok(token.token)
}

async fn forget_installation(state: &AppState, id: i64) -> sqlx::Result<()> {
    sqlx::query("delete from github_installations where installation_id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    state.github_app_tokens.lock().await.remove(&id);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Webhooks

/// Whether `X-Hub-Signature-256` is GitHub's HMAC of the body with the webhook secret.
fn signature_ok(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(sig) = header
        .and_then(|h| h.strip_prefix("sha256="))
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

#[derive(Deserialize)]
struct Repo {
    full_name: String,
}

#[derive(Deserialize)]
struct Installation {
    id: i64,
}

#[derive(Deserialize)]
struct Event {
    action: Option<String>,
    installation: Option<Installation>,
    /// installation events: the repositories it covers.
    #[serde(default)]
    repositories: Vec<Repo>,
    #[serde(default)]
    repositories_added: Vec<Repo>,
    #[serde(default)]
    repositories_removed: Vec<Repo>,
    /// release events: where it was published.
    repository: Option<Repo>,
}

async fn remember(state: &AppState, id: i64, repos: &[Repo]) -> sqlx::Result<()> {
    for repo in repos {
        sqlx::query(
            "insert into github_installations (repo_path, installation_id) values ($1, $2)
             on conflict (repo_path) do update set installation_id = $2, updated_at = now()",
        )
        .bind(repo.full_name.to_ascii_lowercase())
        .bind(id)
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let Some(app) = &state.config.releases.github_app else {
        return Err(ApiError::NotFound);
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if !signature_ok(&app.webhook_secret, &body, header("x-hub-signature-256")) {
        return Err(ApiError::Unauthorized);
    }
    let kind = header("x-github-event").unwrap_or_default();
    if kind == "ping" {
        return Ok(StatusCode::NO_CONTENT);
    }
    let event: Event = serde_json::from_slice(&body)
        .map_err(|_| ApiError::invalid("body", "not a GitHub webhook"))?;
    let action = event.action.as_deref().unwrap_or_default();
    let Some(installation) = event.installation.as_ref().map(|i| i.id) else {
        return Ok(StatusCode::NO_CONTENT);
    };

    match kind {
        "installation" => match action {
            "deleted" | "suspend" => forget_installation(&state, installation).await?,
            _ => remember(&state, installation, &event.repositories).await?,
        },
        "installation_repositories" => {
            remember(&state, installation, &event.repositories_added).await?;
            for repo in &event.repositories_removed {
                sqlx::query(
                    "delete from github_installations where repo_path = $1 and installation_id = $2",
                )
                .bind(repo.full_name.to_ascii_lowercase())
                .bind(installation)
                .execute(&state.db)
                .await?;
            }
        }
        "release" => {
            let Some(repo) = &event.repository else {
                return Ok(StatusCode::NO_CONTENT);
            };
            remember(&state, installation, std::slice::from_ref(repo)).await?;
            // Check this repository's apps on the watcher's next pass, which starts now.
            let woken = sqlx::query(
                "update apps set watch_checked_at = null
                 where lower(regexp_replace(repo_url, '(\\.git)?/*$', '')) = 'https://github.com/' || $1",
            )
            .bind(repo.full_name.to_ascii_lowercase())
            .execute(&state.db)
            .await?
            .rows_affected();
            if woken > 0 {
                tracing::info!(repo = %repo.full_name, action, "release webhook");
                state.watch_wakeup.notify_one();
            }
        }
        _ => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_urls_name_their_repository() {
        let base: Url = "https://api.github.com/".parse().unwrap();
        let repo = |u: &str| repo_of_api_url(&base, &u.parse().unwrap());
        assert_eq!(
            repo("https://api.github.com/repos/Octo/Game/releases/latest").as_deref(),
            Some("octo/game")
        );
        assert_eq!(
            repo("https://api.github.com/repos/octo/game?x=1").as_deref(),
            Some("octo/game")
        );
        assert_eq!(repo("https://api.github.com/user"), None);
        assert_eq!(repo("https://example.com/repos/octo/game"), None);
    }

    #[test]
    fn webhook_signatures() {
        let body = br#"{"zen":"hi"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let good = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(signature_ok("secret", body, Some(&good)));
        assert!(!signature_ok("other", body, Some(&good)));
        assert!(!signature_ok("secret", b"{}", Some(&good)));
        assert!(!signature_ok("secret", body, None));
        assert!(!signature_ok("secret", body, Some("sha1=abc")));
    }
}
