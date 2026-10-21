// SPDX-License-Identifier: AGPL-3.0-or-later
//! Private Slop (DESIGN §13, SECURITY §12): apps only their owner and the people they choose can
//! see. Same requirements as public apps (an approved release), and the source may be a private
//! repository Slop can read; builds stay on the owner's release page and are downloaded through
//! short-lived links (releases.rs). Never in the store, search or recommendations; no store
//! listing, donations or voting. Access is checked on every request (`catalog::push_shared_with`).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult},
    notify::{self, Notice},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/private", get(mine))
        .route("/apps/{slug}/shares", get(shares).post(share))
        .route("/apps/{slug}/shares/{user_id}", delete(unshare))
        .route("/apps/{slug}/team-shares/{team}", delete(unshare_team))
}

async fn owned_private(
    state: &AppState,
    user: &CurrentUser,
    slug: &str,
) -> ApiResult<(Uuid, String)> {
    let row: Option<(Uuid, Uuid, String, String)> =
        sqlx::query_as("select id, maker_id, title, visibility from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some((id, maker, title, visibility)) if maker == user.id => {
            if visibility != "private" {
                return Err(ApiError::Conflict(
                    "only Private Slop is shared this way; public apps are in the store".into(),
                ));
            }
            Ok((id, title))
        }
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Share {
    /// A person's account, or none for a team.
    user_id: Option<Uuid>,
    /// A team's address, or none for a person.
    team: Option<String>,
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    shared_at: OffsetDateTime,
}

async fn shares(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Share>>> {
    let (app_id, _) = owned_private(&state, &user, &slug).await?;
    Ok(Json(
        sqlx::query_as(
            "select s.user_id, null::text as team, u.display_name as name, s.created_at as shared_at from app_shares s
             join users u on u.id = s.user_id where s.app_id = $1
             union all
             select null, t.slug, t.name, ts.created_at from app_team_shares ts join teams t on t.id = ts.team_id
             where ts.app_id = $1
             order by name",
        )
        .bind(app_id)
        .fetch_all(&state.db)
        .await?,
    ))
}

/// Shares with a player team the owner is in, and tells its other members.
async fn share_team(
    state: &AppState,
    user: &CurrentUser,
    app_id: Uuid,
    title: &str,
    slug: &str,
    team: &str,
) -> ApiResult<()> {
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "select t.id, t.kind, t.name from teams t join team_members m on m.team_id = t.id and m.user_id = $2
         where t.slug = $1",
    )
    .bind(team)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (team_id, kind, team_name) =
        row.ok_or_else(|| ApiError::invalid("team", "pick a team you're in"))?;
    if kind != "player" {
        return Err(ApiError::invalid(
            "team",
            "Private Slop is never shared with official teams",
        ));
    }
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "insert into app_team_shares (app_id, team_id) values ($1, $2) on conflict do nothing",
    )
    .bind(app_id)
    .bind(team_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 {
        let members: Vec<Uuid> = sqlx::query_scalar(
            "select user_id from team_members where team_id = $1 and user_id <> $2",
        )
        .bind(team_id)
        .bind(user.id)
        .fetch_all(&mut *tx)
        .await?;
        for m in members {
            notify::send(
                &mut tx,
                Notice {
                    user_id: m,
                    category: "shared",
                    title: format!("{} shared {title} with {team_name}", user.display_name),
                    body: "It's Private Slop: only the team can see it.".into(),
                    link: format!("/app/{slug}"),
                    collapse_key: None,
                },
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn unshare_team(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, team)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = owned_private(&state, &user, &slug).await?;
    sqlx::query("delete from app_team_shares ts using teams t where t.id = ts.team_id and ts.app_id = $1 and t.slug = $2")
        .bind(app_id)
        .bind(&team)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareBody {
    /// Their profile link (…/u/<id>) or account ID.
    user: Option<String>,
    /// Or a player team the owner belongs to (never an official team: DESIGN §13).
    team: Option<String>,
}

/// Picks the account ID out of a profile link or a bare ID.
pub fn parse_user(value: &str) -> Option<Uuid> {
    let v = value.trim().trim_end_matches('/');
    v.rsplit('/').next().and_then(|id| id.parse().ok())
}

async fn share(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<ShareBody>,
) -> ApiResult<(StatusCode, Json<Vec<Share>>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::Share)?;
    let (app_id, title) = owned_private(&state, &user, &slug).await?;
    if let Some(team) = b.team.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        share_team(&state, &user, app_id, &title, &slug, team).await?;
        return Ok((
            StatusCode::CREATED,
            shares(State(state), user, Path(slug)).await?,
        ));
    }
    let to = parse_user(b.user.as_deref().unwrap_or("")).ok_or_else(|| {
        ApiError::invalid(
            "user",
            "paste their profile link (from their profile page) or account ID",
        )
    })?;
    if to == user.id {
        return Err(ApiError::invalid("user", "it's already yours"));
    }
    let exists: bool = sqlx::query_scalar(
        "select exists (select 1 from users where id = $1 and closed_at is null and suspended_at is null)",
    )
    .bind(to)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(ApiError::invalid("user", "no Slop account has that link"));
    }
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "insert into app_shares (app_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(app_id)
    .bind(to)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 {
        notify::send(
            &mut tx,
            Notice {
                user_id: to,
                category: "shared",
                title: format!("{} shared {title} with you", user.display_name),
                body: "It's Private Slop: only the people it's shared with can see it.".into(),
                link: format!("/app/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        shares(State(state), user, Path(slug)).await?,
    ))
}

async fn unshare(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, user_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = owned_private(&state, &user, &slug).await?;
    sqlx::query("delete from app_shares where app_id = $1 and user_id = $2")
        .bind(app_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
struct SharedApp {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    owner: String,
}

#[derive(Serialize)]
struct Mine {
    /// The user's own Private Slop.
    mine: Vec<AppSummary>,
    /// Shared with them (with an approved release).
    shared: Vec<SharedApp>,
}

async fn mine(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns(&mut qb, &state.config);
    qb.push(" where a.visibility = 'private' and a.maker_id = ")
        .push_bind(user.id)
        .push(" order by a.updated_at desc");
    let mine = qb.build_query_as().fetch_all(&state.db).await?;
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(
        &mut qb,
        &state.config,
        ", (select u.display_name from users u where u.id = a.maker_id) as owner",
    );
    qb.push(" where ");
    catalog::push_shared_with(&mut qb, Some(user.id));
    qb.push(" order by a.title");
    let shared = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(Json(Mine { mine, shared }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_links_and_ids_both_work() {
        let id = Uuid::now_v7();
        assert_eq!(parse_user(&id.to_string()), Some(id));
        assert_eq!(
            parse_user(&format!("https://slop.example/u/{id}")),
            Some(id)
        );
        assert_eq!(parse_user(&format!(" /u/{id}/ ")), Some(id));
        assert_eq!(parse_user("Octo Cat"), None);
    }
}
