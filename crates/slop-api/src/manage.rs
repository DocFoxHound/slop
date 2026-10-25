// SPDX-License-Identifier: AGPL-3.0-or-later
//! Makers manage their apps (decided 2026-10-19).
//!
//! A change that would have players download something different is checked before it happens:
//!
//! - **Which release players get.** Makers can hold the app at any approved release (to roll
//!   back a bad update, or to wait before sending a new one out) or go back to following the
//!   newest. This follows the same rule as new releases (updates.rs): approved automatically once
//!   a person has approved one of the app's releases, otherwise a person looks. Every release
//!   offered has already passed its checks.
//! - **The repository.** Moving an app that players have to another repository always waits for
//!   a person, and the next release after a move is reviewed by a person too. Players' apps
//!   still refuse an update from a different repository until the player agrees (slop-client).
//!
//! Everything else (the store page, platforms, update settings) the maker changes directly.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::CurrentUser,
    catalog::validate_repo_url_in,
    error::{ApiError, ApiResult},
    releases::SERVING,
    updates::{self, AUTO_REVIEWER},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/manage", get(overview))
        .route("/apps/{slug}/changes", post(request_change))
        .route("/apps/{slug}/changes/{id}/cancel", post(cancel_change))
        .route("/apps/{slug}/repo-check", post(repo_check))
}

/// "Check now" for a frozen app's repository (frozen.rs).
async fn repo_check(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Option<crate::frozen::Frozen>>> {
    Ok(Json(crate::frozen::check_now(&state, &user, &slug).await?))
}

/// The longest reason a maker can give with a request.
const MAX_NOTE: usize = 500;

/// Whether two repository URLs are the same repository, the way players' apps compare them.
pub(crate) fn same_repo(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Whether players may have the app (it has had an approved release), so moving it is checked.
pub(crate) async fn repo_locked(conn: &mut PgConnection, app_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from releases where app_id = $1 and status in ('approved', 'revoked'))",
    )
    .bind(app_id)
    .fetch_one(conn)
    .await
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct ChangeInfo {
    pub id: Uuid,
    pub kind: String,
    /// kind = release: the release to hold players at; null means "follow the newest".
    pub release_id: Option<Uuid>,
    pub release_version: Option<String>,
    /// kind = repo: the new repository.
    pub repo_url: Option<String>,
    pub note: Option<String>,
    pub status: String,
    pub status_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
    /// Approved by Slop without a person.
    pub automatic: bool,
}

pub(crate) fn change_columns() -> String {
    format!(
        "c.id, c.kind, c.release_id, r.version as release_version, c.repo_url, c.note, c.status,
         c.status_reason, c.created_at, c.reviewed_at,
         coalesce(c.reviewed_by = '{AUTO_REVIEWER}', false) as automatic
         from app_changes c left join releases r on r.id = c.release_id"
    )
}

async fn load_change(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<ChangeInfo> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {} where c.id = $1",
        change_columns()
    )))
    .bind(id)
    .fetch_one(conn)
    .await
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct ApprovedRelease {
    pub id: Uuid,
    pub version: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
    pub automatic: bool,
}

const APPROVED_COLUMNS: &str =
    "r.id, r.version, r.reviewed_at, coalesce(r.reviewed_by = 'slop', false) as automatic";

/// A release not reviewed yet, which its maker may withdraw (releases.rs).
#[derive(Serialize, sqlx::FromRow)]
struct WaitingRelease {
    id: Uuid,
    version: String,
    /// fetching or awaiting_review.
    status: String,
    /// maker, slop_build or github_release.
    source: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// The release players get now.
pub(crate) async fn serving(
    conn: &mut PgConnection,
    app_id: Uuid,
) -> sqlx::Result<Option<ApprovedRelease>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {APPROVED_COLUMNS} {SERVING} limit 1"
    )))
    .bind(app_id)
    .fetch_optional(conn)
    .await
}

/// The newest approved release, which players get when the app isn't held at one.
async fn newest(conn: &mut PgConnection, app_id: Uuid) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "select id from releases
         where app_id = $1 and status = 'approved' and signed_manifest is not null
         order by reviewed_at desc limit 1",
    )
    .bind(app_id)
    .fetch_optional(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// The maker's view

#[derive(Serialize)]
struct Overview {
    /// The release players get now.
    serving: Option<ApprovedRelease>,
    /// Whether the maker holds the app at `serving` instead of following the newest.
    held: bool,
    /// Approved releases players could be given, newest first.
    approved: Vec<ApprovedRelease>,
    /// Releases not reviewed yet, newest first: the maker can withdraw one they didn't send.
    waiting: Vec<WaitingRelease>,
    repo_url: String,
    /// Players have the app, so moving it to another repository is reviewed by a person.
    repo_locked: bool,
    /// Whether a change of release would be approved without a person, and if not, why.
    auto_approval_ready: bool,
    auto_approval_note: Option<String>,
    /// Open requests first, then the latest decisions.
    changes: Vec<ChangeInfo>,
    /// Slop can't read the repository (frozen.rs): no new releases until it can.
    frozen: Option<crate::frozen::Frozen>,
    /// Private Slop: the owner asked to make it public and it waits for a reviewer.
    public_requested: bool,
    /// Community builds of games whose original counts players with Slop Pulse need a Pulse
    /// handshake before they're listed (DESIGN §9): none, waiting, or received.
    pulse_required: Option<bool>,
}

async fn overview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Overview>> {
    let (app_id, repo_url) = updates::owned(&state, &slug, &user).await?;
    let approval = updates::auto_approval(&state.db, app_id).await?;
    let mut conn = state.db.acquire().await?;
    let serving = serving(&mut conn, app_id).await?;
    let pinned: Option<Uuid> =
        sqlx::query_scalar("select pinned_release_id from apps where id = $1")
            .bind(app_id)
            .fetch_one(&mut *conn)
            .await?;
    let approved = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {APPROVED_COLUMNS} from releases r
         where r.app_id = $1 and r.status = 'approved' and r.signed_manifest is not null
         order by r.reviewed_at desc limit 50"
    )))
    .bind(app_id)
    .fetch_all(&mut *conn)
    .await?;
    let waiting = sqlx::query_as(
        "select id, version, status::text as status, source, created_at from releases
         where app_id = $1 and status in ('fetching', 'awaiting_review') order by created_at desc",
    )
    .bind(app_id)
    .fetch_all(&mut *conn)
    .await?;
    let changes = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {} where c.app_id = $1
         order by (c.status = 'pending') desc, c.created_at desc limit 20",
        change_columns()
    )))
    .bind(app_id)
    .fetch_all(&mut *conn)
    .await?;
    let public_requested: bool =
        sqlx::query_scalar("select public_requested_at is not null from apps where id = $1")
            .bind(app_id)
            .fetch_one(&mut *conn)
            .await?;
    let pulse_required = match crate::branches::needs_pulse(&mut conn, app_id).await? {
        true => Some(crate::branches::has_pulse(&mut conn, app_id).await?),
        false => None,
    };
    Ok(Json(Overview {
        frozen: crate::frozen::status(&state.db, app_id).await?,
        public_requested,
        pulse_required,
        held: pinned.is_some() && pinned == serving.as_ref().map(|r| r.id),
        serving,
        approved,
        waiting,
        repo_locked: repo_locked(&mut conn, app_id).await?,
        repo_url,
        auto_approval_ready: approval.is_ok(),
        auto_approval_note: approval.err(),
        changes,
    }))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChangeRequest {
    /// Hold players at `release_id`, or follow the newest approved release when it's null.
    Release {
        release_id: Option<Uuid>,
        #[serde(default)]
        note: Option<String>,
    },
    Repo {
        repo_url: String,
        #[serde(default)]
        note: Option<String>,
    },
}

/// What happens to a request right away.
enum Decision {
    /// Applied now; the reason says why no one needed to look.
    Apply(&'static str),
    /// Waits for a person, for this reason.
    Wait(String),
}

async fn request_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(req): Json<ChangeRequest>,
) -> ApiResult<(StatusCode, Json<ChangeInfo>)> {
    let (app_id, current_repo) = updates::owned(&state, &slug, &user).await?;
    let approval = updates::auto_approval(&state.db, app_id).await?;
    let clean_note = |note: Option<String>| -> ApiResult<Option<String>> {
        let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE) {
            return Err(ApiError::invalid(
                "note",
                format!("keep it under {MAX_NOTE} characters"),
            ));
        }
        Ok(note)
    };

    let mut tx = state.db.begin().await?;
    let pinned: Option<Uuid> =
        sqlx::query_scalar("select pinned_release_id from apps where id = $1 for update")
            .bind(app_id)
            .fetch_one(&mut *tx)
            .await?;

    let (kind, release_id, repo_url, note, decision) = match req {
        ChangeRequest::Release { release_id, note } => {
            let note = clean_note(note)?;
            if let Some(id) = release_id {
                let ok: bool = sqlx::query_scalar(
                    "select exists (select 1 from releases where id = $1 and app_id = $2
                                     and status = 'approved' and signed_manifest is not null)",
                )
                .bind(id)
                .bind(app_id)
                .fetch_one(&mut *tx)
                .await?;
                if !ok {
                    return Err(ApiError::invalid(
                        "release_id",
                        "choose one of this app's approved releases",
                    ));
                }
            }
            let now = serving(&mut tx, app_id).await?.map(|r| r.id);
            let after = match release_id {
                Some(id) => Some(id),
                None => newest(&mut tx, app_id).await?,
            };
            let decision = if after == now {
                if pinned == release_id {
                    return Err(ApiError::Conflict(match release_id {
                        Some(_) => "players already get that release".into(),
                        None => "players already get the newest approved release".into(),
                    }));
                }
                Decision::Apply("Players already get this release, so there was nothing to check.")
            } else {
                match &approval {
                    Ok(()) => Decision::Apply(
                        "Approved automatically: the release already passed every check.",
                    ),
                    Err(why) => Decision::Wait(format!("Waiting for a person: {why}.")),
                }
            };
            ("release", release_id, None, note, decision)
        }
        ChangeRequest::Repo { repo_url, note } => {
            let note = clean_note(note)?;
            let repo_url = validate_repo_url_in(&state, &repo_url)?;
            if same_repo(&repo_url, &current_repo) {
                return Err(ApiError::Conflict(
                    "that's already the app's repository".into(),
                ));
            }
            crate::repo_owner::check(&state, user.id, &repo_url, Some(app_id)).await?;
            let decision = if repo_locked(&mut tx, app_id).await? {
                Decision::Wait(
                    "Waiting for a person: moving an app players already have is always checked."
                        .into(),
                )
            } else {
                Decision::Apply("No one has this app yet, so it moved straight away.")
            };
            ("repo", None, Some(repo_url), note, decision)
        }
    };

    // A new request replaces an open one of the same kind.
    sqlx::query(
        "update app_changes set status = 'cancelled', reviewed_at = now(),
                status_reason = 'Replaced by a newer request.'
         where app_id = $1 and kind = $2 and status = 'pending'",
    )
    .bind(app_id)
    .bind(kind)
    .execute(&mut *tx)
    .await?;
    let id = Uuid::now_v7();
    let reason = match &decision {
        Decision::Apply(why) => why.to_string(),
        Decision::Wait(why) => why.clone(),
    };
    sqlx::query(
        "insert into app_changes (id, app_id, kind, release_id, repo_url, note, status_reason, requested_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(app_id)
    .bind(kind)
    .bind(release_id)
    .bind(&repo_url)
    .bind(&note)
    .bind(&reason)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if matches!(decision, Decision::Apply(_)) {
        apply(&mut tx, id, AUTO_REVIEWER)
            .await
            .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    }
    let change = load_change(&mut tx, id).await?;
    tx.commit().await?;
    if kind == "repo" {
        state.watch_wakeup.notify_one();
    }
    Ok((StatusCode::CREATED, Json(change)))
}

async fn cancel_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ChangeInfo>> {
    let (app_id, _) = updates::owned(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query(
        "update app_changes set status = 'cancelled', reviewed_at = now(), status_reason = 'Cancelled by the maker.'
         where id = $1 and app_id = $2 and status = 'pending'",
    )
    .bind(id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::Conflict(
            "that request isn't waiting any more".into(),
        ));
    }
    let change = load_change(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(change))
}

// ---------------------------------------------------------------------------------------------
// Review

/// A pending change: (app ID, kind, release ID, repository, app slug).
type PendingChange = (Uuid, String, Option<Uuid>, Option<String>, String);

/// Makes a pending change happen, recording who approved it. Refuses if it no longer can.
async fn apply(tx: &mut PgConnection, id: Uuid, reviewer: &str) -> anyhow::Result<()> {
    let row: Option<PendingChange> = sqlx::query_as(
        "select c.app_id, c.kind, c.release_id, c.repo_url, a.slug
         from app_changes c join apps a on a.id = c.app_id
         where c.id = $1 and c.status = 'pending' for update of c, a",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((app_id, kind, release_id, repo_url, slug)) = row else {
        anyhow::bail!("no pending change with that ID");
    };
    let detail = match kind.as_str() {
        "release" => {
            let version: Option<String> = match release_id {
                Some(rid) => Some(
                    sqlx::query_scalar(
                        "select version from releases where id = $1 and status = 'approved'",
                    )
                    .bind(rid)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("that release was withdrawn"))?,
                ),
                None => None,
            };
            sqlx::query("update apps set pinned_release_id = $2, updated_at = now() where id = $1")
                .bind(app_id)
                .bind(release_id)
                .execute(&mut *tx)
                .await?;
            json!({ "change": id, "hold_at": version })
        }
        _ => {
            let repo_url = repo_url.unwrap_or_default();
            // A new repository starts afresh: nothing seen there yet, and its first release
            // waits for a person (updates::auto_approval).
            sqlx::query(
                "update apps set repo_url = $2, repo_changed_at = now(), watch_last_tag = null,
                        watch_problem = null, watch_checked_at = null, updated_at = now()
                 where id = $1",
            )
            .bind(app_id)
            .bind(&repo_url)
            .execute(&mut *tx)
            .await?;
            // The new repository's IDs are recorded on first use (repo_identity.rs).
            crate::repo_identity::forget(&mut *tx, app_id).await?;
            json!({ "change": id, "repo_url": repo_url })
        }
    };
    sqlx::query(
        "update app_changes set status = 'approved', reviewed_by = $2, reviewed_at = now(),
                status_reason = case when $2 = 'slop' then status_reason end
         where id = $1",
    )
    .bind(id)
    .bind(reviewer)
    .execute(&mut *tx)
    .await?;
    admin::audit(tx, reviewer, &format!("app.{kind}_change"), &slug, detail).await?;
    Ok(())
}

/// An admin approves a maker's change.
pub async fn approve_change(db: &sqlx::PgPool, id: Uuid, reviewer: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!reviewer.trim().is_empty(), "say who you are");
    let mut tx = db.begin().await?;
    apply(&mut tx, id, reviewer).await?;
    tx.commit().await?;
    Ok(())
}

/// An admin turns down a maker's change, saying why (the maker sees it).
pub async fn reject_change(
    db: &sqlx::PgPool,
    id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(!reviewer.trim().is_empty(), "say who you are");
    anyhow::ensure!(!reason.trim().is_empty(), "say why; the maker sees it");
    let mut tx = db.begin().await?;
    let slug: Option<String> = sqlx::query_scalar(
        "update app_changes c set status = 'rejected', reviewed_by = $2, reviewed_at = now(), status_reason = $3
         from apps a where a.id = c.app_id and c.id = $1 and c.status = 'pending'
         returning a.slug",
    )
    .bind(id)
    .bind(reviewer)
    .bind(reason.trim())
    .fetch_optional(&mut *tx)
    .await?;
    let slug = slug.ok_or_else(|| anyhow::anyhow!("no pending change with that ID"))?;
    admin::audit(
        &mut tx,
        reviewer,
        "app.change_reject",
        &slug,
        json!({ "change": id, "reason": reason.trim() }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_repo;

    #[test]
    fn repositories_compare_like_players_apps_do() {
        assert!(same_repo(
            "https://github.com/A/b",
            "https://github.com/a/b.git"
        ));
        assert!(same_repo(
            "https://github.com/a/b/",
            "https://github.com/a/b"
        ));
        assert!(!same_repo(
            "https://github.com/a/b",
            "https://github.com/a/c"
        ));
    }
}
