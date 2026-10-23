// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository access (SECURITY §12.2–12.3, DESIGN §17 decision 11).
//!
//! Slop keeps no copies of source or builds, so it has to be able to read every app's repository.
//! A daily check looks at each app's repository:
//!
//! - **Lost access** (the repository is gone, Slop's access was removed, or a public app's
//!   repository was made private): the app is **frozen**. It takes no new releases, because they
//!   couldn't be checked. Installed copies keep working. The owner is told straight away.
//! - **After 14 days frozen**, a public app is delisted and Private Slop is no longer shared
//!   (`catalog::push_shared_with` leaves it out).
//! - **Access back:** the freeze ends, and an app delisted only for being frozen is listed again.
//!
//! Also here: **making Private Slop public**. The owner asks once the repository is public under
//! an open-source licence; a reviewer checks it again from scratch before it goes in the store.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use serde::Serialize;
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
};

/// How long a frozen app stays in the store or shared before it's taken out.
pub const GRACE: Duration = Duration::days(14);
/// How often each repository is checked.
const CHECK_EVERY: Duration = Duration::days(1);
/// Repositories checked per run (the job runs hourly).
const BATCH: i64 = 200;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/apps/{slug}/make-public",
        post(request_public).delete(cancel_public),
    )
}

/// What a check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Access {
    Readable,
    /// Slop can't read it, or a public app's source isn't public any more. The reason is shown to
    /// the owner.
    Lost(String),
    /// Couldn't tell (the host had an error, or Slop has no way to check this host yet).
    Unknown,
}

/// Checks one repository. `public` says whether its source has to be public (public apps).
/// `check`, using the maker's read-only token for a private repository outside GitHub
/// (repo_tokens.rs) when there is one.
pub async fn check_app(state: &AppState, app_id: Uuid, repo_url: &str, public: bool) -> Access {
    if !public && slop_provenance::github_repo_path(repo_url).is_none() {
        match crate::repo_tokens::still_works(state, app_id).await {
            Some(true) => return Access::Readable,
            Some(false) => {
                return Access::Lost(
                    "Slop's token can't read the repository any more: it was revoked or expired, \
                     or the repository moved. Save a new one on the Manage page"
                        .into(),
                );
            }
            None => {}
        }
    }
    check(state, repo_url, public).await
}

pub async fn check(state: &AppState, repo_url: &str, public: bool) -> Access {
    if let Some(repo) = slop_provenance::github_repo_path(repo_url) {
        let Ok(url) = state
            .config
            .releases
            .github_api_base
            .join(&format!("repos/{repo}"))
        else {
            return Access::Unknown;
        };
        let res = match crate::releases::github_get(state, url)
            .await
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!(%repo, error = %err, "repository check");
                return Access::Unknown;
            }
        };
        return match res.status().as_u16() {
            200 => {
                let info: serde_json::Value = res.json().await.unwrap_or_default();
                if public && info["private"].as_bool() == Some(true) {
                    Access::Lost(
                        "the repository is private now, and apps in the store need public source"
                            .into(),
                    )
                } else {
                    Access::Readable
                }
            }
            404 | 410 | 451 => Access::Lost(
                "Slop can't read the repository: it was deleted, renamed, or Slop's access was removed"
                    .into(),
            ),
            _ => Access::Unknown,
        };
    }
    // Other hosts: a public repository answers Git's own discovery request without signing in.
    // Private repositories on other hosts are checked with the maker's token (check_app).
    if !public {
        return Access::Unknown;
    }
    let Ok(url) = Url::parse(&format!(
        "{}/info/refs?service=git-upload-pack",
        repo_url.trim_end_matches('/')
    )) else {
        return Access::Unknown;
    };
    match state
        .fetch_http
        .get(url)
        .header("user-agent", "git/2.45 (slop-api)")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => Access::Readable,
        Ok(res) if [401, 403, 404, 410].contains(&res.status().as_u16()) => Access::Lost(
            "the repository can't be read without signing in: it was deleted, made private, or moved"
                .into(),
        ),
        _ => Access::Unknown,
    }
}

/// Records one check's result: freezes, unfreezes, and tells the owner.
pub async fn apply(state: &AppState, app_id: Uuid, access: Access) -> anyhow::Result<()> {
    let mut tx = state.db.begin().await?;
    // A freeze staff placed by hand (admin_tools.rs) is theirs to lift: the check leaves it.
    let (maker, slug, title, frozen, frozen_delisted, by_staff): (
        Uuid,
        String,
        String,
        bool,
        bool,
        bool,
    ) = sqlx::query_as(
        "update apps set repo_checked_at = now() where id = $1
         returning maker_id, slug, title, frozen_at is not null and frozen_by is null, frozen_delisted,
                   frozen_by is not null",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;
    // Out of the store for being frozen, but no longer frozen (staff lifted a freeze they placed
    // on top of the automatic one): readable again puts it back, as for the automatic freeze.
    let left_out = frozen_delisted && !frozen && !by_staff;
    match access {
        Access::Unknown => {}
        Access::Lost(reason) if !frozen => {
            let frozen_now = sqlx::query(
                "update apps set frozen_at = now(), frozen_reason = $2 where id = $1 and frozen_at is null",
            )
            .bind(app_id)
            .bind(&reason)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            // Already frozen by staff: the maker heard from them.
            if frozen_now == 0 {
                tx.commit().await?;
                return Ok(());
            }
            notify::send(
                &mut tx,
                Notice {
                    user_id: maker,
                    category: "apps",
                    title: format!("{title} is frozen: Slop can't check its repository"),
                    body: format!(
                        "Slop checked {title}'s repository and {reason}. Until it can read it again, \
                         {title} takes no new releases; copies people installed keep working. \
                         If nothing changes within 14 days, it leaves the store (or stops being \
                         shared, for Private Slop). Restore access and Slop picks it up at the \
                         next daily check, or press Check now on the app's Manage page."
                    ),
                    link: format!("/manage/{slug}"),
                    collapse_key: Some(format!("frozen:{app_id}")),
                },
            )
            .await?;
        }
        Access::Lost(_) => {}
        Access::Readable if frozen || left_out => {
            sqlx::query(
                "update apps set frozen_at = null, frozen_reason = null, frozen_delisted = false,
                        status = case when frozen_delisted then 'listed'::app_status else status end,
                        relist_after_appeal = relist_after_appeal and not frozen_delisted
                 where id = $1",
            )
            .bind(app_id)
            .execute(&mut *tx)
            .await?;
            notify::send(
                &mut tx,
                Notice {
                    user_id: maker,
                    category: "apps",
                    title: format!("{title} isn't frozen any more"),
                    body: format!(
                        "Slop can read {title}'s repository again, so it takes new releases{}.",
                        if frozen_delisted {
                            " and is back in the store"
                        } else {
                            ""
                        }
                    ),
                    link: format!("/manage/{slug}"),
                    collapse_key: Some(format!("frozen:{app_id}")),
                },
            )
            .await?;
        }
        Access::Readable => {}
    }
    tx.commit().await?;
    Ok(())
}

/// Delists public apps frozen for longer than the grace period. Private Slop needs nothing done:
/// `catalog::push_shared_with` stops sharing it by itself.
pub async fn end_grace(state: &AppState) -> anyhow::Result<usize> {
    let mut tx = state.db.begin().await?;
    let gone: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "update apps set status = 'delisted', frozen_delisted = true, updated_at = now()
         where frozen_at < now() - $1 and frozen_by is null and status = 'listed' and visibility = 'public'
         returning id, maker_id, slug, title",
    )
    .bind(GRACE)
    .fetch_all(&mut *tx)
    .await?;
    for (id, maker, slug, title) in &gone {
        notify::send(
            &mut tx,
            Notice {
                user_id: *maker,
                category: "apps",
                title: format!("{title} left the store"),
                body: format!(
                    "Slop still can't read {title}'s repository after 14 days, so it's out of the \
                     store. Installed copies keep working. Restore access and it's listed again."
                ),
                link: format!("/manage/{slug}"),
                collapse_key: Some(format!("frozen:{id}")),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(gone.len())
}

/// The apps whose repositories are due a check: everything that's in the store, waiting to be,
/// or shared, except drafts nobody has seen.
async fn due(state: &AppState) -> sqlx::Result<Vec<(Uuid, String, bool)>> {
    sqlx::query_as(
        "select a.id, a.repo_url, a.visibility = 'public' from apps a
         where (a.status <> 'draft' or a.visibility = 'private' or a.frozen_at is not null)
           and (a.repo_checked_at is null or a.repo_checked_at < now() - $1)
         order by a.repo_checked_at nulls first limit $2",
    )
    .bind(CHECK_EVERY)
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
}

/// One run: check what's due, then end grace periods. Returns how many repositories were checked.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let apps = due(state).await?;
    for (id, repo, public) in &apps {
        let access = check_app(state, *id, repo, *public).await;
        apply(state, *id, access).await?;
    }
    end_grace(state).await?;
    Ok(apps.len())
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "repository checks");
        }
    }
}

/// Whether an app is frozen, for refusing new releases (releases.rs) and approvals (admin.rs).
pub async fn frozen_reason<'c>(
    db: impl sqlx::PgExecutor<'c>,
    app_id: Uuid,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(
        "select coalesce(frozen_reason, 'Slop can''t read the repository') from apps
         where id = $1 and frozen_at is not null",
    )
    .bind(app_id)
    .fetch_optional(db)
    .await
}

/// Whether the freeze is one staff placed by hand (admin_tools.rs).
pub async fn by_staff<'c>(db: impl sqlx::PgExecutor<'c>, app_id: Uuid) -> sqlx::Result<bool> {
    Ok(
        sqlx::query_scalar::<_, bool>("select frozen_by is not null from apps where id = $1")
            .bind(app_id)
            .fetch_optional(db)
            .await?
            .unwrap_or(false),
    )
}

/// What the owner sees on the Manage page.
#[derive(Serialize)]
pub struct Frozen {
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
    /// When it leaves the store or stops being shared, if access isn't back by then.
    #[serde(with = "time::serde::rfc3339")]
    pub until: OffsetDateTime,
    /// Staff froze it by hand (admin_tools.rs): it stays in the store, and staff lift it.
    pub by_staff: bool,
}

pub async fn status(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Option<Frozen>> {
    let row: Option<(String, OffsetDateTime, bool)> = sqlx::query_as(
        "select coalesce(frozen_reason, ''), frozen_at, frozen_by is not null from apps
         where id = $1 and frozen_at is not null",
    )
    .bind(app_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(reason, since, by_staff)| Frozen {
        reason,
        since,
        until: since + GRACE,
        by_staff,
    }))
}

// ---------------------------------------------------------------------------------------------
// Checking now, and making Private Slop public

async fn owned(
    state: &AppState,
    user: &CurrentUser,
    slug: &str,
) -> ApiResult<(Uuid, String, String, String)> {
    let row: Option<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "select id, maker_id, repo_url, visibility, license_spdx from apps where slug = $1",
    )
    .bind(slug)
    .fetch_optional(&state.db)
    .await?;
    match row {
        Some((id, maker, repo, visibility, license)) if maker == user.id => {
            Ok((id, repo, visibility, license))
        }
        _ => Err(ApiError::NotFound),
    }
}

/// "Check now" on the Manage page of a frozen app.
pub async fn check_now(
    state: &AppState,
    user: &CurrentUser,
    slug: &str,
) -> ApiResult<Option<Frozen>> {
    let (id, repo, visibility, _) = owned(state, user, slug).await?;
    let access = check_app(state, id, &repo, visibility == "public").await;
    apply(state, id, access).await?;
    Ok(status(&state.db, id).await?)
}

/// The owner asks for Private Slop to go in the store (SECURITY §12.3). The repository has to be
/// public, under the open-source licence the app names; a reviewer then checks it from scratch.
async fn request_public(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let (id, repo, visibility, license) = owned(&state, &user, &slug).await?;
    if visibility != "private" {
        return Err(ApiError::Conflict("it's already public".into()));
    }
    crate::catalog::validate_license(&license)?;
    match check(&state, &repo, true).await {
        Access::Readable => {}
        Access::Lost(reason) => {
            return Err(ApiError::Conflict(format!(
                "make the repository public first: {reason}"
            )));
        }
        Access::Unknown => {
            return Err(ApiError::Conflict(
                "Slop couldn't confirm the repository is public. Make sure anyone can read it \
                 without signing in, then try again."
                    .into(),
            ));
        }
    }
    sqlx::query(
        "update apps set public_requested_at = now() where id = $1 and public_requested_at is null",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_public(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let (id, ..) = owned(&state, &user, &slug).await?;
    sqlx::query("update apps set public_requested_at = null where id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
