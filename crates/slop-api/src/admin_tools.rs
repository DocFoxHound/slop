// SPDX-License-Identifier: AGPL-3.0-or-later
//! Staff fixes to what people wrote, and freezing an app by hand (built 2026-10-22).
//!
//! - **Profiles:** an account's display name and profile picture, and a creative's People page (name, headline,
//!   about, location, links). For an impersonating name, a slur in a headline, a scam link: the
//!   smallest fix instead of hiding the whole page. Teams likewise (name and about).
//! - **Freezing:** stops an app taking new releases while staff look into it (frozen.rs is the
//!   automatic freeze when SLAPorSLOP loses its repository). It stays in the store and installed copies
//!   keep working; the daily repository check never lifts it, and it never takes the app out of
//!   the store on its own. Taking it out is delisting (admin.rs).
//!
//! Every fix needs a reason, which the person is told with what changed, and goes in the audit log
//! with the old values. Like every admin route, these take an admin API token too, so an agent can
//! make them (agent.rs lists them as MCP tools).

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    catalog::validate_text,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    staff::Perm,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/users/{id}/profile",
            get(profile).patch(edit_profile),
        )
        .route("/admin/teams/{slug}", patch(edit_team))
        .route("/admin/apps/{slug}/freeze", post(freeze))
        .route("/admin/apps/{slug}/unfreeze", post(unfreeze))
}

fn reason_of(reason: &str) -> ApiResult<&str> {
    text_of("reason", reason)
}

/// A reason or note for the person, named by its field.
fn text_of<'a>(field: &'static str, value: &'a str) -> ApiResult<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 2000 {
        return Err(ApiError::invalid(
            field,
            "say why, for the person, in up to 2000 characters",
        ));
    }
    Ok(value)
}

/// One line of text (names, headlines).
fn line(field: &'static str, value: &str, max: usize, allow_empty: bool) -> ApiResult<String> {
    let v = validate_text(field, value, max, allow_empty)?;
    if v.contains(['\n', '\r', '\t']) {
        return Err(ApiError::invalid(field, "keep it to one line"));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// Profiles

#[derive(Serialize, sqlx::FromRow)]
struct Creative {
    handle: String,
    name: String,
    headline: String,
    about_md: String,
    location: String,
    links: Value,
    hidden_reason: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct TeamRow {
    slug: String,
    name: String,
    kind: String,
    about: String,
    role: String,
    hidden_reason: Option<String>,
}

#[derive(Serialize)]
struct Profile {
    display_name: String,
    /// Their profile picture from a linked sign-in (avatars.rs); null: the letter.
    avatar_url: Option<String>,
    /// Their People page, if they have one.
    creative: Option<Creative>,
    /// Teams they run (owner or admin), which staff can fix from here.
    teams: Vec<TeamRow>,
}

/// What staff can fix about an account, as it is now.
async fn profile(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Profile>> {
    admin.need(Perm::Moderate)?;
    let display_name: String = sqlx::query_scalar("select display_name from users where id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let creative = sqlx::query_as(
        "select handle, name, headline, about_md, location, links, hidden_reason from creatives
         where user_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let teams = sqlx::query_as(
        "select t.slug, t.name, t.kind, t.about, m.role, t.hidden_reason from team_members m
         join teams t on t.id = m.team_id where m.user_id = $1 and m.role in ('owner', 'admin')
         order by t.name",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Profile {
        display_name,
        avatar_url: crate::avatars::url_of(&state, id).await?,
        creative,
        teams,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreativeEdit {
    name: Option<String>,
    headline: Option<String>,
    about_md: Option<String>,
    location: Option<String>,
    /// true takes every link off their People page.
    #[serde(default)]
    remove_links: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileEdit {
    display_name: Option<String>,
    creative: Option<CreativeEdit>,
    /// true takes their profile picture off (back to the letter).
    #[serde(default)]
    remove_picture: bool,
    /// Why, for the person.
    reason: String,
}

async fn edit_profile(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<ProfileEdit>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Moderate)?;
    let reason = reason_of(&b.reason)?;
    let mut tx = state.db.begin().await?;
    let name_now: String =
        sqlx::query_scalar("select display_name from users where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError::NotFound)?;
    let mut changed = serde_json::Map::new();
    let mut fields: Vec<&str> = Vec::new();
    if let Some(v) = &b.display_name {
        let v = crate::auth::validate_display_name(v)?;
        if v != name_now {
            sqlx::query("update users set display_name = $2 where id = $1")
                .bind(id)
                .bind(&v)
                .execute(&mut *tx)
                .await?;
            changed.insert("display_name".into(), json!({ "from": name_now, "to": v }));
            fields.push("display name");
        }
    }
    if let Some(c) = &b.creative {
        let now: Creative = sqlx::query_as(
            "select handle, name, headline, about_md, location, links, hidden_reason from creatives
             where user_id = $1 for update",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::invalid("creative", "they have no People page"))?;
        // (column, label, old, new)
        let mut sets: Vec<(&str, &str, String, String)> = Vec::new();
        if let Some(v) = &c.name {
            sets.push((
                "name",
                "People name",
                now.name.clone(),
                line("name", v, 60, false)?,
            ));
        }
        if let Some(v) = &c.headline {
            sets.push((
                "headline",
                "headline",
                now.headline.clone(),
                line("headline", v, 140, true)?,
            ));
        }
        if let Some(v) = &c.about_md {
            sets.push((
                "about_md",
                "about",
                now.about_md.clone(),
                validate_text("about_md", v, 6000, true)?,
            ));
        }
        if let Some(v) = &c.location {
            sets.push((
                "location",
                "location",
                now.location.clone(),
                line("location", v, 60, true)?,
            ));
        }
        for (column, label, from, to) in sets {
            if from == to {
                continue;
            }
            let sql = match column {
                "name" => "update creatives set name = $2, updated_at = now() where user_id = $1",
                "headline" => {
                    "update creatives set headline = $2, updated_at = now() where user_id = $1"
                }
                "about_md" => {
                    "update creatives set about_md = $2, updated_at = now() where user_id = $1"
                }
                _ => "update creatives set location = $2, updated_at = now() where user_id = $1",
            };
            sqlx::query(sql)
                .bind(id)
                .bind(&to)
                .execute(&mut *tx)
                .await?;
            changed.insert(column.into(), json!({ "from": from, "to": to }));
            fields.push(label);
        }
        if c.remove_links && now.links.as_array().is_some_and(|l| !l.is_empty()) {
            sqlx::query("update creatives set links = '[]', updated_at = now() where user_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            changed.insert("links".into(), json!({ "from": now.links, "to": [] }));
            fields.push("links");
        }
    }
    if b.remove_picture
        && let Some(from) = crate::avatars::remove(&mut tx, id).await?
    {
        changed.insert("picture".into(), json!({ "from": from, "to": null }));
        fields.push("picture");
    }
    if fields.is_empty() {
        return Err(ApiError::invalid("reason", "nothing changed"));
    }
    notify::send(
        &mut tx,
        Notice {
            user_id: id,
            category: "moderation",
            title: "SLAPorSLOP changed your profile".into(),
            body: format!("Changed: {}.\n\n{reason}", fields.join(", ")),
            link: "/account".into(),
            collapse_key: None,
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "user.profile_edited",
        &id.to_string(),
        json!({ "changed": changed, "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(json!({ "changed": fields })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamEdit {
    name: Option<String>,
    about: Option<String>,
    reason: String,
}

/// Fixes a team's name or description. Its owners and admins are told.
async fn edit_team(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<TeamEdit>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Moderate)?;
    let reason = reason_of(&b.reason)?;
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("select id, name, about from teams where slug = $1 for update")
            .bind(&slug)
            .fetch_optional(&mut *tx)
            .await?;
    let (team_id, name, about) = row.ok_or(ApiError::NotFound)?;
    let mut changed = serde_json::Map::new();
    let mut fields: Vec<&str> = Vec::new();
    if let Some(v) = &b.name {
        let v = line("name", v, 60, false)?;
        if v != name {
            sqlx::query("update teams set name = $2 where id = $1")
                .bind(team_id)
                .bind(&v)
                .execute(&mut *tx)
                .await?;
            changed.insert("name".into(), json!({ "from": name, "to": v }));
            fields.push("name");
        }
    }
    if let Some(v) = &b.about {
        let v = validate_text("about", v, 2000, true)?;
        if v != about {
            sqlx::query("update teams set about = $2 where id = $1")
                .bind(team_id)
                .bind(&v)
                .execute(&mut *tx)
                .await?;
            changed.insert("about".into(), json!({ "from": about, "to": v }));
            fields.push("description");
        }
    }
    if fields.is_empty() {
        return Err(ApiError::invalid("reason", "nothing changed"));
    }
    let runners: Vec<Uuid> = sqlx::query_scalar(
        "select user_id from team_members where team_id = $1 and role in ('owner', 'admin')",
    )
    .bind(team_id)
    .fetch_all(&mut *tx)
    .await?;
    for user_id in runners {
        notify::send(
            &mut tx,
            Notice {
                user_id,
                category: "teams",
                title: "SLAPorSLOP changed your team's page".into(),
                body: format!("Changed: {}.\n\n{reason}", fields.join(", ")),
                link: format!("/teams/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        "team.edited",
        &format!("team {slug}"),
        json!({ "changed": changed, "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(json!({ "changed": fields })))
}

// ---------------------------------------------------------------------------------------------
// Freezing

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreezeBody {
    /// Why, for the maker: what's wrong and what would fix it.
    reason: String,
}

/// Stops an app taking new releases until staff lift it. An app the automatic check froze
/// becomes staff's to lift.
async fn freeze(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<FreezeBody>,
) -> ApiResult<Json<Option<crate::frozen::Frozen>>> {
    admin.need(Perm::Moderate)?;
    let reason = reason_of(&b.reason)?;
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "select id, maker_id, title, frozen_by from apps where slug = $1 for update",
    )
    .bind(&slug)
    .fetch_optional(&mut *tx)
    .await?;
    let (app_id, maker, title, by) = row.ok_or(ApiError::NotFound)?;
    if by.is_some() {
        return Err(ApiError::Conflict(
            "staff already froze it; lift that first to change the reason".into(),
        ));
    }
    sqlx::query(
        "update apps set frozen_at = coalesce(frozen_at, now()), frozen_reason = $2, frozen_by = $3
         where id = $1",
    )
    .bind(app_id)
    .bind(reason)
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await?;
    notify::send(
        &mut tx,
        Notice {
            user_id: maker,
            category: "apps",
            title: format!("SLAPorSLOP froze {title}"),
            body: format!(
                "{title} can't take new releases for now: {reason}\n\nIt stays in the store and \
                 copies people installed keep working. SLAPorSLOP lifts the freeze once it's dealt with."
            ),
            link: format!("/publish/{slug}/manage"),
            collapse_key: Some(format!("frozen:{app_id}")),
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "app.frozen",
        &slug,
        json!({ "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(crate::frozen::status(&state.db, app_id).await?))
}

/// (id, maker, title, repository, public, frozen, frozen by)
type FreezeNow = (Uuid, Uuid, String, String, bool, bool, Option<String>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnfreezeBody {
    /// For the record, and the maker is told.
    note: String,
}

/// Lifts a staff freeze. For the automatic freeze it checks the repository again now, which is
/// the only thing that lifts it. Returns the freeze still in place, if any.
async fn unfreeze(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<UnfreezeBody>,
) -> ApiResult<Json<Option<crate::frozen::Frozen>>> {
    admin.need(Perm::Moderate)?;
    let note = text_of("note", &b.note)?;
    let row: Option<FreezeNow> = sqlx::query_as(
        "select id, maker_id, title, repo_url, visibility = 'public', frozen_at is not null, frozen_by
         from apps where slug = $1",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await?;
    let (app_id, maker, title, repo, public, frozen, by) = row.ok_or(ApiError::NotFound)?;
    if !frozen {
        return Err(ApiError::Conflict("it isn't frozen".into()));
    }
    let mut tx = state.db.begin().await?;
    if by.is_some() {
        // Back to normal; the next daily check freezes it again if SLAPorSLOP can't read the repository.
        sqlx::query(
            "update apps set frozen_at = null, frozen_reason = null, frozen_by = null,
                    repo_checked_at = null where id = $1",
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
                body: format!("{title} takes new releases again.\n\n{note}"),
                link: format!("/publish/{slug}/manage"),
                collapse_key: Some(format!("frozen:{app_id}")),
            },
        )
        .await?;
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        if by.is_some() {
            "app.unfrozen"
        } else {
            "app.repo_rechecked"
        },
        &slug,
        json!({ "note": note }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    if by.is_none() {
        let access = crate::frozen::check_app(&state, app_id, &repo, public).await;
        crate::frozen::apply(&state, app_id, access)
            .await
            .map_err(ApiError::Internal)?;
    }
    Ok(Json(crate::frozen::status(&state.db, app_id).await?))
}
