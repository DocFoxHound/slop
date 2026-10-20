//! The Featured carousel at the top of the front page (DESIGN §5): apps Slop staff pick, in
//! their order, each with one line saying why. Only apps in the store for the viewer show.

use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::{
    AppState,
    admin_web::AdminUser,
    auth::session::MaybeUser,
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

/// At most this many slides.
pub const MAX_FEATURED: usize = 8;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/featured", get(featured))
        .route("/admin/featured", get(admin_list).put(admin_set))
}

#[derive(Serialize, sqlx::FromRow)]
struct Slide {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    blurb: String,
}

async fn featured(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Slide>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, &state.config, ", f.blurb");
    qb.push(" join featured_apps f on f.app_id = a.id where ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by f.position limit ")
        .push_bind(MAX_FEATURED as i64);
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct AdminSlide {
    slug: String,
    title: String,
    blurb: String,
    /// Whether it's in the store for everyone; a slide that isn't simply doesn't show.
    listed: bool,
}

async fn admin_list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<AdminSlide>>> {
    Ok(Json(
        sqlx::query_as(
            "select a.slug, a.title, f.blurb, (a.status = 'listed' and a.visibility = 'public') as listed
             from featured_apps f join apps a on a.id = f.app_id order by f.position",
        )
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSlide {
    slug: String,
    #[serde(default)]
    blurb: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBody {
    apps: Vec<SetSlide>,
}

/// Replaces the carousel with these apps, in this order.
async fn admin_set(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SetBody>,
) -> ApiResult<Json<Vec<AdminSlide>>> {
    admin.may_decide()?;
    if body.apps.len() > MAX_FEATURED {
        return Err(ApiError::invalid(
            "apps",
            format!("feature at most {MAX_FEATURED} apps"),
        ));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from featured_apps")
        .execute(&mut *tx)
        .await?;
    let mut slugs = Vec::new();
    for (position, slide) in body.apps.iter().enumerate() {
        let blurb = slide.blurb.trim();
        if blurb.chars().count() > 160 {
            return Err(ApiError::invalid("blurb", "keep it to 160 characters"));
        }
        let app_id: Option<Uuid> = sqlx::query_scalar(
            "select id from apps where slug = $1 and parent_app_id is null and visibility = 'public'",
        )
        .bind(&slide.slug)
        .fetch_optional(&mut *tx)
        .await?;
        let app_id = app_id
            .ok_or_else(|| ApiError::invalid("apps", format!("no public app at {}", slide.slug)))?;
        sqlx::query(
            "insert into featured_apps (app_id, position, blurb, added_by) values ($1, $2, $3, $4)
             on conflict (app_id) do nothing",
        )
        .bind(app_id)
        .bind(position as i32)
        .bind(blurb)
        .bind(admin.user_id)
        .execute(&mut *tx)
        .await?;
        slugs.push(slide.slug.clone());
    }
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "featured.set",
        "front page",
        serde_json::json!({ "apps": slugs }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    admin_list(State(state), admin).await
}
