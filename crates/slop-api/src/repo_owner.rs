// SPDX-License-Identifier: AGPL-3.0-or-later
//! Only a repository's owner can put it on Slop (DESIGN §8).
//!
//! Every way a repository gets linked (publishing an app, changing or moving its repository,
//! registering a branch from a fork) goes through `check`. How ownership is proved depends on
//! the host:
//!
//! - **GitHub:** a GitHub account linked to the Slop account (Account → Sign-ins) must own the
//!   repository, or be an owner of the organization that does (checked through Slop's GitHub
//!   App installed on the organization). Collaborators can't, even with admin rights.
//! - **GitLab.com:** a linked GitLab account must own the project, or be an Owner of its group.
//! - **Bitbucket Cloud:** a linked Bitbucket account must own the repository. A workspace's
//!   repository is proved with a code, as below.
//! - **Anywhere else** (self-hosted GitLab, Gitea, Forgejo, cgit…), and groups or workspaces Slop
//!   can't ask about: a code. Slop gives the user a code to put in the repository's README (or a
//!   `slop-verify.txt` file at its root) on the default branch, then reads it. That proves the
//!   user can push to the repository, which is the most any host without an owner check allows.
//!
//! A **private** repository outside GitHub is checked the same way through the host's API, with
//! the read-only token the maker gives for it (repo_tokens.rs): GitLab.com and Bitbucket owners
//! by their linked accounts, anything else by the code, read through the API.
//!
//! One app per repository, whatever the host. Accounts are matched by the provider's numeric ID
//! (GitHub, GitLab) or UUID (Bitbucket), which survive renames.

use std::{net::IpAddr, process::Stdio, time::Duration};

use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

/// A verified code proves ownership for this long (enough to finish publishing).
const CLAIM_VALID_DAYS: i32 = 30;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/repos/ownership", post(ownership))
        .route("/repos/ownership/check", post(check_code))
}

/// The same repository however it's written (case, a trailing slash or `.git`).
pub fn normalized(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Whether another app (not `except`) already uses this repository: how to say so. Only an app
/// in the store is named; a private, draft or delisted one isn't revealed (security review,
/// 2026-10-22).
async fn taken(
    state: &AppState,
    repo_url: &str,
    except: Option<Uuid>,
) -> sqlx::Result<Option<String>> {
    let found: Option<Option<String>> = sqlx::query_scalar(
        "select case when status = 'listed' and visibility = 'public' then title end from apps
         where regexp_replace(lower(trim(repo_url)), '(/+|\\.git/*)$', '') = $1
           and ($2::uuid is null or id <> $2)
         limit 1",
    )
    .bind(normalized(repo_url))
    .bind(except)
    .fetch_optional(&state.db)
    .await?;
    Ok(found.map(|title| match title {
        Some(title) => format!("that repository is already on Slop as {title}"),
        None => "that repository is already linked to an app on Slop".to_string(),
    }))
}

/// How ownership of a repository is (or would be) proved, and whether it is.
#[derive(Serialize, Debug, Clone)]
pub struct Assessment {
    pub ok: bool,
    /// account (a linked sign-in) or code (a code in the README).
    pub method: &'static str,
    /// What to do, or why it's refused.
    pub message: String,
    /// The sign-in to link to prove it (github, gitlab, bitbucket).
    pub link: Option<&'static str>,
    /// For the code method: the code to add.
    pub code: Option<String>,
}

impl Assessment {
    fn yes(method: &'static str) -> Self {
        Self {
            ok: true,
            method,
            message: "You own this repository.".into(),
            link: None,
            code: None,
        }
    }

    fn no(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            method: "account",
            message: message.into(),
            link: None,
            code: None,
        }
    }

    fn link(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            link: Some(provider),
            ..Self::no(message)
        }
    }
}

/// Refuses a repository the user doesn't own, or one another app already uses.
pub async fn check(
    state: &AppState,
    user: Uuid,
    repo_url: &str,
    except: Option<Uuid>,
) -> ApiResult<()> {
    check_with(state, user, repo_url, except, None).await
}

/// `check`, reading a private repository with the maker's token.
pub async fn check_with(
    state: &AppState,
    user: Uuid,
    repo_url: &str,
    except: Option<Uuid>,
    token: Option<&crate::repo_tokens::Given>,
) -> ApiResult<()> {
    if let Some(said) = taken(state, repo_url, except).await? {
        return Err(ApiError::Conflict(format!(
            "{said}. To build on it, fork it and register a branch"
        )));
    }
    if !state.config.releases.require_repo_owner {
        return Ok(());
    }
    let a = assess_with(state, user, repo_url, token).await?;
    if a.ok {
        Ok(())
    } else {
        Err(ApiError::invalid("repo_url", a.message))
    }
}

/// The user's linked accounts at a provider: (ID, login).
async fn accounts(
    state: &AppState,
    user: Uuid,
    provider: &str,
) -> sqlx::Result<Vec<(String, Option<String>)>> {
    sqlx::query_as(
        "select provider_subject, login from identities where user_id = $1 and provider = $2 order by created_at",
    )
    .bind(user)
    .bind(provider)
    .fetch_all(&state.db)
    .await
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("").to_ascii_lowercase()
}

/// The repository's path on its host: "owner/repo" or "group/sub/project".
fn repo_path(url: &Url) -> String {
    url.path()
        .trim_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

pub async fn assess(state: &AppState, user: Uuid, repo_url: &str) -> ApiResult<Assessment> {
    assess_with(state, user, repo_url, None).await
}

pub async fn assess_with(
    state: &AppState,
    user: Uuid,
    repo_url: &str,
    token: Option<&crate::repo_tokens::Given>,
) -> ApiResult<Assessment> {
    let url = Url::parse(repo_url).map_err(|_| ApiError::invalid("repo_url", "not an address"))?;
    let host = host_of(&url);
    if slop_provenance::github_repo_path(repo_url).is_some() {
        return github(state, user, repo_url).await;
    }
    let auth = token.map(|t| t.header());
    let gitlab_host = state
        .config
        .gitlab
        .as_ref()
        .map(|g| host_of(&g.authorize_url))
        .unwrap_or_else(|| "gitlab.com".into());
    if host == gitlab_host {
        if let Some(a) = gitlab(state, user, &url, auth.as_deref()).await? {
            return Ok(a);
        }
    } else if host == "bitbucket.org"
        && let Some(a) = bitbucket(state, user, &url, auth.as_deref()).await?
    {
        return Ok(a);
    }
    code_assessment(state, user, repo_url).await
}

async fn github(state: &AppState, user: Uuid, repo_url: &str) -> ApiResult<Assessment> {
    let path = slop_provenance::github_repo_path(repo_url).unwrap_or_default();
    let mine = accounts(state, user, "github").await?;
    let api = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{path}"))
        .map_err(|e| ApiError::Internal(e.into()))?;
    let res = crate::releases::github_get(state, api)
        .await
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    match res.status().as_u16() {
        200 => {}
        404 => {
            return Ok(Assessment::no(
                if state.config.releases.github_app.is_some() {
                    "Slop can't see that repository. Check the address; if it's private, install Slop's GitHub App on it first"
                } else {
                    "Slop can't see that repository. Check the address; private GitHub repositories need Slop's GitHub App, which this server hasn't set up yet, so use a public repository for now"
                },
            ));
        }
        code => {
            return Err(ApiError::Unavailable(if code == 403 || code == 429 {
                "GitHub is limiting Slop's requests right now; try again in a few minutes"
            } else {
                "GitHub didn't answer; try again in a few minutes"
            }));
        }
    }
    let repo: serde_json::Value = res.json().await.map_err(|e| ApiError::Internal(e.into()))?;
    let owner = &repo["owner"];
    let login = owner["login"].as_str().unwrap_or("").to_string();
    if mine.is_empty() {
        return Ok(Assessment::link(
            "github",
            format!(
                "link the GitHub account that owns this repository ({login}) to your Slop account, under Account → Sign-ins"
            ),
        ));
    }
    if owner["type"] == "Organization" {
        let token = crate::github_app::org_token(state, &login)
            .await
            .map_err(ApiError::Internal)?;
        let Some(token) = token else {
            return Ok(Assessment::no(format!(
                "{login} is an organization: install Slop's GitHub App on it (with read access to members) so Slop can check you're one of its owners"
            )));
        };
        for (id, _) in &mine {
            let me = crate::teams::github_get(state, &token, &format!("user/{id}"))
                .await
                .map_err(ApiError::Internal)?;
            let Some(my_login) = me.as_ref().and_then(|m| m["login"].as_str()) else {
                continue;
            };
            let owner_of_org = crate::teams::github_get(
                state,
                &token,
                &format!("orgs/{login}/memberships/{my_login}"),
            )
            .await
            .map_err(ApiError::Internal)?
            .is_some_and(|m| m["role"] == "admin" && m["state"] == "active");
            if owner_of_org {
                return Ok(Assessment::yes("account"));
            }
        }
        return Ok(Assessment::link(
            "github",
            format!(
                "only owners of the {login} organization can put its repositories on Slop. If you are one, link that GitHub account under Account → Sign-ins"
            ),
        ));
    }
    let owner_id = owner["id"].as_i64().map(|i| i.to_string());
    if mine.iter().any(|(id, _)| Some(id) == owner_id.as_ref()) {
        return Ok(Assessment::yes("account"));
    }
    Ok(Assessment::link(
        "github",
        format!(
            "only the repository's owner can put it on Slop, and {login} owns this one. If that's you, link that GitHub account under Account → Sign-ins; otherwise fork it and register a branch from your fork"
        ),
    ))
}

/// GitLab.com: a personal project's owner, or an Owner of the group. `None` when GitLab can't
/// say (then a code proves it).
async fn gitlab(
    state: &AppState,
    user: Uuid,
    url: &Url,
    auth: Option<&str>,
) -> ApiResult<Option<Assessment>> {
    let api = state
        .config
        .gitlab
        .as_ref()
        .map(|g| g.api_base.clone())
        .unwrap_or_else(|| "https://gitlab.com/api/v4/".parse().expect("valid"));
    let path = repo_path(url);
    let encoded: String = url::form_urlencoded::byte_serialize(path.as_bytes()).collect();
    let project = get_json(state, &api, &format!("projects/{encoded}"), auth).await?;
    let Some(project) = project else {
        return Ok(Some(Assessment::no(if auth.is_some() {
            "GitLab can't find that project with this token"
        } else {
            "GitLab says there's no public project at that address. If it's private, add a read-only token for it"
        })));
    };
    let mine = accounts(state, user, "gitlab").await?;
    let namespace = project["namespace"]["full_path"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if mine.is_empty() {
        // Without GitLab sign-in on this server, a code proves it instead.
        return Ok(state.config.gitlab.is_some().then(|| {
            Assessment::link(
                "gitlab",
                format!("link the GitLab account that owns {namespace} to your Slop account, under Account → Sign-ins"),
            )
        }));
    }
    if project["namespace"]["kind"] == "user" {
        let owner = project["owner"]["id"].as_i64().map(|i| i.to_string());
        return Ok(Some(
            if mine.iter().any(|(id, _)| Some(id) == owner.as_ref()) {
                Assessment::yes("account")
            } else {
                Assessment::link(
                    "gitlab",
                    format!(
                        "only the project's owner can put it on Slop, and {namespace} owns this one"
                    ),
                )
            },
        ));
    }
    // A group's project: an Owner of it (access level 50), when GitLab shows its members.
    let id = project["id"].as_i64().unwrap_or(0);
    for (account, _) in &mine {
        let member = get_json(
            state,
            &api,
            &format!("projects/{id}/members/all/{account}"),
            auth,
        )
        .await?;
        if member.is_some_and(|m| m["access_level"].as_i64() == Some(50)) {
            return Ok(Some(Assessment::yes("account")));
        }
    }
    Ok(None)
}

/// Bitbucket Cloud: a personal repository's owner. `None` for a workspace's (a code proves it).
async fn bitbucket(
    state: &AppState,
    user: Uuid,
    url: &Url,
    auth: Option<&str>,
) -> ApiResult<Option<Assessment>> {
    let api = state
        .config
        .bitbucket
        .as_ref()
        .map(|b| b.api_base.clone())
        .unwrap_or_else(|| "https://api.bitbucket.org/2.0/".parse().expect("valid"));
    let path = repo_path(url);
    let repo = get_json(state, &api, &format!("repositories/{path}"), auth).await?;
    let Some(repo) = repo else {
        return Ok(Some(Assessment::no(if auth.is_some() {
            "Bitbucket can't find that repository with this token"
        } else {
            "Bitbucket says there's no public repository at that address. If it's private, add a read-only token for it"
        })));
    };
    if repo["owner"]["type"] != "user" {
        return Ok(None);
    }
    let mine = accounts(state, user, "bitbucket").await?;
    let owner = repo["owner"]["uuid"].as_str().unwrap_or("").to_string();
    let name = repo["owner"]["display_name"]
        .as_str()
        .unwrap_or("its owner")
        .to_string();
    if mine.iter().any(|(id, _)| *id == owner) {
        return Ok(Some(Assessment::yes("account")));
    }
    if state.config.bitbucket.is_none() {
        return Ok(None);
    }
    Ok(Some(Assessment::link(
        "bitbucket",
        format!(
            "only the repository's owner can put it on Slop, and {name} owns this one. If that's you, link that Bitbucket account under Account → Sign-ins"
        ),
    )))
}

async fn get_json(
    state: &AppState,
    base: &Url,
    path: &str,
    auth: Option<&str>,
) -> ApiResult<Option<serde_json::Value>> {
    let url = base.join(path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut request = state.http.get(url);
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    let res = request
        .header("accept", "application/json")
        .header("user-agent", "slop-api")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| {
            ApiError::Unavailable("the repository's host didn't answer; try again shortly")
        })?;
    if !res.status().is_success() {
        return Ok(None);
    }
    Ok(res.json().await.ok())
}

// ---------------------------------------------------------------------------------------------
// The code method

fn new_code() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let bytes = rand::random::<[u8; 16]>();
    let s: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("slop-verify-{s}")
}

async fn code_assessment(state: &AppState, user: Uuid, repo_url: &str) -> ApiResult<Assessment> {
    let key = normalized(repo_url);
    let (code, verified): (String, bool) = sqlx::query_as(
        "insert into repo_claims (user_id, repo_key, code) values ($1, $2, $3)
         on conflict (user_id, repo_key) do update set code = repo_claims.code
         returning code, coalesce(verified_at > now() - make_interval(days => $4), false)",
    )
    .bind(user)
    .bind(&key)
    .bind(new_code())
    .bind(CLAIM_VALID_DAYS)
    .fetch_one(&state.db)
    .await?;
    Ok(Assessment {
        ok: verified,
        method: "code",
        message: if verified {
            "You proved you can change this repository.".into()
        } else {
            format!(
                "Slop can't ask this host who owns the repository, so prove it: add {code} to the README (or a file named slop-verify.txt at the top of the repository) on its default branch, then press Check. You can remove it once the app is on Slop."
            )
        },
        link: None,
        code: Some(code),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoBody {
    repo_url: String,
    /// A private repository outside GitHub: its read-only token (repo_tokens.rs).
    #[serde(default)]
    repo_token: Option<crate::repo_tokens::Given>,
}

/// How to prove you own a repository, and whether you have: the Publish form asks before
/// publishing, so it can show what to do.
async fn ownership(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<RepoBody>,
) -> ApiResult<Json<Assessment>> {
    let repo_url = crate::catalog::validate_repo_url_in(&state, &b.repo_url)?;
    if let Some(said) = taken(&state, &repo_url, None).await? {
        return Ok(Json(Assessment::no(format!(
            "{said}. To build on it, fork it and register a branch"
        ))));
    }
    if !state.config.releases.require_repo_owner {
        return Ok(Json(Assessment::yes("account")));
    }
    if let Some(t) = &b.repo_token {
        crate::repo_tokens::verify_given(&state, &repo_url, t).await?;
    }
    Ok(Json(
        assess_with(&state, user.id, &repo_url, b.repo_token.as_ref()).await?,
    ))
}

/// Reads the repository for the user's code.
async fn check_code(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<RepoBody>,
) -> ApiResult<Json<Assessment>> {
    let repo_url = crate::catalog::validate_repo_url_in(&state, &b.repo_url)?;
    let key = normalized(&repo_url);
    let code: Option<String> =
        sqlx::query_scalar("select code from repo_claims where user_id = $1 and repo_key = $2")
            .bind(user.id)
            .bind(&key)
            .fetch_optional(&state.db)
            .await?;
    let code = code.ok_or_else(|| ApiError::Conflict("ask for a code first".into()))?;
    let read = match &b.repo_token {
        Some(t) => {
            crate::repo_tokens::verify_given(&state, &repo_url, t).await?;
            crate::repo_tokens::read_proof(&state, &repo_url, t).await
        }
        None => read_proof(&state, &repo_url).await,
    };
    let found = match read {
        Ok(text) => text.is_some_and(|t| t.contains(&code)),
        Err(e) => {
            tracing::info!(error = %e, "repository proof");
            return Err(ApiError::Conflict(format!(
                "Slop couldn't read the repository: {e}. It needs to be public and reachable over https"
            )));
        }
    };
    if found {
        sqlx::query(
            "update repo_claims set verified_at = now() where user_id = $1 and repo_key = $2",
        )
        .bind(user.id)
        .bind(&key)
        .execute(&state.db)
        .await?;
    }
    let mut a = code_assessment(&state, user.id, &repo_url).await?;
    if !found {
        a.message = format!(
            "Slop didn't find {code} in the README or slop-verify.txt on the default branch yet. Push it, wait a moment, and check again."
        );
    }
    Ok(Json(a))
}

fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn is_proof_file(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l == "slop-verify.txt" || l == "readme" || l.starts_with("readme.")
}

async fn run(mut cmd: tokio::process::Command) -> anyhow::Result<std::process::Output> {
    tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("the repository took too long to answer"))?
        .map_err(Into::into)
}

/// Refuses hosts on private networks (Slop mustn't be used to reach its own neighbours).
pub async fn public_host(state: &AppState, url: &Url) -> anyhow::Result<()> {
    if state.config.releases.allow_http_downloads {
        return Ok(());
    }
    anyhow::ensure!(url.scheme() == "https", "only https addresses");
    let host = url.host_str().unwrap_or("");
    let port = url.port_or_known_default().unwrap_or(443);
    for addr in tokio::net::lookup_host((host, port)).await? {
        anyhow::ensure!(
            !private_ip(addr.ip()),
            "that address is on a private network"
        );
    }
    Ok(())
}

pub(crate) fn is_proof_name(name: &str) -> bool {
    is_proof_file(name)
}

/// The README and slop-verify.txt at the top of the repository's default branch, fetched with
/// Git itself so it works on any host. Only the file list and those files are downloaded where
/// the host allows (a partial, shallow clone).
pub async fn read_proof(state: &AppState, repo_url: &str) -> anyhow::Result<Option<String>> {
    let url = Url::parse(repo_url)?;
    // Tests serve repositories over plain http on this machine.
    let test_setup = state.config.releases.allow_http_downloads;
    anyhow::ensure!(
        url.scheme() == "https" || (test_setup && url.scheme() == "http"),
        "only https addresses"
    );
    if !test_setup {
        let host = url.host_str().unwrap_or("");
        let port = url.port_or_known_default().unwrap_or(443);
        for addr in tokio::net::lookup_host((host, port)).await? {
            anyhow::ensure!(
                !private_ip(addr.ip()),
                "that address is on a private network"
            );
        }
    }
    let dir = std::env::temp_dir().join(format!("slop-proof-{}", Uuid::now_v7()));
    let target = dir.to_string_lossy().to_string();
    let http = if test_setup { "always" } else { "never" };
    let git = |args: &[&str], cwd: &std::path::Path| {
        let mut cmd = tokio::process::Command::new("git");
        cmd.args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
        ])
        .args(["-c", &format!("protocol.http.allow={http}")])
        .args(["-c", "credential.helper=", "-c", "core.askPass="])
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
        cmd
    };
    let result = async {
        let tmp = std::env::temp_dir();
        let shallow = git(
            &[
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--filter=blob:none",
                "--no-checkout",
                repo_url,
                &target,
            ],
            &tmp,
        );
        if !run(shallow).await?.status.success() {
            // Hosts that can't do partial clones (plain "dumb" HTTP): a normal one.
            let _ = tokio::fs::remove_dir_all(&dir).await;
            let full = git(
                &["clone", "--quiet", "--no-checkout", repo_url, &target],
                &tmp,
            );
            anyhow::ensure!(
                run(full).await?.status.success(),
                "it isn't a public Git repository Slop can read"
            );
        }
        let names = run(git(&["ls-tree", "--name-only", "HEAD"], &dir)).await?;
        let names = String::from_utf8_lossy(&names.stdout).to_string();
        let mut text = String::new();
        let mut any = false;
        for name in names.lines().filter(|n| is_proof_file(n)) {
            let out = run(git(&["show", &format!("HEAD:{name}")], &dir)).await?;
            text.push_str(&String::from_utf8_lossy(
                &out.stdout[..out.stdout.len().min(1024 * 1024)],
            ));
            text.push('\n');
            any = true;
        }
        anyhow::Ok(any.then_some(text))
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_normalize() {
        assert_eq!(
            normalized("https://GitHub.com/Octo/Game.git/"),
            "https://github.com/octo/game"
        );
        assert_eq!(
            normalized("https://git.example/a/b/"),
            "https://git.example/a/b"
        );
    }

    #[test]
    fn proof_files() {
        for ok in ["README.md", "readme", "Readme.rst", "slop-verify.txt"] {
            assert!(is_proof_file(ok), "{ok}");
        }
        assert!(!is_proof_file("src"));
        assert!(!is_proof_file("docs/README.md"));
    }

    #[test]
    fn codes_look_right() {
        let c = new_code();
        assert!(c.starts_with("slop-verify-") && c.len() == 28, "{c}");
        assert_ne!(c, new_code());
    }
}
