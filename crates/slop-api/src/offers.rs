// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offering a community build's changes back to the original (DESIGN §9, plan phases 10 and 12).
//!
//! The community build's maker (or their agent) opens a pull request on the original's
//! repository, then registers it here; the original's maker is told, and accepts or declines.
//! Merging happens on the repository: Slop only keeps track and tells both sides, so it needs no
//! write access to anyone's code.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult, is_unique_violation},
    notify,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/offers", get(list).post(offer))
        .route("/apps/{slug}/offers/{id}/answer", post(answer))
        .route("/apps/{slug}/offers/{id}/withdraw", post(withdraw))
}

#[derive(Serialize, sqlx::FromRow)]
struct Offer {
    id: Uuid,
    /// `received` (offered to this app) or `sent` (offered by this community build).
    direction: String,
    branch_slug: String,
    branch_title: String,
    original_slug: String,
    original_title: String,
    offered_by: String,
    pull_request_url: String,
    message: String,
    status: String,
    answer: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    answered_at: Option<OffsetDateTime>,
}

const COLUMNS: &str = "o.id, b.slug as branch_slug, b.title as branch_title, p.slug as original_slug,
    p.title as original_title, u.display_name as offered_by, o.pull_request_url, o.message, o.status,
    o.answer, o.created_at, o.answered_at
    from upstream_offers o join apps b on b.id = o.branch_app_id join apps p on p.id = o.original_app_id
    join users u on u.id = o.offered_by";

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Offer>>> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let offers = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select case when o.original_app_id = $1 then 'received' else 'sent' end as direction, {COLUMNS}
         where o.original_app_id = $1 or o.branch_app_id = $1 order by o.created_at desc limit 100"
    )))
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(offers))
}

/// A pull request URL on the original's repository: GitHub's `/pull/N`, GitLab's
/// `/-/merge_requests/N`, or Gitea, Forgejo and Codeberg's `/pulls/N`.
fn is_pull_request_of(url: &str, repo_url: &str) -> bool {
    let repo = repo_url.trim_end_matches('/').trim_end_matches(".git");
    let Some(rest) = url.strip_prefix(repo).and_then(|r| r.strip_prefix('/')) else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    let number = ["pull/", "-/merge_requests/", "pulls/"]
        .iter()
        .find_map(|p| rest.strip_prefix(p));
    url.starts_with("https://")
        && number.is_some_and(|n| {
            !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit())
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewOffer {
    pull_request_url: String,
    message: String,
}

async fn offer(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<NewOffer>,
) -> ApiResult<(StatusCode, Json<Offer>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::Offer)?;
    let (branch_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let original: Option<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "select p.id, p.maker_id, p.repo_url, b.title, p.slug from apps b join apps p on p.id = b.parent_app_id
         where b.id = $1",
    )
    .bind(branch_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((original_id, original_maker, repo_url, branch_title, original_slug)) = original
    else {
        return Err(ApiError::invalid(
            "slug",
            "only a community build can offer its changes to an original",
        ));
    };
    let url = body.pull_request_url.trim();
    if !is_pull_request_of(url, &repo_url) {
        return Err(ApiError::invalid(
            "pull_request_url",
            format!("a pull or merge request on the original's repository, {repo_url}"),
        ));
    }
    let message = body.message.trim();
    if message.is_empty() || message.chars().count() > 2000 {
        return Err(ApiError::invalid("message", "1 to 2,000 characters"));
    }
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into upstream_offers (id, branch_app_id, original_app_id, offered_by, pull_request_url, message)
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(branch_id)
    .bind(original_id)
    .bind(user.id)
    .bind(url)
    .bind(message)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::Conflict("that pull request is already offered".into())
        } else {
            e.into()
        }
    })?;
    notify::send(
        &mut tx,
        notify::Notice {
            user_id: original_maker,
            category: "apps",
            title: format!("{branch_title} offered you a fix"),
            body: format!(
                "{}: {}",
                user.display_name,
                message.chars().take(200).collect::<String>()
            ),
            link: format!("/publish/{original_slug}/manage#offers"),
            // One notice per branch until it's read, however many it sends.
            collapse_key: Some(format!("offer:{branch_id}")),
        },
    )
    .await?;
    let offer = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select 'sent' as direction, {COLUMNS} where o.id = $1"
    )))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(offer)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    /// `accepted` or `declined`.
    decision: String,
    #[serde(default)]
    message: Option<String>,
}

async fn answer(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<Answer>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    if body.decision != "accepted" && body.decision != "declined" {
        return Err(ApiError::invalid("decision", "accepted or declined"));
    }
    let message = body
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if message.is_some_and(|m| m.chars().count() > 2000) {
        return Err(ApiError::invalid("message", "up to 2,000 characters"));
    }
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "update upstream_offers o set status = $3, answer = $4, answered_at = now()
         from apps b, apps p
         where o.id = $1 and o.original_app_id = $2 and o.status = 'open'
           and b.id = o.branch_app_id and p.id = o.original_app_id
         returning o.offered_by, b.slug, p.title",
    )
    .bind(id)
    .bind(app_id)
    .bind(&body.decision)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await?;
    let (offered_by, branch_slug, original_title) = row.ok_or(ApiError::NotFound)?;
    notify::send(
        &mut tx,
        notify::Notice {
            user_id: offered_by,
            category: "apps",
            title: format!("{original_title} {} your fix", body.decision),
            body: message.unwrap_or_default().chars().take(200).collect(),
            link: format!("/publish/{branch_slug}/manage#offers"),
            collapse_key: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn withdraw(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let done = sqlx::query(
        "update upstream_offers set status = 'withdrawn', answered_at = now()
         where id = $1 and branch_app_id = $2 and status = 'open'",
    )
    .bind(id)
    .bind(app_id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::is_pull_request_of;

    #[test]
    fn only_pull_requests_on_the_original() {
        let repo = "https://github.com/octo/game";
        assert!(is_pull_request_of(
            "https://github.com/octo/game/pull/12",
            repo
        ));
        assert!(is_pull_request_of(
            "https://github.com/octo/game/pull/12",
            "https://github.com/octo/game.git"
        ));
        assert!(is_pull_request_of(
            "https://gitlab.com/o/g/-/merge_requests/3",
            "https://gitlab.com/o/g"
        ));
        assert!(is_pull_request_of(
            "https://codeberg.org/o/g/pulls/7",
            "https://codeberg.org/o/g"
        ));
        for bad in [
            "https://github.com/octo/game-evil/pull/12",
            "https://github.com/octo/game/pull/12/../../x",
            "https://github.com/octo/game/issues/12",
            "https://github.com/octo/game/pull/",
            "http://github.com/octo/game/pull/12",
        ] {
            assert!(!is_pull_request_of(bad, repo), "{bad}");
        }
    }
}
