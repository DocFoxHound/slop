// SPDX-License-Identifier: AGPL-3.0-or-later
//! Which GitHub repository an app really is (SECURITY §5.1, built 2026-10-22).
//!
//! An app is linked to a repository by its URL, but a URL is a name: if the maker renames their
//! account or deletes the repository, someone else can register the same `owner/repo` and push
//! releases from it ("repojacking"). GitHub's numeric repository and owner IDs never move to a new
//! repository, so SLAPorSLOP records them when the app is linked (or the repository is moved, or by the
//! background fill below for apps linked before) and from then on accepts releases only when:
//! - Slop Build's token names the same repository ID (slop_build.rs), and
//! - every file's build attestation names it (releases.rs `find_attestation`, through
//!   slop-provenance `Expected::repo_id`).

use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;

#[derive(Deserialize)]
struct Repo {
    id: i64,
    owner: Owner,
}

#[derive(Deserialize)]
struct Owner {
    id: i64,
}

/// GitHub's (repository ID, owner ID) for `owner/repo`, as GitHub sees that name now.
pub async fn lookup(state: &AppState, repo_path: &str) -> Option<(i64, i64)> {
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{repo_path}"))
        .ok()?;
    let res = crate::releases::github_get(state, url)
        .await
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let repo: Repo = res.json().await.ok()?;
    Some((repo.id, repo.owner.id))
}

/// The recorded IDs for an app, recording them now if the app has none yet.
pub async fn ensure(state: &AppState, app_id: Uuid) -> sqlx::Result<Option<(i64, i64)>> {
    let row: Option<(String, Option<i64>, Option<i64>)> =
        sqlx::query_as("select repo_url, github_repo_id, github_owner_id from apps where id = $1")
            .bind(app_id)
            .fetch_optional(&state.db)
            .await?;
    let Some((repo_url, repo_id, owner_id)) = row else {
        return Ok(None);
    };
    if let (Some(r), Some(o)) = (repo_id, owner_id) {
        return Ok(Some((r, o)));
    }
    let Some(path) = slop_provenance::github_repo_path(&repo_url) else {
        return Ok(None);
    };
    let Some((r, o)) = lookup(state, &path).await else {
        return Ok(None);
    };
    // Only fill an empty record, and only for the URL it was looked up for.
    sqlx::query(
        "update apps set github_repo_id = $2, github_owner_id = $3
         where id = $1 and github_repo_id is null and repo_url = $4",
    )
    .bind(app_id)
    .bind(r)
    .bind(o)
    .bind(&repo_url)
    .execute(&state.db)
    .await?;
    Ok(Some((r, o)))
}

/// The repository IDs for a repository URL, from the first app linked to it (recorded now if
/// that app has none yet).
pub async fn recorded_for_url(
    state: &AppState,
    repo_url: &str,
) -> sqlx::Result<Option<(i64, i64)>> {
    let app: Option<Uuid> =
        sqlx::query_scalar("select id from apps where repo_url = $1 order by created_at limit 1")
            .bind(repo_url)
            .fetch_optional(&state.db)
            .await?;
    match app {
        Some(id) => ensure(state, id).await,
        None => Ok(None),
    }
}

/// Forgets the IDs when an app moves to another repository; `ensure` records the new ones.
pub async fn forget(conn: &mut sqlx::PgConnection, app_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("update apps set github_repo_id = null, github_owner_id = null where id = $1")
        .bind(app_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Fills in the IDs for apps linked before they were recorded, a few at a time, hourly.
pub async fn run_fill(state: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        tick.tick().await;
        let apps: Vec<Uuid> = match sqlx::query_scalar(
            "select id from apps where github_repo_id is null and repo_url like 'https://github.com/%'
             order by created_at limit 100",
        )
        .fetch_all(&state.db)
        .await
        {
            Ok(a) => a,
            Err(err) => {
                tracing::error!(error = %err, "repository IDs");
                continue;
            }
        };
        for id in apps {
            if let Err(err) = ensure(&state, id).await {
                tracing::error!(error = %err, "repository IDs");
            }
        }
    }
}
