// SPDX-License-Identifier: AGPL-3.0-or-later
//! The beta's invite code (decided 2026-11-19). Anyone can sign up, donate, play, make store pages
//! and send releases through the checks; going public needs an invite code while SLAPorSLOP staff
//! have it switched on: publishing a release (publish.rs), putting an app in the store the first
//! time (manage.rs `list_app`) and asking for a private app to go public (frozen.rs). The routes
//! check it with `require`, not the functions behind them.
//!
//! - **One code** for everyone, set and changed by staff (Admin → Store). It isn't a secret: it
//!   only paces the beta, so staff change it when too many people have it.
//! - **Entered once:** an account that entered it (`users.publish_invited_at`) keeps going public
//!   after the code changes. Makers who had an app in the store, or a published release, before
//!   the code existed count as invited (migration 0103). Staff never need it.
//! - **Where to ask:** staff set the SLAPorSLOP Discord's invite link next to the code
//!   (`discord_url`, migration 0107); the closed-beta notice on the maker's pages links to it.
//! - **Only the person** enters it, signed in on the website (never an AI agent's token), at most
//!   `limits::Action::InviteCode` times an hour. A wrong code is a 422 on `code`.

use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin_web::AdminUser,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    limits,
};

/// The code's length when it's required.
const CODE_CHARS: std::ops::RangeInclusive<usize> = 4..=64;
/// The Discord link's longest.
const URL_CHARS: usize = 300;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/publish-invite", get(mine).post(enter))
        .route("/admin/publish-invite", get(admin_get).put(admin_set))
}

/// Refuses going public (409 `invite_required`) while the invite code is required and the user
/// hasn't entered it. Staff pass.
pub async fn require(db: &PgPool, user_id: Uuid) -> ApiResult<()> {
    let ok: bool = sqlx::query_scalar(
        "select coalesce((select not i.required or u.publish_invited_at is not null or u.is_admin
                          from publish_invite i, users u where u.id = $1), true)",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(ApiError::InviteRequired)
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Mine {
    /// Going public needs the code now.
    required: bool,
    /// This account can go public without entering it: it entered it before, or it's staff's.
    invited: bool,
    /// The SLAPorSLOP Discord, where makers ask for the code; empty when staff haven't set it.
    discord_url: String,
}

async fn load_mine(db: &PgPool, user_id: Uuid) -> sqlx::Result<Mine> {
    sqlx::query_as(
        "select i.required, (u.publish_invited_at is not null or u.is_admin) as invited, i.discord_url
         from publish_invite i, users u where u.id = $1",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

async fn mine(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    Ok(Json(load_mine(&state.db, user.id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enter {
    code: String,
}

/// The code as it's compared: spaces around it and letter case don't matter.
fn normalize(code: &str) -> String {
    code.trim().to_lowercase()
}

/// The person enters the invite code. Right: their account can go public from now on, whatever
/// the code becomes later.
async fn enter(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Enter>,
) -> ApiResult<Json<Mine>> {
    if user.agent.is_some() {
        return Err(ApiError::Refused(
            "The invite code is for the person, signed in on SLAPorSLOP: an AI agent can't enter it. Tell your maker to enter it on the website when they publish.",
        ));
    }
    limits::check(&state, user.id, limits::Action::InviteCode)?;
    let code: String = sqlx::query_scalar("select code from publish_invite")
        .fetch_one(&state.db)
        .await?;
    let given = normalize(&body.code);
    if given.is_empty() {
        return Err(ApiError::invalid("code", "Enter the invite code."));
    }
    if code.trim().is_empty() || given != normalize(&code) {
        return Err(ApiError::invalid(
            "code",
            "That isn't the invite code. Check it and try again.",
        ));
    }
    sqlx::query(
        "update users set publish_invited_at = coalesce(publish_invited_at, now()) where id = $1",
    )
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(Json(load_mine(&state.db, user.id).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct AdminView {
    required: bool,
    code: String,
    discord_url: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    /// Accounts that can go public without entering it again (entered it, or made apps public
    /// before it existed).
    invited: i64,
    /// Of those, the ones that entered it since these settings were last saved.
    invited_since_update: i64,
}

async fn admin_view(db: &PgPool) -> sqlx::Result<AdminView> {
    sqlx::query_as(
        "select i.required, i.code, i.discord_url, i.updated_at,
                (select count(*) from users where publish_invited_at is not null) as invited,
                (select count(*) from users where publish_invited_at >= i.updated_at) as invited_since_update
         from publish_invite i",
    )
    .fetch_one(db)
    .await
}

async fn admin_get(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<AdminView>> {
    Ok(Json(admin_view(&state.db).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBody {
    required: bool,
    #[serde(default)]
    code: String,
    /// The Discord's invite link (`https://…`); left out keeps it, empty clears it.
    #[serde(default)]
    discord_url: Option<String>,
}

/// Switches the code on or off, and sets it. Accounts that entered an earlier code keep going
/// public.
async fn admin_set(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SetBody>,
) -> ApiResult<Json<AdminView>> {
    admin.need(crate::staff::Perm::Review)?;
    let code = body.code.trim();
    if (body.required || !code.is_empty()) && !CODE_CHARS.contains(&code.chars().count()) {
        return Err(ApiError::invalid(
            "code",
            format!(
                "Make the code {} to {} characters.",
                CODE_CHARS.start(),
                CODE_CHARS.end()
            ),
        ));
    }
    let discord_url = body.discord_url.as_deref().map(str::trim);
    if let Some(url) = discord_url.filter(|u| !u.is_empty()) {
        let ok = url.chars().count() <= URL_CHARS
            && url::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some());
        if !ok {
            return Err(ApiError::invalid(
                "discord_url",
                "Give the Discord's invite link, starting https://.",
            ));
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update publish_invite set required = $1, code = $2, discord_url = coalesce($4, discord_url),
                updated_at = now(), updated_by = $3",
    )
    .bind(body.required)
    .bind(code)
    .bind(admin.user_id)
    .bind(discord_url)
    .execute(&mut *tx)
    .await?;
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "publish_invite.set",
        "publishing invite",
        json!({ "required": body.required, "code": code, "discord_url": discord_url }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(admin_view(&state.db).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_compare_without_case_or_spaces_around() {
        assert_eq!(normalize("  Beta-Fox "), normalize("beta-fox"));
        assert_ne!(normalize("beta fox"), normalize("betafox"));
    }
}
