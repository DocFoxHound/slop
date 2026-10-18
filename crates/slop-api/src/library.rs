//! A user's library (DESIGN §6). Adding is free and instant; installing happens on the device.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::Serialize;
use sqlx::QueryBuilder;
use time::OffsetDateTime;

use crate::{
    AppState,
    auth::session::CurrentUser,
    catalog::{self, AppSummary},
    error::ApiResult,
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/library", get(list))
        .route("/me/library/{slug}", put(add).delete(remove))
}

#[derive(Serialize)]
struct Entry {
    #[serde(flatten)]
    app: AppSummary,
    #[serde(with = "time::serde::rfc3339")]
    added_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct Row {
    #[sqlx(flatten)]
    app: AppSummary,
    added_at: OffsetDateTime,
}

/// The library, newest first. Apps that have left the store stay listed, so people can still
/// launch what they installed (DESIGN §8: existing installs keep working).
async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<Entry>>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, state.config.catalog, ", l.added_at");
    qb.push(" join library_entries l on l.app_id = a.id where l.user_id = ")
        .push_bind(user.id)
        .push(" order by l.added_at desc");
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Entry {
                app: r.app,
                added_at: r.added_at,
            })
            .collect(),
    ))
}

/// Adds an app the user can see in the store (or their own).
async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    sqlx::query(
        "insert into library_entries (user_id, app_id) values ($1, $2) on conflict do nothing",
    )
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
    sqlx::query("delete from library_entries l using apps a where a.id = l.app_id and l.user_id = $1 and a.slug = $2")
        .bind(user.id)
        .bind(&slug)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
