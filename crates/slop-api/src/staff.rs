// SPDX-License-Identifier: AGPL-3.0-or-later
//! Staff roles and two-person approval (SECURITY §8.4).
//!
//! **Roles.** Every staff member (`users.is_admin`) has roles that say what they may do:
//!
//! | Role | May |
//! |---|---|
//! | owner | everything |
//! | moderator | review, reports and appeals, and sensitive actions |
//! | reviewer | review releases, changes, pictures, portfolios and listings |
//! | support | reports, appeals and People problems; no sensitive actions |
//! | finance | payments (nothing in the admin API needs it yet) |
//! | security | tokens, staff roles and the audit log |
//!
//! Staff without a role can only look. An API token is capped by its level as well
//! (admin_web.rs `AdminUser::need`).
//!
//! **Two people.** Sensitive actions (withdrawing a release, taking an app out of the store,
//! suspending or reinstating an account, changing staff roles) don't happen on one person's
//! word: they become a `staff_actions` request that needs `SLOP_APPROVALS_REQUIRED` different
//! people (the requester is the first). While Slop has one person it's 1, so the action happens
//! at once, but it's recorded the same way; set it to 2 when there's a second person. A token
//! counts as its admin, and approving takes a person signed in themselves (never a token), so an
//! agent can ask but never be the second person. Requests expire after three days.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    error::{ApiError, ApiResult},
};

/// Unapproved requests lapse after this long.
const EXPIRES_AFTER: time::Duration = time::Duration::days(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Moderator,
    Reviewer,
    Support,
    Finance,
    Security,
}

pub const ROLES: [Role; 6] = [
    Role::Owner,
    Role::Moderator,
    Role::Reviewer,
    Role::Support,
    Role::Finance,
    Role::Security,
];

/// What an admin request needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perm {
    /// Looking: every staff member.
    Read,
    /// Approving and rejecting releases, changes, pictures, portfolios, listings.
    Review,
    /// Reports, appeals, warnings, People problems, reading an account's details.
    Moderate,
    /// Withdrawing releases, delisting apps, suspending and reinstating accounts.
    Sensitive,
    /// Payments.
    Finance,
    /// Tokens, staff roles.
    Security,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Moderator => "moderator",
            Self::Reviewer => "reviewer",
            Self::Support => "support",
            Self::Finance => "finance",
            Self::Security => "security",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ROLES.into_iter().find(|r| r.name() == s)
    }

    pub fn allows(self, perm: Perm) -> bool {
        use Perm::*;
        match self {
            Self::Owner => true,
            Self::Moderator => matches!(perm, Read | Review | Moderate | Sensitive),
            Self::Reviewer => matches!(perm, Read | Review),
            Self::Support => matches!(perm, Read | Moderate),
            Self::Finance => matches!(perm, Read | Finance),
            Self::Security => matches!(perm, Read | Security),
        }
    }
}

pub async fn roles_of(db: &sqlx::PgPool, user_id: Uuid) -> sqlx::Result<Vec<Role>> {
    let names: Vec<String> =
        sqlx::query_scalar("select role from staff_roles where user_id = $1 order by role")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    Ok(names.iter().filter_map(|n| Role::parse(n)).collect())
}

// ---------------------------------------------------------------------------------------------
// Two-person approval

/// What happened to a sensitive action.
pub enum Outcome {
    /// Done now (enough approvals): what the action returned.
    Done(Value),
    /// Waiting for more people.
    Pending { id: Uuid, more_needed: i64 },
}

impl IntoResponse for Outcome {
    fn into_response(self) -> Response {
        match self {
            Self::Done(v) if v.is_null() => StatusCode::NO_CONTENT.into_response(),
            Self::Done(v) => Json(v).into_response(),
            Self::Pending { id, more_needed } => (
                StatusCode::ACCEPTED,
                Json(json!({
                    "pending_action": id,
                    "approvals_still_needed": more_needed,
                    "message": "Recorded. It happens once another staff member approves it in the admin queue.",
                })),
            )
                .into_response(),
        }
    }
}

/// Records a sensitive action and does it once enough people approved it (now, when one is
/// enough). `kind` and `payload` must be something `execute` knows.
pub async fn request(
    state: &AppState,
    admin: &AdminUser,
    kind: &str,
    summary: &str,
    payload: Value,
) -> ApiResult<Outcome> {
    let needed = i16::from(state.config.approvals_required.max(1));
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into staff_actions (id, kind, summary, payload, requested_by, requested_actor, approvals_needed)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(kind)
    .bind(summary)
    .bind(&payload)
    .bind(admin.user_id)
    .bind(&admin.actor)
    .bind(needed)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into staff_action_approvals (action_id, user_id, actor) values ($1, $2, $3)",
    )
    .bind(id)
    .bind(admin.user_id)
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "staff_action.request",
        &id.to_string(),
        json!({ "kind": kind, "summary": summary, "approvals_needed": needed }),
    )
    .await
    .map_err(ApiError::Internal)?;
    if needed > 1 {
        // Tell the others who could approve it.
        let others: Vec<Uuid> = sqlx::query_scalar(
            "select distinct u.id from users u join staff_roles r on r.user_id = u.id
             where u.is_admin and u.closed_at is null and u.id <> $1 and r.role in ('owner', 'moderator', 'security')",
        )
        .bind(admin.user_id)
        .fetch_all(&mut *tx)
        .await?;
        for other in others {
            crate::notify::send(
                &mut tx,
                crate::notify::Notice {
                    user_id: other,
                    category: "ops",
                    title: "A second approval is needed".into(),
                    body: summary.to_string(),
                    link: "/admin".into(),
                    collapse_key: Some(format!("staff-action:{id}")),
                },
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(Outcome::Pending {
            id,
            more_needed: i64::from(needed) - 1,
        });
    }
    tx.commit().await?;
    finish(state, id, kind, &payload, &admin.actor).await
}

/// Runs an approved action and records how it went. `actor` names everyone who approved it.
async fn finish(
    state: &AppState,
    id: Uuid,
    kind: &str,
    payload: &Value,
    actor: &str,
) -> ApiResult<Outcome> {
    let result = execute(state, kind, payload, actor).await;
    let (status, text) = match &result {
        Ok(v) => ("done", v.to_string()),
        Err(e) => ("failed", format!("{e:?}")),
    };
    sqlx::query(
        "update staff_actions set status = $2, result = $3, decided_at = now() where id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(text)
    .execute(&state.db)
    .await?;
    result.map(Outcome::Done)
}

#[derive(Deserialize)]
struct RevokePayload {
    release_id: Uuid,
    reason: String,
}

#[derive(Deserialize)]
struct DelistPayload {
    slug: String,
    rule: String,
    reason: String,
}

#[derive(Deserialize)]
struct RolePayload {
    user_id: Uuid,
    role: Role,
    grant: bool,
}

/// Carries out an approved action.
async fn execute(state: &AppState, kind: &str, payload: &Value, actor: &str) -> ApiResult<Value> {
    let bad =
        |e: serde_json::Error| ApiError::Internal(anyhow::anyhow!("stored action is damaged: {e}"));
    let refused = |e: anyhow::Error| ApiError::Conflict(format!("{e:#}"));
    match kind {
        "release.revoke" => {
            let p: RevokePayload = serde_json::from_value(payload.clone()).map_err(bad)?;
            admin::revoke(&state.db, p.release_id, actor, &p.reason)
                .await
                .map_err(refused)?;
            Ok(Value::Null)
        }
        "app.delist" => {
            let p: DelistPayload = serde_json::from_value(payload.clone()).map_err(bad)?;
            admin::delist_app(&state.db, &p.slug, actor, &p.rule, &p.reason)
                .await
                .map_err(refused)?;
            Ok(Value::Null)
        }
        "report.act" => crate::moderation::act_on_report_stored(state, payload, actor).await,
        "account.suspend" => crate::agent::suspend_stored(state, payload, actor).await,
        "account.lift" => crate::agent::lift_stored(state, payload, actor).await,
        "account.passkey_grace" => crate::agent::passkey_grace_stored(state, payload, actor).await,
        "staff.role" => {
            let p: RolePayload = serde_json::from_value(payload.clone()).map_err(bad)?;
            set_role(state, p.user_id, p.role, p.grant, actor).await?;
            Ok(Value::Null)
        }
        _ => Err(ApiError::Internal(anyhow::anyhow!(
            "unknown staff action {kind}"
        ))),
    }
}

/// Gives or takes a role. Giving one makes the account staff; taking the last one leaves it
/// staff that can only look (removing staff entirely is `slop-api admin revoke`).
async fn set_role(
    state: &AppState,
    user_id: Uuid,
    role: Role,
    grant: bool,
    actor: &str,
) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    let exists: Option<bool> =
        sqlx::query_scalar("select closed_at is null from users where id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists != Some(true) {
        return Err(ApiError::Conflict("no open account with that ID".into()));
    }
    if grant {
        sqlx::query("update users set is_admin = true where id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "insert into staff_roles (user_id, role, granted_by) values ($1, $2, $3) on conflict do nothing",
        )
        .bind(user_id)
        .bind(role.name())
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    } else {
        if role == Role::Owner {
            let owners: i64 = sqlx::query_scalar(
                "select count(*) from staff_roles r join users u on u.id = r.user_id
                 where r.role = 'owner' and u.is_admin and u.closed_at is null and r.user_id <> $1",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if owners == 0 {
                return Err(ApiError::Conflict("Slop needs at least one owner".into()));
            }
        }
        sqlx::query("delete from staff_roles where user_id = $1 and role = $2")
            .bind(user_id)
            .bind(role.name())
            .execute(&mut *tx)
            .await?;
    }
    admin::audit(
        &mut tx,
        actor,
        if grant {
            "staff.grant_role"
        } else {
            "staff.remove_role"
        },
        &user_id.to_string(),
        json!({ "role": role.name() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(())
}

/// Marks requests nobody approved in time.
async fn expire(db: &sqlx::PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "update staff_actions set status = 'expired', decided_at = now()
         where status = 'pending' and requested_at < $1",
    )
    .bind(OffsetDateTime::now_utc() - EXPIRES_AFTER)
    .execute(db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Endpoints

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/actions", get(list_actions))
        .route("/admin/actions/{id}/approve", post(approve))
        .route("/admin/actions/{id}/cancel", post(cancel))
        .route("/admin/staff", get(list_staff))
        .route("/admin/staff/{user_id}/roles", post(change_role))
}

#[derive(Serialize, sqlx::FromRow)]
struct ActionRow {
    id: Uuid,
    kind: String,
    summary: String,
    requested_actor: String,
    approvals_needed: i16,
    status: String,
    result: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    requested_at: OffsetDateTime,
    approvals: Vec<String>,
}

#[derive(Deserialize)]
struct ActionsQuery {
    /// pending (the default), or all for the recent history.
    status: Option<String>,
}

#[derive(Serialize)]
struct Actions {
    /// How many people each sensitive action needs (SLOP_APPROVALS_REQUIRED).
    approvals_required: u8,
    actions: Vec<ActionRow>,
}

async fn list_actions(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<ActionsQuery>,
) -> ApiResult<Json<Actions>> {
    expire(&state.db).await?;
    let all = q.status.as_deref() == Some("all");
    let actions = sqlx::query_as(
        "select a.id, a.kind, a.summary, a.requested_actor, a.approvals_needed, a.status, a.result, a.requested_at,
                coalesce(array(select p.actor from staff_action_approvals p where p.action_id = a.id order by p.approved_at), '{}') as approvals
         from staff_actions a where $1 or a.status = 'pending' order by a.requested_at desc limit 100",
    )
    .bind(all)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Actions {
        approvals_required: state.config.approvals_required.max(1),
        actions,
    }))
}

/// The permission approving an action needs.
fn perm_for(kind: &str) -> Perm {
    if kind == "staff.role" {
        Perm::Security
    } else {
        Perm::Sensitive
    }
}

async fn approve(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Outcome> {
    expire(&state.db).await?;
    let mut tx = state.db.begin().await?;
    let row: Option<(String, Value, String, i16)> = sqlx::query_as(
        "select kind, payload, status, approvals_needed from staff_actions where id = $1 for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, payload, status, needed) = row.ok_or(ApiError::NotFound)?;
    admin.need(perm_for(&kind))?;
    // A person approves, signed in themselves (recently, or confirmed with a passkey): never an
    // agent's token.
    if admin.token.is_some() {
        return Err(ApiError::Forbidden);
    }
    admin.confirm_recent(&state.db).await?;
    if status != "pending" {
        return Err(ApiError::Conflict(format!("that request is {status}")));
    }
    let inserted = sqlx::query(
        "insert into staff_action_approvals (action_id, user_id, actor) values ($1, $2, $3) on conflict do nothing",
    )
    .bind(id)
    .bind(admin.user_id)
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict(
            "you already approved this: it needs someone else".into(),
        ));
    }
    let approvers: Vec<String> = sqlx::query_scalar(
        "select actor from staff_action_approvals where action_id = $1 order by approved_at",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "staff_action.approve",
        &id.to_string(),
        json!({ "kind": kind }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    let count = approvers.len() as i64;
    if count < i64::from(needed) {
        return Ok(Outcome::Pending {
            id,
            more_needed: i64::from(needed) - count,
        });
    }
    let actor = approvers.join(", approved by ");
    finish(&state, id, &kind, &payload, &actor).await
}

async fn cancel(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "select kind, requested_by from staff_actions where id = $1 and status = 'pending'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (kind, by) = row.ok_or(ApiError::NotFound)?;
    // The requester can withdraw their own; anyone who could approve it can turn it down.
    if by != Some(admin.user_id) {
        admin.need(perm_for(&kind))?;
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("update staff_actions set status = 'cancelled', decided_at = now(), result = $2 where id = $1")
        .bind(id)
        .bind(format!("cancelled by {}", admin.actor))
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "staff_action.cancel",
        &id.to_string(),
        json!({ "kind": kind }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
struct StaffMember {
    id: Uuid,
    display_name: String,
    roles: Vec<String>,
    suspended: bool,
}

async fn list_staff(State(state): State<AppState>, admin: AdminUser) -> ApiResult<Json<Value>> {
    let staff: Vec<StaffMember> = sqlx::query_as(
        "select u.id, u.display_name,
                coalesce(array(select r.role from staff_roles r where r.user_id = u.id order by r.role), '{}') as roles,
                u.suspended_at is not null as suspended
         from users u where u.is_admin and u.closed_at is null order by u.display_name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "staff": staff,
        "roles": ROLES.iter().map(|r| r.name()).collect::<Vec<_>>(),
        "my_roles": admin.roles.iter().map(|r| r.name()).collect::<Vec<_>>(),
        "approvals_required": state.config.approvals_required.max(1),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeRole {
    role: Role,
    /// true gives the role (making the account staff), false takes it away.
    grant: bool,
}

async fn change_role(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(user_id): Path<Uuid>,
    Json(b): Json<ChangeRole>,
) -> ApiResult<Outcome> {
    admin.need(Perm::Security)?;
    admin.signed_in()?;
    admin.confirm_recent(&state.db).await?;
    // Only an owner gives or takes the owner role.
    if b.role == Role::Owner && !admin.roles.contains(&Role::Owner) {
        return Err(ApiError::Forbidden);
    }
    let name: String = sqlx::query_scalar("select display_name from users where id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let summary = format!(
        "{} the {} role {} {name}",
        if b.grant { "Give" } else { "Take away" },
        b.role.name(),
        if b.grant { "to" } else { "from" }
    );
    request(
        &state,
        &admin,
        "staff.role",
        &summary,
        json!({ "user_id": user_id, "role": b.role, "grant": b.grant }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_allow_what_the_table_says() {
        assert!(Role::Owner.allows(Perm::Security));
        assert!(Role::Moderator.allows(Perm::Sensitive));
        assert!(!Role::Reviewer.allows(Perm::Moderate));
        assert!(!Role::Support.allows(Perm::Sensitive));
        assert!(Role::Support.allows(Perm::Moderate));
        assert!(!Role::Finance.allows(Perm::Review));
        assert!(!Role::Security.allows(Perm::Review));
        for r in ROLES {
            assert!(r.allows(Perm::Read));
            assert_eq!(Role::parse(r.name()), Some(r));
        }
    }
}
