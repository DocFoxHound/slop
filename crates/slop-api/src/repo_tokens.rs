//! Read-only access tokens for private repositories outside GitHub (DESIGN §13, ARCHITECTURE
//! §6.5): GitLab, Bitbucket, and Gitea or Forgejo servers. GitHub uses Slop's GitHub App instead.
//!
//! - The maker pastes a token that can only read the repository. Slop checks it works, then keeps
//!   it encrypted (ChaCha20-Poly1305 with `SLOP_REPO_TOKEN_KEY`, bound to the app) and never shows
//!   it again.
//! - It's only sent to the repository's own host, in the `Authorization` header, which the HTTP
//!   client drops if a redirect leaves that host.
//! - It's tied to the repository: moving the app to another repository forgets it.
//! - Players the app is shared with get the host's short-lived download link, like GitHub's. A
//!   host that serves the file directly instead can't be shared through Slop: Slop never relays
//!   files.

use anyhow::{Context, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/apps/{slug}/repo-token",
        get(show).put(save).delete(forget),
    )
}

/// Which kind of host the token is for: it decides how it's sent and checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Gitlab,
    Bitbucket,
    /// Gitea, Forgejo and Codeberg.
    Gitea,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Gitlab => "gitlab",
            Self::Bitbucket => "bitbucket",
            Self::Gitea => "gitea",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Gitlab, Self::Bitbucket, Self::Gitea]
            .into_iter()
            .find(|k| k.name() == s)
    }

    /// The `Authorization` header value.
    fn header(self, token: &str) -> String {
        match self {
            Self::Gitlab | Self::Bitbucket => format!("Bearer {token}"),
            Self::Gitea => format!("token {token}"),
        }
    }
}

/// A token the maker pasted, before it's saved.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Given {
    pub kind: Kind,
    pub token: String,
}

impl Given {
    pub fn header(&self) -> String {
        self.kind.header(self.token.trim())
    }
}

/// Checks a pasted token reads the repository: on a public host, and through the host's API.
pub async fn verify_given(state: &AppState, repo_url: &str, given: &Given) -> ApiResult<()> {
    if state.config.releases.repo_token_key.is_none() {
        return Err(ApiError::Unavailable(
            "this server can't keep repository tokens yet",
        ));
    }
    if slop_provenance::github_repo_path(repo_url).is_some() {
        return Err(ApiError::invalid(
            "repo_token",
            "GitHub repositories use Slop's GitHub App instead of a token",
        ));
    }
    let token = given.token.trim();
    if token.is_empty() || token.len() > 500 || token.chars().any(char::is_whitespace) {
        return Err(ApiError::invalid(
            "repo_token",
            "paste the token on its own",
        ));
    }
    verify(state, given.kind, repo_url, token)
        .await
        .map_err(|e| ApiError::invalid("repo_token", format!("{e:#}")))
}

/// Saves a checked token for an app's repository (in the caller's transaction).
pub async fn store(
    tx: &mut sqlx::PgConnection,
    key: &[u8; 32],
    app_id: Uuid,
    repo_url: &str,
    given: &Given,
    user: Uuid,
) -> ApiResult<()> {
    let sealed = seal(key, app_id, given.token.trim()).map_err(ApiError::Internal)?;
    sqlx::query(
        "insert into repo_tokens (app_id, repo_url, kind, sealed, created_by) values ($1, $2, $3, $4, $5)
         on conflict (app_id) do update set repo_url = $2, kind = $3, sealed = $4, created_by = $5,
                                            created_at = now(), last_used_at = null",
    )
    .bind(app_id)
    .bind(repo_url)
    .bind(given.kind.name())
    .bind(&sealed)
    .bind(user)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The README and slop-verify.txt at the top of the default branch, read through the host's API
/// with the token (repo_owner.rs's code method, for private repositories).
pub async fn read_proof(
    state: &AppState,
    repo_url: &str,
    given: &Given,
) -> anyhow::Result<Option<String>> {
    let repo = Url::parse(repo_url)?;
    crate::repo_owner::public_host(state, &repo).await?;
    let path =
        repo_path(&repo).ok_or_else(|| anyhow!("the repository URL has no owner and name"))?;
    let auth = given.header();
    let get = |url: String| {
        let auth = auth.clone();
        async move {
            let res = state
                .http
                .get(&url)
                .header("authorization", auth)
                .header("user-agent", "slop-api")
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?;
            anyhow::ensure!(
                res.status().is_success(),
                "the host answered HTTP {}",
                res.status().as_u16()
            );
            let bytes = res.bytes().await?;
            anyhow::Ok(bytes[..bytes.len().min(1024 * 1024)].to_vec())
        }
    };
    let json = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default();
    let enc = |s: &str| percent_encoding::utf8_percent_encode(s, PATH_PART).to_string();
    let base = origin(&repo);
    // (file names at the top, URL of a file by name)
    let (names, file_url): (Vec<String>, FileUrl) =
        match given.kind {
            Kind::Gitlab => {
                let project = format!("{base}/api/v4/projects/{}", enc(&path));
                let info = json(get(project.clone()).await?);
                let branch = info["default_branch"]
                    .as_str()
                    .unwrap_or("main")
                    .to_string();
                let tree = json(
                    get(format!(
                        "{project}/repository/tree?per_page=100&ref={}",
                        enc(&branch)
                    ))
                    .await?,
                );
                let names = tree
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["type"] == "blob")
                    .filter_map(|e| e["name"].as_str().map(String::from))
                    .collect();
                (
                    names,
                    Box::new(move |n: &str| {
                        format!(
                            "{project}/repository/files/{}/raw?ref={}",
                            enc(n),
                            enc(&branch)
                        )
                    }),
                )
            }
            Kind::Bitbucket => {
                let api = if repo.host_str() == Some("bitbucket.org") {
                    "https://api.bitbucket.org".to_string()
                } else {
                    format!("{base}/bitbucket-api")
                };
                let root = format!("{api}/2.0/repositories/{path}");
                let info = json(get(root.clone()).await?);
                let branch = info["mainbranch"]["name"]
                    .as_str()
                    .unwrap_or("main")
                    .to_string();
                let list = json(get(format!("{root}/src/{}/?pagelen=100", enc(&branch))).await?);
                let names = list["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["type"] == "commit_file")
                    .filter_map(|e| e["path"].as_str().map(String::from))
                    .collect();
                (
                    names,
                    Box::new(move |n: &str| format!("{root}/src/{}/{}", enc(&branch), enc(n))),
                )
            }
            Kind::Gitea => {
                let root = format!("{base}/api/v1/repos/{path}");
                let list = json(get(format!("{root}/contents")).await?);
                let names = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["type"] == "file")
                    .filter_map(|e| e["name"].as_str().map(String::from))
                    .collect();
                (
                    names,
                    Box::new(move |n: &str| format!("{root}/raw/{}", enc(n))),
                )
            }
        };
    let mut text = String::new();
    let mut any = false;
    for name in names.iter().filter(|n| crate::repo_owner::is_proof_name(n)) {
        let body = get(file_url(name)).await?;
        text.push_str(&String::from_utf8_lossy(&body));
        text.push('\n');
        any = true;
    }
    Ok(any.then_some(text))
}

/// A file's API address, by its name at the top of the repository.
type FileUrl = Box<dyn Fn(&str) -> String + Send + Sync>;

/// One path segment or query value: everything but letters, digits and `-._~` is escaped.
const PATH_PART: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn cipher(key: &[u8; 32]) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(key.into())
}

/// Encrypts a token for one app: nonce (12 bytes) then ciphertext, with the app's ID as
/// associated data so a row copied to another app doesn't decrypt.
fn seal(key: &[u8; 32], app_id: Uuid, token: &str) -> anyhow::Result<Vec<u8>> {
    let nonce: [u8; 12] = rand::random();
    let sealed = cipher(key)
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: token.as_bytes(),
                aad: app_id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("couldn't encrypt the token"))?;
    Ok([nonce.as_slice(), &sealed].concat())
}

fn open(key: &[u8; 32], app_id: Uuid, sealed: &[u8]) -> anyhow::Result<String> {
    if sealed.len() < 12 {
        bail!("stored token is damaged");
    }
    let (nonce, body) = sealed.split_at(12);
    let nonce: [u8; 12] = nonce.try_into()?;
    let plain = cipher(key)
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: body,
                aad: app_id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("stored token doesn't decrypt (was SLOP_REPO_TOKEN_KEY changed?)"))?;
    String::from_utf8(plain).context("stored token isn't text")
}

/// `(owner path, repo name)` from a repository URL: `https://host/group/sub/repo`.
fn repo_path(repo_url: &Url) -> Option<String> {
    let path = repo_url.path().trim_matches('/').trim_end_matches(".git");
    (path.contains('/') && !path.is_empty()).then(|| path.to_string())
}

fn origin(repo: &Url) -> String {
    repo.origin().ascii_serialization()
}

/// Checks the token can read the repository, through the host's API.
async fn verify(state: &AppState, kind: Kind, repo_url: &str, token: &str) -> anyhow::Result<()> {
    let repo = Url::parse(repo_url).context("the repository URL isn't valid")?;
    crate::repo_owner::public_host(state, &repo).await?;
    let path =
        repo_path(&repo).ok_or_else(|| anyhow!("the repository URL has no owner and name"))?;
    let api = match kind {
        Kind::Gitlab => format!(
            "{}/api/v4/projects/{}",
            origin(&repo),
            percent_encoding::utf8_percent_encode(&path, percent_encoding::NON_ALPHANUMERIC)
        ),
        Kind::Bitbucket => {
            if repo.host_str() != Some("bitbucket.org")
                && !state.config.releases.allow_http_downloads
            {
                bail!("Bitbucket tokens are for bitbucket.org repositories");
            }
            let base = if repo.host_str() == Some("bitbucket.org") {
                "https://api.bitbucket.org".to_string()
            } else {
                // Tests: the fake host stands in for api.bitbucket.org.
                format!("{}/bitbucket-api", origin(&repo))
            };
            format!("{base}/2.0/repositories/{path}")
        }
        Kind::Gitea => format!("{}/api/v1/repos/{path}", origin(&repo)),
    };
    let res = state
        .http
        .get(&api)
        .header("authorization", kind.header(token))
        .header("accept", "application/json")
        .header("user-agent", "slop-api")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("couldn't reach the repository's host")?;
    match res.status().as_u16() {
        200 => Ok(()),
        401 | 403 => bail!("the host refused the token: check it can read this repository"),
        404 => bail!("the host can't find the repository with this token"),
        code => bail!("the host answered HTTP {code}"),
    }
}

#[derive(sqlx::FromRow)]
struct Stored {
    kind: String,
    sealed: Vec<u8>,
}

/// The `Authorization` header for a request to `url` on the app's repository host, if the maker
/// gave a token. Only for URLs on the same host (and port) as the app's current repository.
pub async fn auth_for(state: &AppState, app_id: Uuid, url: &str) -> anyhow::Result<Option<String>> {
    let Some(key) = state.config.releases.repo_token_key.as_ref() else {
        return Ok(None);
    };
    let row: Option<(String, Vec<u8>, String)> = sqlx::query_as(
        "select t.kind, t.sealed, a.repo_url from repo_tokens t join apps a on a.id = t.app_id
         where t.app_id = $1 and t.repo_url = a.repo_url",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((kind, sealed, repo_url)) = row else {
        return Ok(None);
    };
    let stored = Stored { kind, sealed };
    let (Ok(target), Ok(repo)) = (Url::parse(url), Url::parse(&repo_url)) else {
        return Ok(None);
    };
    if target.host_str() != repo.host_str()
        || target.port_or_known_default() != repo.port_or_known_default()
        || target.scheme() != repo.scheme()
    {
        return Ok(None);
    }
    let kind = Kind::parse(&stored.kind).ok_or_else(|| anyhow!("unknown token kind"))?;
    crate::repo_owner::public_host(state, &target).await?;
    let token = open(key, app_id, &stored.sealed)?;
    sqlx::query("update repo_tokens set last_used_at = now() where app_id = $1")
        .bind(app_id)
        .execute(&state.db)
        .await?;
    Ok(Some(kind.header(&token)))
}

/// Whether the token still reads the repository (frozen.rs), when there is one.
pub async fn still_works(state: &AppState, app_id: Uuid) -> Option<bool> {
    let key = state.config.releases.repo_token_key.as_ref()?;
    let (kind, sealed, repo_url): (String, Vec<u8>, String) = sqlx::query_as(
        "select t.kind, t.sealed, a.repo_url from repo_tokens t join apps a on a.id = t.app_id
         where t.app_id = $1 and t.repo_url = a.repo_url",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await
    .ok()??;
    let token = open(key, app_id, &sealed).ok()?;
    match verify(state, Kind::parse(&kind)?, &repo_url, &token).await {
        Ok(()) => Some(true),
        Err(err) => {
            tracing::info!(app = %app_id, error = %err, "repository token check");
            Some(false)
        }
    }
}

/// A short-lived download link for a private file: the host's redirect to its file storage,
/// asked for with the token and not followed. Hosts that answer with the file itself can't give
/// players a link, and Slop doesn't relay files.
pub async fn short_lived_link(state: &AppState, url: &str, auth: &str) -> anyhow::Result<String> {
    let res = state
        .http
        .get(url)
        .header("authorization", auth)
        .header("user-agent", "slop-api")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("couldn't reach the repository's host")?;
    let from = Url::parse(url)?;
    if res.status().is_redirection()
        && let Some(location) = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        && let Ok(to) = from.join(location)
        && to.host_str() != from.host_str()
        && (to.scheme() == "https" || state.config.releases.allow_http_downloads)
    {
        return Ok(to.into());
    }
    bail!(
        "this repository's host serves private files directly instead of through short-lived links, \
         so Slop can't pass them on (it never relays files): the people it's shared with need their own access"
    )
}

// ---------------------------------------------------------------------------------------------
// The maker's settings (Manage page)

#[derive(Serialize)]
struct Status {
    /// Whether a token is saved for the current repository.
    saved: bool,
    kind: Option<Kind>,
    #[serde(with = "time::serde::rfc3339::option")]
    saved_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_used_at: Option<OffsetDateTime>,
    /// Off on this server (no SLOP_REPO_TOKEN_KEY).
    available: bool,
    /// GitHub repositories use Slop's GitHub App instead.
    github: bool,
}

async fn owned(state: &AppState, user: &CurrentUser, slug: &str) -> ApiResult<(Uuid, String)> {
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("select id, maker_id, repo_url from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some((id, maker, repo)) if maker == user.id => Ok((id, repo)),
        _ => Err(ApiError::NotFound),
    }
}

async fn status(state: &AppState, app_id: Uuid, repo_url: &str) -> ApiResult<Status> {
    let row: Option<(String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "select kind, created_at, last_used_at from repo_tokens where app_id = $1 and repo_url = $2",
    )
    .bind(app_id)
    .bind(repo_url)
    .fetch_optional(&state.db)
    .await?;
    Ok(Status {
        saved: row.is_some(),
        kind: row.as_ref().and_then(|r| Kind::parse(&r.0)),
        saved_at: row.as_ref().map(|r| r.1),
        last_used_at: row.and_then(|r| r.2),
        available: state.config.releases.repo_token_key.is_some(),
        github: slop_provenance::github_repo_path(repo_url).is_some(),
    })
}

async fn show(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Status>> {
    let (app_id, repo) = owned(&state, &user, &slug).await?;
    Ok(Json(status(&state, app_id, &repo).await?))
}

async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<Given>,
) -> ApiResult<Json<Status>> {
    let (app_id, repo) = owned(&state, &user, &slug).await?;
    verify_given(&state, &repo, &body).await?;
    let key = state
        .config
        .releases
        .repo_token_key
        .ok_or(ApiError::Unavailable(
            "this server can't keep repository tokens yet",
        ))?;
    let mut tx = state.db.begin().await?;
    store(&mut tx, &key, app_id, &repo, &body, user.id).await?;
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "repo_token.save",
        &slug,
        serde_json::json!({ "kind": body.kind.name() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(status(&state, app_id, &repo).await?))
}

async fn forget(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = owned(&state, &user, &slug).await?;
    sqlx::query("delete from repo_tokens where app_id = $1")
        .bind(app_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_only_open_for_their_app() {
        let key = [3u8; 32];
        let app = Uuid::now_v7();
        let sealed = seal(&key, app, "glpat-secret").unwrap();
        assert_eq!(open(&key, app, &sealed).unwrap(), "glpat-secret");
        assert!(open(&key, Uuid::now_v7(), &sealed).is_err());
        assert!(open(&[4u8; 32], app, &sealed).is_err());
        assert_ne!(
            seal(&key, app, "glpat-secret").unwrap(),
            sealed,
            "fresh nonce each time"
        );
    }

    #[test]
    fn headers_by_host() {
        assert_eq!(Kind::Gitlab.header("t"), "Bearer t");
        assert_eq!(Kind::Gitea.header("t"), "token t");
        assert_eq!(
            repo_path(&Url::parse("https://gitlab.com/group/sub/repo.git").unwrap()).as_deref(),
            Some("group/sub/repo")
        );
        assert_eq!(repo_path(&Url::parse("https://host/repo").unwrap()), None);
    }
}
