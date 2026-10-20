//! Interested (DESIGN §5, §6, §8): people mark Coming soon and Beta apps they want. It puts them
//! in their library, counts publicly as the app's demand signal ("Most anticipated"), feeds "For
//! you", and tells them when the app moves to Beta or is released.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::Serialize;
use sqlx::QueryBuilder;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppStage, AppSummary},
    error::{ApiError, ApiResult},
    foryou,
    notify::{self, Notice},
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/interested", get(list))
        .route("/me/interested/{slug}", put(add).delete(remove))
        .route("/upcoming/for-you", get(upcoming_for_you))
}

#[derive(Serialize, sqlx::FromRow)]
struct Entry {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    #[serde(with = "time::serde::rfc3339")]
    interested_at: OffsetDateTime,
}

/// Everything the user marked, newest first, including apps that have since been released (the
/// library offers to add them) or left the store.
async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<Entry>>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, &state.config, ", i.created_at as interested_at");
    qb.push(" join interests i on i.app_id = a.id where i.user_id = ")
        .push_bind(user.id)
        .push(" order by i.created_at desc");
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}

async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, mine) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    if mine {
        return Err(ApiError::Conflict(
            "it's your own app: Interested counts other people".into(),
        ));
    }
    let stage: AppStage = sqlx::query_scalar("select stage from apps where id = $1")
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    if stage == AppStage::Released {
        return Err(ApiError::Conflict(
            "it's released: add it to your library instead".into(),
        ));
    }
    sqlx::query("insert into interests (user_id, app_id) values ($1, $2) on conflict do nothing")
        .bind(user.id)
        .bind(app_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    sqlx::query(
        "delete from interests i using apps a where a.id = i.app_id and i.user_id = $1 and a.slug = $2",
    )
    .bind(user.id)
    .bind(&slug)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Tells everyone Interested that an app moved to Beta or was released.
pub(crate) async fn stage_moved(
    tx: &mut sqlx::PgConnection,
    app_id: Uuid,
    stage: AppStage,
) -> sqlx::Result<usize> {
    let what = match stage {
        AppStage::Beta => "is in beta: you can try it now",
        AppStage::Released => "is out now",
        AppStage::ComingSoon => return Ok(0),
    };
    let (slug, title): (String, String) =
        sqlx::query_as("select slug, title from apps where id = $1")
            .bind(app_id)
            .fetch_one(&mut *tx)
            .await?;
    let users: Vec<Uuid> = sqlx::query_scalar("select user_id from interests where app_id = $1")
        .bind(app_id)
        .fetch_all(&mut *tx)
        .await?;
    for user_id in &users {
        notify::send(
            tx,
            Notice {
                user_id: *user_id,
                category: "interested",
                title: format!("{title} {what}"),
                body: "You marked it Interested.".into(),
                link: format!("/app/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    Ok(users.len())
}

#[derive(Serialize)]
struct Card {
    #[serde(flatten)]
    app: AppSummary,
    reason: String,
}

/// "Coming soon & in beta, for you" (DESIGN §5): upcoming apps scored by the tags in the user's
/// library (weight 1) and Interested list (weight 2), plus votes and play as in "For you",
/// boosted by log(1 + Interested count). Dead projects and apps already marked are left out.
/// Without personal recommendations, it's the most anticipated.
async fn upcoming_for_you(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Card>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let personal = match &user {
        Some(u) => foryou::personalized(&state.db, u.id, policy).await?,
        None => false,
    };
    let mut candidates: Vec<foryou::Candidate> =
        foryou::candidates(&state.db, &state.config, policy)
            .await?
            .into_iter()
            .filter(|c| c.app.stage != AppStage::Released && !c.app.is_dead)
            .collect();
    let Some(u) = user.filter(|_| personal) else {
        candidates.sort_by(|a, b| {
            b.app
                .interested
                .cmp(&a.app.interested)
                .then(a.app.slug.cmp(&b.app.slug))
        });
        return Ok(Json(
            candidates
                .into_iter()
                .take(12)
                .map(|c| Card {
                    reason: format!("{} interested", c.app.interested),
                    app: c.app,
                })
                .collect(),
        ));
    };
    let signals = foryou::signals(&state.db, u.id).await?;
    let all = foryou::candidates(&state.db, &state.config, ViewerPolicy::FULL_VIEW).await?;
    let tags_of: HashMap<Uuid, Vec<String>> =
        all.iter().map(|c| (c.id, c.app.tags.clone())).collect();
    let taste = foryou::taste(&signals, &tags_of);
    let names = foryou::tag_names(&state.db).await?;
    let mut scored: Vec<(f64, String, foryou::Candidate)> = candidates
        .into_iter()
        .filter(|c| {
            !signals.library.contains(&c.id)
                && !signals.interested.contains(&c.id)
                && c.maker_id != u.id
        })
        .map(|c| {
            let mut matched: Vec<(&String, f64)> = c
                .app
                .tags
                .iter()
                .filter_map(|t| taste.get(t).filter(|w| **w > 0.0).map(|w| (t, *w)))
                .collect();
            matched.sort_by(|a, b| b.1.total_cmp(&a.1));
            let fit: f64 = c
                .app
                .tags
                .iter()
                .map(|t| taste.get(t).copied().unwrap_or(0.0))
                .sum::<f64>()
                / (c.app.tags.len().max(1) as f64).sqrt();
            let score = fit + 0.5 * (1.0 + c.app.interested as f64).ln();
            let reason = if matched.is_empty() {
                format!("{} interested", c.app.interested)
            } else {
                let liked: Vec<&str> = matched
                    .iter()
                    .take(2)
                    .map(|(t, _)| names.get(*t).map_or(t.as_str(), String::as_str))
                    .collect();
                format!("Because you like {}", liked.join(" & "))
            };
            (score, reason, c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.app.slug.cmp(&b.2.app.slug)));
    Ok(Json(
        scored
            .into_iter()
            .take(12)
            .map(|(_, reason, c)| Card { app: c.app, reason })
            .collect(),
    ))
}
