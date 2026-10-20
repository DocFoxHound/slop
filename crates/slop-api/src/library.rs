//! A user's library (DESIGN §6). Adding is free and instant; installing happens on the device.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;
use time::OffsetDateTime;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/library", get(list))
        .route("/me/library/{slug}", put(add).delete(remove))
        .route("/library/updates", get(updates))
}

// ---------------------------------------------------------------------------------------------
// Updates for installed apps (DESIGN §6): what players get now, what's being checked, and the
// other builds of the same app to switch to. Works signed out: installed apps do too.

#[derive(Deserialize)]
struct UpdatesQuery {
    /// Comma-separated app addresses (the apps installed on this device).
    apps: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Serving {
    release_id: uuid::Uuid,
    version: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Pending {
    version: String,
    /// fetching (downloading, hashing and scanning) or awaiting_review.
    status: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Build {
    slug: String,
    title: String,
    /// The original app, not a community branch.
    official: bool,
    platforms: Vec<String>,
}

#[derive(Serialize)]
struct AppUpdate {
    slug: String,
    /// The release players get now; compare its ID with the installed one.
    latest: Option<Serving>,
    /// A newer release still being checked, if any: the "queued update".
    pending: Option<Pending>,
    /// The official app and its community builds in the store, when there's more than one.
    builds: Vec<Build>,
}

const MAX_UPDATE_CHECK: usize = 100;

async fn updates(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Query(query): Query<UpdatesQuery>,
) -> ApiResult<Json<Vec<AppUpdate>>> {
    let slugs: Vec<&str> = query
        .apps
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if slugs.len() > MAX_UPDATE_CHECK {
        return Err(ApiError::invalid(
            "apps",
            format!("check at most {MAX_UPDATE_CHECK} apps at once"),
        ));
    }
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let mut out = Vec::new();
    for slug in slugs {
        // Apps this viewer can't see are left out, as if they didn't exist.
        let Ok((app_id, _)) = catalog::visible_app(&state, slug, user.as_ref(), policy).await
        else {
            continue;
        };
        let latest: Option<Serving> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select r.id as release_id, r.version {} limit 1",
            crate::releases::SERVING
        )))
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?;
        let pending: Option<Pending> = sqlx::query_as(
            "select r.version, r.status::text as status from releases r
             where r.app_id = $1 and r.status::text in ('fetching', 'awaiting_review')
               and r.created_at > coalesce((select max(x.reviewed_at) from releases x
                                            where x.app_id = $1 and x.status = 'approved'), '-infinity')
             order by r.created_at desc limit 1",
        )
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?;
        // The family: the original (this app's parent, or itself) and its branches in the store.
        let mut qb = QueryBuilder::new(
            "select a.slug, a.title, a.parent_app_id is null as official, a.platforms from apps a
             where (a.id = coalesce((select parent_app_id from apps where id = ",
        );
        qb.push_bind(app_id)
            .push("), ")
            .push_bind(app_id)
            .push(") or a.parent_app_id = coalesce((select parent_app_id from apps where id = ")
            .push_bind(app_id)
            .push("), ")
            .push_bind(app_id)
            .push(")) and (a.id = ")
            .push_bind(app_id)
            .push(" or (");
        catalog::push_store_visible(&mut qb, state.config.catalog, policy);
        qb.push(")) order by a.parent_app_id is not null, a.title limit 20");
        let builds: Vec<Build> = qb.build_query_as().fetch_all(&state.db).await?;
        out.push(AppUpdate {
            slug: slug.to_string(),
            latest,
            pending,
            builds: if builds.len() > 1 { builds } else { Vec::new() },
        });
    }
    Ok(Json(out))
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
    catalog::push_summary_columns_with(&mut qb, &state.config, ", l.added_at");
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
