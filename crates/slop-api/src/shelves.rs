// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shelves staff make for the front page (DESIGN §5), beside the automatic ones: a title, a line
//! saying what ties the apps together, and the apps in staff's order. Each also has its own page
//! (`/shelf/{slug}`); one that's off the front page is only there. Staff can also hide any of the
//! automatic shelves (`BUILT_IN`), for example if "Fresh slop" is being used to pile on someone.
//!
//! Only apps in the store for the viewer show, as everywhere; a shelf with none left doesn't show.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    auth::session::MaybeUser,
    catalog::{self, AppSummary, validate_text},
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
    staff::Perm,
};

/// The automatic front-page shelves (web/src/pages/Store.tsx), which staff may hide.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("popular", "Popular this month"),
    ("upcoming", "Coming soon & in beta"),
    ("categories", "Browse by category"),
    ("not_slop", "Most Slaps right now"),
    ("barely_seen", "New and barely seen"),
    ("glow_ups", "Glow-ups"),
    ("community", "Community branches worth installing"),
    ("fresh", "New & updated"),
    ("fresh_slop", "Fresh slop"),
];

const MAX_SHELVES: i64 = 20;
const MAX_APPS: usize = 24;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/shelves", get(front))
        .route("/shelves/staff/{slug}", get(one))
        .route("/admin/shelves", get(admin_list).post(create))
        .route("/admin/shelves/order", put(reorder))
        .route("/admin/shelves/{slug}", put(update).delete(remove))
        .route("/admin/front-shelves/{key}", post(set_built_in))
}

// ---------------------------------------------------------------------------------------------
// What visitors see

#[derive(Serialize)]
struct Shelf {
    slug: String,
    title: String,
    blurb: String,
    apps: Vec<AppSummary>,
}

#[derive(Serialize)]
struct Front {
    /// Staff shelves for the front page, in order; ones with no app the viewer can see are left out.
    shelves: Vec<Shelf>,
    /// Automatic shelves staff hid (keys from `BUILT_IN`).
    hidden: Vec<String>,
}

async fn shelf_apps(
    state: &AppState,
    policy: ViewerPolicy,
    shelf_id: Uuid,
) -> ApiResult<Vec<AppSummary>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, &state.config, "");
    qb.push(" join store_shelf_apps s on s.app_id = a.id where s.shelf_id = ")
        .push_bind(shelf_id)
        .push(" and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by s.position");
    Ok(qb.build_query_as().fetch_all(&state.db).await?)
}

async fn front(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Front>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "select id, slug, title, blurb from store_shelves where on_front order by position, created_at",
    )
    .fetch_all(&state.db)
    .await?;
    let mut shelves = Vec::new();
    for (id, slug, title, blurb) in rows {
        let apps = shelf_apps(&state, policy, id).await?;
        if !apps.is_empty() {
            shelves.push(Shelf {
                slug,
                title,
                blurb,
                apps,
            });
        }
    }
    let hidden = sqlx::query_scalar("select key from hidden_front_shelves order by key")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(Front { shelves, hidden }))
}

async fn one(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Shelf>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("select id, title, blurb from store_shelves where slug = $1")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?;
    let (id, title, blurb) = row.ok_or(ApiError::NotFound)?;
    Ok(Json(Shelf {
        apps: shelf_apps(&state, policy, id).await?,
        slug,
        title,
        blurb,
    }))
}

// ---------------------------------------------------------------------------------------------
// Staff

#[derive(Serialize, sqlx::FromRow)]
struct AdminApp {
    slug: String,
    title: String,
    /// In the store for everyone; one that isn't simply doesn't show.
    listed: bool,
}

#[derive(Serialize)]
struct AdminShelf {
    slug: String,
    title: String,
    blurb: String,
    on_front: bool,
    apps: Vec<AdminApp>,
}

#[derive(Serialize)]
struct BuiltIn {
    key: &'static str,
    title: &'static str,
    hidden: bool,
}

#[derive(Serialize)]
struct AdminShelves {
    shelves: Vec<AdminShelf>,
    built_in: Vec<BuiltIn>,
}

async fn admin_list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<AdminShelves>> {
    let rows: Vec<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "select id, slug, title, blurb, on_front from store_shelves order by position, created_at",
    )
    .fetch_all(&state.db)
    .await?;
    let mut shelves = Vec::new();
    for (id, slug, title, blurb, on_front) in rows {
        let apps = sqlx::query_as(
            "select a.slug, a.title, (a.status = 'listed' and a.visibility = 'public') as listed
             from store_shelf_apps s join apps a on a.id = s.app_id where s.shelf_id = $1 order by s.position",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        shelves.push(AdminShelf {
            slug,
            title,
            blurb,
            on_front,
            apps,
        });
    }
    let hidden: Vec<String> = sqlx::query_scalar("select key from hidden_front_shelves")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(AdminShelves {
        shelves,
        built_in: BUILT_IN
            .iter()
            .map(|(key, title)| BuiltIn {
                key,
                title,
                hidden: hidden.iter().any(|h| h == key),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShelfBody {
    /// Its address (`/shelf/{slug}`): lowercase letters, digits and dashes. Only when creating.
    slug: Option<String>,
    title: String,
    #[serde(default)]
    blurb: String,
    #[serde(default = "yes")]
    on_front: bool,
    /// App addresses, in order.
    apps: Vec<String>,
}

fn yes() -> bool {
    true
}

fn valid_slug(slug: &str) -> bool {
    (2..=40).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

async fn lock_shelves(tx: &mut sqlx::PgConnection) -> sqlx::Result<()> {
    sqlx::query("select pg_advisory_xact_lock(hashtextextended('store_shelves', 0))")
        .execute(tx)
        .await?;
    Ok(())
}

/// Writes a shelf's fields and apps; returns the app addresses for the audit log.
async fn write(tx: &mut sqlx::PgConnection, id: Uuid, b: &ShelfBody) -> ApiResult<Vec<String>> {
    let title = validate_text("title", &b.title, 60, false)?;
    let blurb = validate_text("blurb", &b.blurb, 160, true)?;
    if title.contains('\n') || blurb.contains('\n') {
        return Err(ApiError::invalid("title", "one line each"));
    }
    if b.apps.len() > MAX_APPS {
        return Err(ApiError::invalid(
            "apps",
            format!("put at most {MAX_APPS} apps on a shelf"),
        ));
    }
    sqlx::query(
        "update store_shelves set title = $2, blurb = $3, on_front = $4, updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(&title)
    .bind(&blurb)
    .bind(b.on_front)
    .execute(&mut *tx)
    .await?;
    sqlx::query("delete from store_shelf_apps where shelf_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let mut slugs = Vec::new();
    for (position, slug) in b.apps.iter().enumerate() {
        let slug = slug.trim();
        let app_id: Option<Uuid> =
            sqlx::query_scalar("select id from apps where slug = $1 and visibility = 'public'")
                .bind(slug)
                .fetch_optional(&mut *tx)
                .await?;
        let app_id =
            app_id.ok_or_else(|| ApiError::invalid("apps", format!("no public app at {slug}")))?;
        let added = sqlx::query(
            "insert into store_shelf_apps (shelf_id, app_id, position) values ($1, $2, $3)
             on conflict do nothing",
        )
        .bind(id)
        .bind(app_id)
        .bind(position as i32)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // A repeated app is on the shelf (and in the log) once, at its first place.
        if added > 0 {
            slugs.push(slug.to_string());
        }
    }
    Ok(slugs)
}

async fn create(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(b): Json<ShelfBody>,
) -> ApiResult<Json<AdminShelves>> {
    admin.need(Perm::Review)?;
    let slug = b.slug.as_deref().unwrap_or("").trim().to_string();
    if !valid_slug(&slug) {
        return Err(ApiError::invalid(
            "slug",
            "2 to 40 lowercase letters, digits and single dashes",
        ));
    }
    // `/admin/shelves/order` is the reordering, so a shelf there could never be changed.
    if slug == "order" {
        return Err(ApiError::invalid("slug", "that address is taken"));
    }
    let mut tx = state.db.begin().await?;
    // One shelf change at a time, so two can't both take the last place.
    lock_shelves(&mut tx).await?;
    let count: i64 = sqlx::query_scalar("select count(*) from store_shelves")
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_SHELVES {
        return Err(ApiError::invalid(
            "slug",
            format!("SLAPorSLOP keeps at most {MAX_SHELVES} shelves: remove one first"),
        ));
    }
    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "insert into store_shelves (id, slug, title, position, created_by)
         values ($1, $2, '-', (select coalesce(max(position), 0) + 1 from store_shelves), $3)
         on conflict (slug) do nothing",
    )
    .bind(id)
    .bind(&slug)
    .bind(admin.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::invalid(
            "slug",
            "there's already a shelf at that address",
        ));
    }
    let apps = write(&mut tx, id, &b).await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "shelf.created",
        &format!("shelf {slug}"),
        json!({ "title": b.title.trim(), "on_front": b.on_front, "apps": apps }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    admin_list(State(state), admin).await
}

async fn update(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<ShelfBody>,
) -> ApiResult<Json<AdminShelves>> {
    admin.need(Perm::Review)?;
    if b.slug.as_deref().is_some_and(|s| s != slug) {
        return Err(ApiError::invalid("slug", "a shelf's address can't change"));
    }
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("select id from store_shelves where slug = $1 for update")
        .bind(&slug)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
    let apps = write(&mut tx, id, &b).await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "shelf.changed",
        &format!("shelf {slug}"),
        json!({ "title": b.title.trim(), "on_front": b.on_front, "apps": apps }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    admin_list(State(state), admin).await
}

async fn remove(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    admin.need(Perm::Review)?;
    let mut tx = state.db.begin().await?;
    let gone = sqlx::query("delete from store_shelves where slug = $1")
        .bind(&slug)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(ApiError::NotFound);
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        "shelf.removed",
        &format!("shelf {slug}"),
        json!({}),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    /// Every shelf's address, in the new order.
    slugs: Vec<String>,
}

async fn reorder(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(b): Json<Order>,
) -> ApiResult<Json<AdminShelves>> {
    admin.need(Perm::Review)?;
    let mut tx = state.db.begin().await?;
    lock_shelves(&mut tx).await?;
    // Every shelf once: a partial or mistyped list would leave two shelves at one place.
    let mut all: Vec<String> = sqlx::query_scalar("select slug from store_shelves")
        .fetch_all(&mut *tx)
        .await?;
    let mut given = b.slugs.clone();
    all.sort();
    given.sort();
    if all != given {
        return Err(ApiError::invalid(
            "slugs",
            "name every shelf once, in the new order",
        ));
    }
    for (position, slug) in b.slugs.iter().enumerate() {
        sqlx::query("update store_shelves set position = $2 where slug = $1")
            .bind(slug)
            .bind(position as i32)
            .execute(&mut *tx)
            .await?;
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        "shelf.reordered",
        "front page",
        json!({ "order": b.slugs }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    admin_list(State(state), admin).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuiltInBody {
    hidden: bool,
}

/// Hides or shows one of the automatic front-page shelves.
async fn set_built_in(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(key): Path<String>,
    Json(b): Json<BuiltInBody>,
) -> ApiResult<Json<AdminShelves>> {
    admin.need(Perm::Review)?;
    if !BUILT_IN.iter().any(|(k, _)| *k == key) {
        return Err(ApiError::NotFound);
    }
    let mut tx = state.db.begin().await?;
    if b.hidden {
        sqlx::query(
            "insert into hidden_front_shelves (key, hidden_by) values ($1, $2) on conflict do nothing",
        )
        .bind(&key)
        .bind(admin.user_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query("delete from hidden_front_shelves where key = $1")
            .bind(&key)
            .execute(&mut *tx)
            .await?;
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        if b.hidden {
            "shelf.built_in_hidden"
        } else {
            "shelf.built_in_shown"
        },
        &format!("shelf {key}"),
        json!({}),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    admin_list(State(state), admin).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_addresses() {
        assert!(valid_slug("cozy-games"));
        assert!(valid_slug("top10"));
        assert!(!valid_slug("Cozy"));
        assert!(!valid_slug("-cozy"));
        assert!(!valid_slug("cozy--games"));
        assert!(!valid_slug("x"));
    }
}
