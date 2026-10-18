//! Catalog: categories, tags, store listings and makers' app drafts (DESIGN §5, §8).
//!
//! Makers can create and edit drafts. Moving a draft into the store happens through Publishing
//! (repo link, build, scans), which isn't built yet.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    config::CatalogRules,
    error::{ApiError, ApiResult, is_unique_violation},
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/categories", get(list_categories))
        .route("/apps", get(list_apps).post(create_app))
        .route("/apps/{slug}", get(get_app).patch(update_app))
        .route("/me/apps", get(my_apps))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_kind", rename_all = "snake_case")]
pub enum AppKind {
    Game,
    App,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_stage", rename_all = "snake_case")]
pub enum AppStage {
    ComingSoon,
    Beta,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_status", rename_all = "snake_case")]
pub enum AppStatus {
    Draft,
    Listed,
    Delisted,
}

// ---------------------------------------------------------------------------------------------
// Categories

#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct TagRef {
    slug: String,
    name: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Category {
    slug: String,
    name: String,
    description: String,
    #[sqlx(json)]
    tags: Vec<TagRef>,
}

async fn list_categories(State(state): State<AppState>) -> ApiResult<Json<Vec<Category>>> {
    let categories = sqlx::query_as(
        "select c.slug, c.name, c.description,
                coalesce((select json_agg(json_build_object('slug', t.slug, 'name', t.name) order by t.name)
                          from tags t where t.category_id = c.id), '[]') as tags
         from categories c order by c.position",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(categories))
}

// ---------------------------------------------------------------------------------------------
// Store listing and search

/// Appends the conditions that put an app in the store for this viewer: listed, allowed by the
/// viewer's age policy, and not past its stage deadline (DESIGN §8: missed deadline = delisted).
fn push_store_visible(qb: &mut QueryBuilder<Postgres>, rules: CatalogRules, policy: ViewerPolicy) {
    qb.push("a.status = 'listed' and a.min_age <= ")
        .push_bind(policy.max_min_age)
        .push(" and not ((a.stage = 'coming_soon' and a.stage_entered_at < now() - make_interval(days => ")
        .push_bind(rules.coming_soon_days)
        .push(")) or (a.stage = 'beta' and a.stage_entered_at < now() - make_interval(days => ")
        .push_bind(rules.beta_days)
        .push(")))");
}

/// Columns shared by every app summary query. `a` is `apps`.
fn push_summary_columns(qb: &mut QueryBuilder<Postgres>, rules: CatalogRules) {
    qb.push(
        "select a.slug, a.title, a.summary, a.kind, a.stage, a.status, a.min_age, a.updated_at,
                coalesce(array(select t.slug from app_tags at join tags t on t.id = at.tag_id
                               where at.app_id = a.id order by t.slug), '{}') as tags,
                (a.stage <> 'released' and a.last_commit_at is not null
                 and a.last_commit_at < now() - make_interval(days => ",
    )
    .push_bind(rules.dead_after_days)
    .push(")) as is_dead from apps a");
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AppSummary {
    slug: String,
    title: String,
    summary: String,
    kind: AppKind,
    stage: AppStage,
    #[serde(skip_serializing_if = "is_listed")]
    status: AppStatus,
    min_age: i16,
    tags: Vec<String>,
    /// No commits for a while in Coming soon or Beta. Shown, but never recommended.
    is_dead: bool,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

fn is_listed(status: &AppStatus) -> bool {
    *status == AppStatus::Listed
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sort {
    Recent,
    Title,
    Relevance,
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    tag: Option<String>,
    category: Option<String>,
    stage: Option<AppStage>,
    kind: Option<AppKind>,
    sort: Option<Sort>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct AppPage {
    apps: Vec<AppSummary>,
    next_offset: Option<i64>,
}

const MAX_PAGE: i64 = 50;

async fn list_apps(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<AppPage>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let rules = state.config.catalog;
    let limit = query.limit.unwrap_or(24).clamp(1, MAX_PAGE);
    let offset = query.offset.unwrap_or(0).clamp(0, 10_000);
    let search = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    if search.is_some_and(|q| q.chars().count() > 100) {
        return Err(ApiError::invalid("q", "search is too long"));
    }

    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, rules);
    qb.push(" where ");
    push_store_visible(&mut qb, rules, policy);
    if let Some(q) = search {
        qb.push(" and a.search @@ websearch_to_tsquery('english', ")
            .push_bind(q)
            .push(")");
    }
    if let Some(tag) = &query.tag {
        qb.push(" and exists (select 1 from app_tags at join tags t on t.id = at.tag_id where at.app_id = a.id and t.slug = ")
            .push_bind(tag)
            .push(")");
    }
    if let Some(category) = &query.category {
        qb.push(
            " and exists (select 1 from app_tags at join tags t on t.id = at.tag_id
                          join categories c on c.id = t.category_id where at.app_id = a.id and c.slug = ",
        )
        .push_bind(category)
        .push(")");
    }
    if let Some(stage) = query.stage {
        qb.push(" and a.stage = ").push_bind(stage);
    }
    if let Some(kind) = query.kind {
        qb.push(" and a.kind = ").push_bind(kind);
    }
    match (
        query.sort.unwrap_or(if search.is_some() {
            Sort::Relevance
        } else {
            Sort::Recent
        }),
        search,
    ) {
        (Sort::Relevance, Some(q)) => {
            qb.push(" order by ts_rank(a.search, websearch_to_tsquery('english', ")
                .push_bind(q)
                .push(")) desc, a.updated_at desc");
        }
        (Sort::Title, _) => {
            qb.push(" order by lower(a.title), a.slug");
        }
        _ => {
            qb.push(" order by a.updated_at desc, a.slug");
        }
    }
    // Fetch one extra row to know whether there's another page.
    qb.push(" limit ")
        .push_bind(limit + 1)
        .push(" offset ")
        .push_bind(offset);

    let mut apps: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
    let next_offset = (apps.len() as i64 > limit).then_some(offset + limit);
    apps.truncate(limit as usize);
    Ok(Json(AppPage { apps, next_offset }))
}

// ---------------------------------------------------------------------------------------------
// Store page

#[derive(Serialize)]
struct Maker {
    display_name: String,
}

#[derive(Serialize)]
struct AppDetail {
    #[serde(flatten)]
    summary: AppSummary,
    description_md: String,
    repo_url: String,
    license_spdx: String,
    maker: Maker,
    #[serde(with = "time::serde::rfc3339")]
    stage_entered_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_commit_at: Option<OffsetDateTime>,
    tag_details: Vec<TagRef>,
    is_mine: bool,
}

#[derive(sqlx::FromRow)]
struct AppExtra {
    id: Uuid,
    maker_id: Uuid,
    description_md: String,
    repo_url: String,
    license_spdx: String,
    maker_name: String,
    stage_entered_at: OffsetDateTime,
    last_commit_at: Option<OffsetDateTime>,
    store_visible: bool,
}

/// Loads an app's store page. Makers always see their own apps; everyone else only sees apps
/// that are in the store for them.
async fn load_detail(
    state: &AppState,
    slug: &str,
    user: Option<&CurrentUser>,
    policy: ViewerPolicy,
) -> ApiResult<AppDetail> {
    let rules = state.config.catalog;
    let mut qb = QueryBuilder::new(
        "select a.id, a.maker_id, a.description_md, a.repo_url, a.license_spdx, u.display_name as maker_name,
                a.stage_entered_at, a.last_commit_at, (",
    );
    push_store_visible(&mut qb, rules, policy);
    qb.push(") as store_visible from apps a join users u on u.id = a.maker_id where a.slug = ")
        .push_bind(slug);
    let extra: AppExtra = qb
        .build_query_as()
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    let is_mine = user.is_some_and(|u| u.id == extra.maker_id);
    if !extra.store_visible && !is_mine {
        return Err(ApiError::NotFound);
    }

    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, rules);
    qb.push(" where a.id = ").push_bind(extra.id);
    let summary: AppSummary = qb.build_query_as().fetch_one(&state.db).await?;
    let tag_details = sqlx::query_as(
        "select t.slug, t.name from app_tags at join tags t on t.id = at.tag_id where at.app_id = $1 order by t.slug",
    )
    .bind(extra.id)
    .fetch_all(&state.db)
    .await?;

    Ok(AppDetail {
        summary,
        description_md: extra.description_md,
        repo_url: extra.repo_url,
        license_spdx: extra.license_spdx,
        maker: Maker {
            display_name: extra.maker_name,
        },
        stage_entered_at: extra.stage_entered_at,
        last_commit_at: extra.last_commit_at,
        tag_details,
        is_mine,
    })
}

async fn get_app(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<AppDetail>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    Ok(Json(
        load_detail(&state, &slug, user.as_ref(), policy).await?,
    ))
}

async fn my_apps(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<AppSummary>>> {
    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, state.config.catalog);
    qb.push(" where a.maker_id = ")
        .push_bind(user.id)
        .push(" order by a.updated_at desc");
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}

// ---------------------------------------------------------------------------------------------
// Makers: create and edit drafts

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewApp {
    slug: String,
    title: String,
    summary: String,
    #[serde(default)]
    description_md: String,
    kind: AppKind,
    stage: AppStage,
    repo_url: String,
    license_spdx: String,
    min_age: i16,
    #[serde(default)]
    tags: Vec<String>,
}

/// Fields a maker can change. Slug, kind and stage aren't editable here: stage changes go
/// through Publishing, since each stage has requirements (DESIGN §8).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppChanges {
    title: Option<String>,
    summary: Option<String>,
    description_md: Option<String>,
    repo_url: Option<String>,
    license_spdx: Option<String>,
    min_age: Option<i16>,
    tags: Option<Vec<String>>,
}

const MAX_TAGS: usize = 10;

fn validate_slug(slug: &str) -> ApiResult<()> {
    let ok = (3..=60).contains(&slug.len())
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !ok {
        return Err(ApiError::invalid(
            "slug",
            "use 3 to 60 lowercase letters, numbers and single dashes",
        ));
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> ApiResult<String> {
    let value = value.trim();
    let len = value.chars().count();
    if (!allow_empty && len == 0) || len > max {
        let min = if allow_empty { 0 } else { 1 };
        return Err(ApiError::invalid(
            field,
            format!("use {min} to {max} characters"),
        ));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
    {
        return Err(ApiError::invalid(
            field,
            "contains characters that aren't allowed",
        ));
    }
    Ok(value.to_string())
}

/// Public `https` repo URLs only (DESIGN §8: GitHub, GitLab, Bitbucket or self-hosted).
fn validate_repo_url(value: &str) -> ApiResult<String> {
    let invalid = || ApiError::invalid("repo_url", "use the https:// address of the repository");
    let url = Url::parse(value.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_matches('/').is_empty()
        || url.as_str().len() > 300
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// An SPDX licence expression's characters. Whether the licence is OSI-approved is checked by the
/// licence scan (LEGAL-GUIDANCE IP-2), which runs against the repo itself.
fn validate_license(value: &str) -> ApiResult<String> {
    let value = value.trim();
    let ok = (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " .-+()".contains(c));
    if !ok {
        return Err(ApiError::invalid(
            "license_spdx",
            "use an SPDX licence ID, such as MIT or GPL-3.0-only",
        ));
    }
    Ok(value.to_string())
}

fn validate_min_age(value: i16) -> ApiResult<i16> {
    if ![0, 3, 7, 12, 16, 18].contains(&value) {
        return Err(ApiError::invalid("min_age", "use 0, 3, 7, 12, 16 or 18"));
    }
    Ok(value)
}

/// Resolves tag slugs to IDs, rejecting unknown tags and duplicates.
async fn resolve_tags(db: &sqlx::PgPool, tags: &[String]) -> ApiResult<Vec<i16>> {
    if tags.len() > MAX_TAGS {
        return Err(ApiError::invalid(
            "tags",
            format!("use at most {MAX_TAGS} tags"),
        ));
    }
    let mut unique = tags.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() != tags.len() {
        return Err(ApiError::invalid("tags", "a tag is listed twice"));
    }
    let found: Vec<(i16, String)> =
        sqlx::query_as("select id, slug from tags where slug = any($1)")
            .bind(&unique)
            .fetch_all(db)
            .await?;
    if let Some(missing) = unique.iter().find(|s| !found.iter().any(|(_, f)| f == *s)) {
        return Err(ApiError::invalid("tags", format!("unknown tag: {missing}")));
    }
    Ok(found.into_iter().map(|(id, _)| id).collect())
}

async fn set_tags(tx: &mut sqlx::PgConnection, app_id: Uuid, tag_ids: &[i16]) -> ApiResult<()> {
    sqlx::query("delete from app_tags where app_id = $1")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into app_tags (app_id, tag_id) select $1, unnest($2::smallint[])")
        .bind(app_id)
        .bind(tag_ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn create_app(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(req): Json<NewApp>,
) -> ApiResult<(StatusCode, Json<AppDetail>)> {
    validate_slug(&req.slug)?;
    let title = validate_text("title", &req.title, 80, false)?;
    let summary = validate_text("summary", &req.summary, 200, false)?;
    let description = validate_text("description_md", &req.description_md, 20_000, true)?;
    let repo_url = validate_repo_url(&req.repo_url)?;
    let license = validate_license(&req.license_spdx)?;
    let min_age = validate_min_age(req.min_age)?;
    let tag_ids = resolve_tags(&state.db, &req.tags).await?;

    let app_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, maker_id, repo_url, license_spdx, min_age)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(app_id)
    .bind(&req.slug)
    .bind(&title)
    .bind(&summary)
    .bind(&description)
    .bind(req.kind)
    .bind(req.stage)
    .bind(user.id)
    .bind(&repo_url)
    .bind(&license)
    .bind(min_age)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            ApiError::invalid("slug", "that address is taken")
        } else {
            err.into()
        }
    })?;
    set_tags(&mut tx, app_id, &tag_ids).await?;
    tx.commit().await?;

    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let detail = load_detail(&state, &req.slug, Some(&user), policy).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

async fn update_app(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(changes): Json<AppChanges>,
) -> ApiResult<Json<AppDetail>> {
    let mut tx = state.db.begin().await?;
    let owner: Option<(Uuid, Uuid)> =
        sqlx::query_as("select id, maker_id from apps where slug = $1 for update")
            .bind(&slug)
            .fetch_optional(&mut *tx)
            .await?;
    // Someone else's app looks the same as a missing one, so drafts can't be discovered.
    let app_id = match owner {
        Some((id, maker_id)) if maker_id == user.id => id,
        _ => return Err(ApiError::NotFound),
    };

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("update apps set updated_at = now()");
    if let Some(title) = &changes.title {
        qb.push(", title = ")
            .push_bind(validate_text("title", title, 80, false)?);
    }
    if let Some(summary) = &changes.summary {
        qb.push(", summary = ")
            .push_bind(validate_text("summary", summary, 200, false)?);
    }
    if let Some(description) = &changes.description_md {
        qb.push(", description_md = ").push_bind(validate_text(
            "description_md",
            description,
            20_000,
            true,
        )?);
    }
    if let Some(repo_url) = &changes.repo_url {
        qb.push(", repo_url = ")
            .push_bind(validate_repo_url(repo_url)?);
    }
    if let Some(license) = &changes.license_spdx {
        qb.push(", license_spdx = ")
            .push_bind(validate_license(license)?);
    }
    if let Some(min_age) = changes.min_age {
        qb.push(", min_age = ")
            .push_bind(validate_min_age(min_age)?);
    }
    qb.push(" where id = ").push_bind(app_id);
    qb.build().execute(&mut *tx).await?;

    if let Some(tags) = &changes.tags {
        let tag_ids = resolve_tags(&state.db, tags).await?;
        set_tags(&mut tx, app_id, &tag_ids).await?;
    }
    tx.commit().await?;

    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    Ok(Json(load_detail(&state, &slug, Some(&user), policy).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        for ok in ["abc", "pixel-dungeon-2", "a1b"] {
            assert!(validate_slug(ok).is_ok(), "{ok}");
        }
        for bad in ["ab", "Abc", "a--b", "-abc", "abc-", "a_b", "ünï"] {
            assert!(validate_slug(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repo_urls() {
        assert_eq!(
            validate_repo_url("https://github.com/a/b/").unwrap(),
            "https://github.com/a/b"
        );
        for bad in [
            "http://github.com/a/b",
            "https://github.com",
            "https://user:pw@github.com/a/b",
            "https://github.com/a/b?x=1",
            "git@github.com:a/b.git",
            "javascript:alert(1)",
        ] {
            assert!(validate_repo_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn licences() {
        assert!(validate_license("MIT OR Apache-2.0").is_ok());
        assert!(validate_license("GPL-3.0-or-later").is_ok());
        assert!(validate_license("<script>").is_err());
        assert!(validate_license("").is_err());
    }
}
