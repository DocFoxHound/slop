//! Only a repository's owner can put it on Slop.
//!
//! Every way a repository gets linked (publishing an app, changing or moving its repository,
//! registering a branch from a fork) goes through `check`:
//!
//! - **A personal repository:** the signed-in user's GitHub account must be its owner.
//!   Collaborators, even with admin rights, can't link it.
//! - **An organization's repository:** the user must be one of the organization's owners,
//!   checked through Slop's GitHub App installed on the organization (as for official teams).
//! - **One app per repository**, so nobody can list a copy of someone else's app.
//! - Other hosts (GitLab, Bitbucket, self-hosted) can't be checked yet, so they're refused.
//!
//! Ownership is checked by GitHub's numeric account ID, which survives renames.

use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
};

fn refuse(message: impl Into<String>) -> ApiError {
    ApiError::invalid("repo_url", message)
}

/// The same repository however it's written (case, a trailing slash or `.git`).
pub fn normalized(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Whether another app (not `except`) already uses this repository.
async fn taken(
    state: &AppState,
    repo_url: &str,
    except: Option<Uuid>,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(
        "select title from apps
         where regexp_replace(lower(trim(repo_url)), '(/+|\\.git/*)$', '') = $1
           and ($2::uuid is null or id <> $2)
         limit 1",
    )
    .bind(normalized(repo_url))
    .bind(except)
    .fetch_optional(&state.db)
    .await
}

/// Refuses a repository the user doesn't own, or one another app already uses.
pub async fn check(
    state: &AppState,
    user: Uuid,
    repo_url: &str,
    except: Option<Uuid>,
) -> ApiResult<()> {
    if let Some(title) = taken(state, repo_url, except).await? {
        return Err(ApiError::Conflict(format!(
            "that repository is already on Slop as {title}. To build on it, fork it and register a branch"
        )));
    }
    if !state.config.releases.require_repo_owner {
        return Ok(());
    }
    let Some(path) = slop_provenance::github_repo_path(repo_url) else {
        return Err(refuse(
            "Slop can only check who owns GitHub repositories so far; GitLab, Bitbucket and self-hosted Git come later",
        ));
    };
    let mine: Option<String> = sqlx::query_scalar(
        "select provider_subject from identities where user_id = $1 and provider = 'github' limit 1",
    )
    .bind(user)
    .fetch_optional(&state.db)
    .await?;
    let Some(my_id) = mine else {
        return Err(refuse(
            "sign in with GitHub to link a GitHub repository, so Slop can check it's yours",
        ));
    };
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{path}"))
        .map_err(|e| ApiError::Internal(e.into()))?;
    let res = crate::releases::github_get(state, url)
        .await
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    match res.status().as_u16() {
        200 => {}
        404 => {
            return Err(refuse(
                "Slop can't see that repository. Check the address; if it's private, install Slop's GitHub App on it first",
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
    let login = owner["login"].as_str().unwrap_or("");
    let owner_id = owner["id"].as_i64().map(|i| i.to_string());
    if owner["type"] == "Organization" {
        let token = crate::github_app::org_token(state, login)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| {
                refuse(format!(
                    "{login} is an organization: install Slop's GitHub App on it (with read access to members) so Slop can check you're one of its owners"
                ))
            })?;
        let me = crate::teams::github_get(state, &token, &format!("user/{my_id}"))
            .await
            .map_err(ApiError::Internal)?;
        let my_login = me.as_ref().and_then(|m| m["login"].as_str()).unwrap_or("");
        let owner_of_org = !my_login.is_empty()
            && crate::teams::github_get(
                state,
                &token,
                &format!("orgs/{login}/memberships/{my_login}"),
            )
            .await
            .map_err(ApiError::Internal)?
            .is_some_and(|m| m["role"] == "admin" && m["state"] == "active");
        if !owner_of_org {
            return Err(refuse(format!(
                "only owners of the {login} organization can put its repositories on Slop"
            )));
        }
        return Ok(());
    }
    if owner_id.as_deref() != Some(my_id.as_str()) {
        return Err(refuse(format!(
            "only the repository's owner can put it on Slop, and {login} owns this one. To build on it, fork it and register a branch from your fork"
        )));
    }
    Ok(())
}
