// SPDX-License-Identifier: AGPL-3.0-or-later
//! Results on GitHub (owner decision 2026-11-10, docs/DESIGN.md §8): SLAPorSLOP's result for each
//! release, posted as a commit status named "SLAPorSLOP" on the release's commit, for apps whose
//! maker switched it on (the Manage page, or their AI agent when asked). Off for the whole server
//! unless SLOP_GITHUB_STATUSES is set and the GitHub App is configured, and never in simulation:
//! then the route isn't there (404 like any unknown address), the Manage answer has no
//! `commit_status` and nothing is posted.
//!
//! Only with the repository's installation token (`github_app::installation_only`): never the
//! shared token, never a maker's stored token. The body is four fixed fields: one of GitHub's four
//! states, one of seven fixed sentences (`wanted`), the name SLAPorSLOP and the app's public page.
//! Never the reason, the findings, a staff note or `status_reason`.
//!
//! The rule:
//!
//! > While it is switched on and the app is in the store, every state is posted. When it is
//! > switched off or the app is out of the store, exactly one thing is still posted: if the last
//! > thing SLAPorSLOP posted on a commit was "Passed" and no release on that commit is approved any
//! > more, the commit's current state replaces it. Never for a private app. Nothing at all while
//! > the server switch is off or in simulation.
//!
//! Only releases sent after the maker switched it on get a status; a commit that already has a
//! row stays followed. Several releases can share a commit while GitHub keeps one status per
//! commit and name, so one speaks for it: the newest that isn't withdrawn, else the newest.
//!
//! The worker (`run`) is never told about a change: each round it compares each followed commit's
//! speaking release with what was last posted (`commit_statuses`), so every path that changes a
//! release's state (the reviewers' pages, the staff command line in its own process, the fetcher)
//! is covered without a hook. Rows are claimed with a 5-minute lease, renewed just before each
//! commit is posted, so several API copies post each state once. A refusal (3xx, or a 4xx other
//! than a rate limit or a first 401) stops that commit until the maker presses Check again; only
//! the correction of a "Passed" is tried again by itself, once a day, so a refusal can't leave it
//! on GitHub for good. A 401 is tried once more with a fresh token and then counts as a failed
//! try; GitHub errors and no answer are tried 5 times in all with longer waits; a missing
//! installation, a permission not accepted yet and rate limits make the whole app wait. GitHub's
//! answer body is never read. Log lines carry the repository and the HTTP code only, never a token.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::put,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgConnection, Postgres, QueryBuilder};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::CurrentUser,
    catalog,
    config::{CatalogRules, Config},
    error::{ApiError, ApiResult},
    github_app::{self, InstallationToken},
    limits,
    policy::ViewerPolicy,
    updates,
};

/// The status's name on GitHub.
const CONTEXT: &str = "SLAPorSLOP";
/// Commits claimed per round.
const BATCH: i64 = 5;
/// A claim older than this is free again (the API copy holding it stopped).
const LEASE_SECS: f64 = 300.0;
/// Failed tries before a commit stops (as notify.rs does for emails).
const MAX_ATTEMPTS: i32 = 5;
/// The wait after each failed try: 1 minute, 10 minutes, 1 hour, 6 hours.
const BACKOFF_SECS: [i64; 4] = [60, 600, 3600, 21600];
/// How long an app waits when its installation or the permission is missing.
const WAIT_SECS: u64 = 600;
/// A stopped correction of a "Passed" is tried again after this: a green status never outlives
/// the approval behind it, also when GitHub once refused or didn't answer.
const RETRY_CORRECTION_SECS: f64 = 86_400.0;
const POST_TIMEOUT: Duration = Duration::from_secs(15);
const EVERY: Duration = Duration::from_secs(30);

/// Whether results on GitHub can be offered on this server at all.
pub fn available(config: &Config) -> bool {
    config.releases.github_statuses && !config.simulation && config.releases.github_app.is_some()
}

/// (GitHub's state, the fixed sentence) for a release state. None for a state this code doesn't know.
pub(crate) fn wanted(status: &str) -> Option<(&'static str, &'static str)> {
    Some(match status {
        "fetching" => ("pending", "SLAPorSLOP is checking this release"),
        "awaiting_review" => (
            "pending",
            "Checked by SLAPorSLOP: waiting for a person's review",
        ),
        "approved" => ("success", "Passed SLAPorSLOP's checks"),
        "rejected" => ("failure", "Not approved by SLAPorSLOP"),
        "failed" => ("error", "SLAPorSLOP couldn't check this release"),
        // Replaces a green status, or "Passed" would stay on a release that no longer launches.
        "revoked" => ("failure", "Withdrawn by SLAPorSLOP after it was approved"),
        "withdrawn" => ("error", "Withdrawn by the maker before review"),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// What the maker sees

/// SLAPorSLOP's result on the app's GitHub commits, for the Manage page and the maker's agent.
#[derive(Serialize)]
pub struct CommitStatus {
    /// The maker has it switched on.
    on: bool,
    /// When they switched it on; null while off.
    #[serde(with = "time::serde::rfc3339::option")]
    since: Option<OffsetDateTime>,
    /// SLAPorSLOP's GitHub App covers the repository now.
    installed: bool,
    /// Whether GitHub lets the App write commit statuses there; null while off or not known yet.
    permission: Option<bool>,
    /// Where to install the App or review its request on GitHub.
    github_url: String,
    /// The app is in the store, so its results are posted.
    in_store: bool,
    /// GitHub's last refusal for this app while it's on: its HTTP status (0: no answer).
    problem: Option<Problem>,
    /// The newest status posted.
    last: Option<Last>,
}

#[derive(Serialize)]
struct Problem {
    code: i32,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

#[derive(Serialize)]
struct Last {
    commit_sha: String,
    /// Null when that release is gone.
    version: Option<String>,
    state: &'static str,
    description: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    posted_at: OffsetDateTime,
}

/// The panel's state, or None (no panel, no key) unless it can be offered for this app: switched
/// on, or a public app whose repository is on GitHub. Database only: no call to GitHub.
pub(crate) async fn for_app(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
) -> ApiResult<Option<CommitStatus>> {
    let Some(app) = &state.config.releases.github_app else {
        return Ok(None);
    };
    if !available(&state.config) {
        return Ok(None);
    }
    type Row = (
        bool,
        bool,
        Option<OffsetDateTime>,
        Option<bool>,
        Option<i32>,
        Option<OffsetDateTime>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select a.visibility = 'public', s.app_id is not null and s.switched_off_at is null,
                s.switched_on_at, s.permission, s.problem_code, s.problem_at
         from apps a left join app_commit_status s on s.app_id = a.id
         where a.id = $1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((public, on, since, permission, problem_code, problem_at)) = row else {
        return Ok(None);
    };
    let repo = slop_provenance::github_repo_path(repo_url);
    if !on && !(public && repo.is_some()) {
        return Ok(None);
    }
    let installed = match &repo {
        Some(repo) => github_app::installed(state, repo).await?,
        None => false,
    };
    let mut qb = QueryBuilder::new("select exists (select 1 from apps a where a.id = ");
    qb.push_bind(app_id).push(" and ");
    catalog::push_store_visible_as(&mut qb, state.config.catalog, ViewerPolicy::FULL_VIEW, "a");
    qb.push(")");
    let in_store: bool = qb.build_query_scalar().fetch_one(&state.db).await?;
    let last: Option<(String, String, OffsetDateTime, Option<String>)> = sqlx::query_as(
        "select p.commit_sha, p.posted_status::text, p.posted_at, r.version
         from commit_statuses p left join releases r on r.id = p.posted_release_id
         where p.app_id = $1 and p.posted_at is not null
         order by p.posted_at desc limit 1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let last = last.and_then(|(commit_sha, status, posted_at, version)| {
        let (state, description) = wanted(&status)?;
        Some(Last {
            commit_sha,
            version,
            state,
            description,
            posted_at,
        })
    });
    Ok(Some(CommitStatus {
        on,
        since: since.filter(|_| on),
        installed,
        permission: permission.filter(|_| on),
        github_url: github_app::install_url(app),
        in_store,
        problem: match (on, problem_code, problem_at) {
            (true, Some(code), Some(at)) => Some(Problem { code, at }),
            _ => None,
        },
        last,
    }))
}

// ---------------------------------------------------------------------------------------------
// Switching it on and off

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps/{slug}/commit-status", put(switch))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    on: bool,
}

/// The maker switches results on GitHub on or off; `on: true` while it's on is Check again. The
/// only call to GitHub is the token exchange: no status is posted here.
async fn switch(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(change): Json<Change>,
) -> ApiResult<Json<CommitStatus>> {
    if !available(&state.config) {
        return Err(ApiError::NotFound);
    }
    let (app_id, repo_url) = updates::owned(&state, &slug, &user).await?;

    // Switching off always works: after the app left the store, the App was uninstalled, or
    // whatever the repository is now.
    if !change.on {
        let mut tx = state.db.begin().await?;
        let off = sqlx::query(
            "update app_commit_status set switched_off_at = now()
             where app_id = $1 and switched_off_at is null",
        )
        .bind(app_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if off {
            audit(&mut tx, &user, &slug, app_id, false).await?;
        }
        tx.commit().await?;
        return for_app(&state, app_id, &repo_url)
            .await?
            .map(Json)
            .ok_or(ApiError::NotFound);
    }

    let public: bool = sqlx::query_scalar("select visibility = 'public' from apps where id = $1")
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    let repo = slop_provenance::github_repo_path(&repo_url)
        .filter(|_| public)
        .ok_or(ApiError::NotFound)?;
    limits::check(&state, user.id, limits::Action::CommitStatus)?;
    // The maker agreed to statuses on this repository: it must still be theirs.
    crate::repo_owner::check(&state, user.id, &repo_url, Some(app_id)).await?;
    if !github_app::installed(&state, &repo).await? {
        return Err(ApiError::Conflict(
            "SLAPorSLOP's GitHub App isn't installed on this repository: install it on GitHub, \
             then try again"
                .into(),
        ));
    }
    // A fresh token says whether the maker accepted the permission; a failure doesn't fail this.
    let permission = token_for(&state, &repo, true)
        .await
        .and_then(|t| t.statuses_write);

    let mut tx = state.db.begin().await?;
    // The address checked above must still be the app's: a store-page edit or an approved move in
    // between would leave it on for a repository nobody checked. One that comes after waits for
    // this lock, then switches it off again (`forget`).
    let repo_now: String = sqlx::query_scalar("select repo_url from apps where id = $1 for share")
        .bind(app_id)
        .fetch_one(&mut *tx)
        .await?;
    if !crate::manage::same_repo(&repo_now, &repo_url) {
        return Err(ApiError::Conflict(
            "This app's repository address just changed: try again".into(),
        ));
    }
    let was_off: Option<bool> = sqlx::query_scalar(
        "select switched_off_at is not null from app_commit_status where app_id = $1 for no key update",
    )
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let switched_on = match was_off {
        None => {
            sqlx::query(
                "insert into app_commit_status (app_id, permission, permission_checked_at)
                 values ($1, $2, now()) on conflict (app_id) do nothing",
            )
            .bind(app_id)
            .bind(permission)
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        }
        Some(true) => {
            sqlx::query(
                "update app_commit_status
                 set switched_on_at = now(), switched_off_at = null, permission = $2,
                     permission_checked_at = now(), wait_until = null, problem_code = null,
                     problem_at = null
                 where app_id = $1",
            )
            .bind(app_id)
            .bind(permission)
            .execute(&mut *tx)
            .await?;
            true
        }
        Some(false) => false,
    };
    if switched_on {
        audit(&mut tx, &user, &slug, app_id, true).await?;
    } else {
        // Check again.
        sqlx::query(
            "update app_commit_status
             set permission = $2, permission_checked_at = now(), wait_until = null,
                 problem_code = null, problem_at = null
             where app_id = $1",
        )
        .bind(app_id)
        .bind(permission)
        .execute(&mut *tx)
        .await?;
    }
    // Stopped and waiting commits are tried again: the newest 20, so one press can't queue a
    // flood of statuses.
    sqlx::query(
        "update commit_statuses set stopped_at = null, attempts = 0, next_attempt_at = null
         where (app_id, commit_sha) in (
             select app_id, commit_sha from commit_statuses
             where app_id = $1
               and (stopped_at is not null or next_attempt_at is not null or attempts > 0)
             order by stopped_at desc nulls last limit 20)",
    )
    .bind(app_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    for_app(&state, app_id, &repo_url)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn audit(
    tx: &mut PgConnection,
    user: &CurrentUser,
    slug: &str,
    app_id: Uuid,
    on: bool,
) -> ApiResult<()> {
    admin::audit(
        tx,
        &user.id.to_string(),
        "app.commit_status",
        &app_id.to_string(),
        json!({ "app": slug, "on": on, "by_agent": user.agent.is_some() }),
    )
    .await
    .map_err(ApiError::Internal)
}

/// Ends results on GitHub for an app whose repository address changes: the maker agreed to
/// statuses on one repository, so a status never goes to another without a new yes. Deletes both
/// tables' rows; when it was on, the audit log says so, by `actor`. Call it in the same
/// transaction as the change of `apps.repo_url`.
pub async fn forget(
    conn: &mut PgConnection,
    app_id: Uuid,
    slug: &str,
    actor: &str,
) -> anyhow::Result<()> {
    sqlx::query("delete from commit_statuses where app_id = $1")
        .bind(app_id)
        .execute(&mut *conn)
        .await?;
    let was_on: Option<bool> = sqlx::query_scalar(
        "delete from app_commit_status where app_id = $1 returning switched_off_at is null",
    )
    .bind(app_id)
    .fetch_optional(&mut *conn)
    .await?;
    if was_on == Some(true) {
        admin::audit(
            conn,
            actor,
            "app.commit_status",
            &app_id.to_string(),
            json!({ "app": slug, "on": false, "why": "repository changed" }),
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The worker

/// The worker: `run_once` again at once after a round that did something, else every 30 seconds.
pub async fn run(state: AppState) {
    loop {
        match run_once(&state).await {
            Ok(n) if n > 0 => continue,
            Ok(_) => {}
            Err(err) => tracing::error!(error = ?err, "results on GitHub"),
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// Posts what is due, once. Returns how many commits ended posted, stopped or backed off; a claim
/// only given back (the app waits, or there's nothing to say) doesn't count, so `run` can't spin.
/// Tests call this.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    if !available(&state.config) {
        return Ok(0);
    }
    let claimed = match claim(state).await {
        Ok(claimed) => claimed,
        // The app's row went while this round claimed one of its commits (a new repository
        // address, the account closing): the claim goes with it, and the next round takes the rest.
        Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut ended = 0;
    for (app_id, sha, claimed_at) in claimed {
        match post_due(state, app_id, &sha, claimed_at).await {
            Ok(true) => ended += 1,
            Ok(false) => {}
            // Left claimed: the lease runs out and it's tried again then.
            Err(err) => tracing::error!(error = ?err, %app_id, "results on GitHub"),
        }
    }
    Ok(ended)
}

/// The condition for posting on a commit now: the rule in the module comment. Over `s`
/// (app_commit_status), `p` (commit_statuses, may be missing) and `a` (apps); `commit` and
/// `newest_at` are static SQL for the commit and its newest release's time. The claim and the
/// second look both use it, so the two can never differ.
fn push_may_post(
    qb: &mut QueryBuilder<Postgres>,
    rules: CatalogRules,
    commit: &'static str,
    newest_at: &'static str,
) {
    // The usual case: switched on, in the store.
    qb.push(format!(
        "((s.switched_off_at is null
           and (p.app_id is not null or {newest_at} >= s.switched_on_at)
           and ("
    ));
    catalog::push_store_visible_as(qb, rules, ViewerPolicy::FULL_VIEW, "a");
    qb.push(")) or ");
    // The correction: a green status never outlives the approval behind it.
    push_correction(qb, commit);
    qb.push(")");
}

/// The correction's condition: the last thing posted on the commit was "Passed", the app is
/// public, and no release on the commit is approved any more.
fn push_correction(qb: &mut QueryBuilder<Postgres>, commit: &'static str) {
    qb.push(format!(
        "(p.posted_status = 'approved'
          and a.visibility = 'public'
          and not exists (select 1 from releases g
                          where g.app_id = a.id and g.commit_sha = {commit}
                            and g.status = 'approved'))"
    ));
}

/// Claims up to `BATCH` commits whose speaking release's state differs from what was posted,
/// with the time of each claim.
async fn claim(state: &AppState) -> sqlx::Result<Vec<(Uuid, String, OffsetDateTime)>> {
    let mut qb = QueryBuilder::new(
        "with speaking as (
             select distinct on (r.app_id, r.commit_sha)
                    r.app_id, r.commit_sha, r.status,
                    max(r.created_at) over (partition by r.app_id, r.commit_sha) as newest_at
             from releases r
             join app_commit_status s on s.app_id = r.app_id
             where (s.wait_until is null or s.wait_until <= now())
               and (s.switched_off_at is null
                    or exists (select 1 from commit_statuses g
                               where g.app_id = s.app_id and g.posted_status = 'approved'))
             order by r.app_id, r.commit_sha, (r.status = 'withdrawn'), r.created_at desc, r.id desc
         ), due as (
             select k.app_id, k.commit_sha
             from speaking k
             join app_commit_status s on s.app_id = k.app_id
             join apps a on a.id = k.app_id
             left join commit_statuses p on p.app_id = k.app_id and p.commit_sha = k.commit_sha
             where p.posted_status is distinct from k.status
               and (p.next_attempt_at is null or p.next_attempt_at <= now())
               and (p.claimed_at is null or p.claimed_at < now() - make_interval(secs => ",
    );
    qb.push_bind(LEASE_SECS)
        .push(
            "))
               and (p.stopped_at is null
                    or (p.stopped_at < now() - make_interval(secs => ",
        )
        .push_bind(RETRY_CORRECTION_SECS)
        .push(") and ");
    push_correction(&mut qb, "k.commit_sha");
    qb.push(")) and ");
    push_may_post(&mut qb, state.config.catalog, "k.commit_sha", "k.newest_at");
    qb.push(" order by k.newest_at limit ")
        .push_bind(BATCH)
        .push(
            ")
         insert into commit_statuses (app_id, commit_sha, claimed_at)
         select app_id, commit_sha, now() from due
         on conflict (app_id, commit_sha) do update set claimed_at = now()
             where commit_statuses.claimed_at is null
                or commit_statuses.claimed_at < now() - make_interval(secs => ",
        )
        .push_bind(LEASE_SECS)
        .push(") returning app_id, commit_sha, claimed_at");
    qb.build_query_as().fetch_all(&state.db).await
}

/// The newest release's time on the commit, for the second look (static SQL over `a` and `p`).
const NEWEST_AT: &str = "(select max(n.created_at) from releases n
                          where n.app_id = a.id and n.commit_sha = p.commit_sha)";

/// Posts one claimed commit's state if it is still due. Returns whether the commit ended posted,
/// stopped or backed off.
async fn post_due(
    state: &AppState,
    app_id: Uuid,
    sha: &str,
    claimed_at: OffsetDateTime,
) -> anyhow::Result<bool> {
    // The lease starts again now, so the commits after a slow one in the round keep theirs; a
    // claim another copy took over meanwhile is left to it.
    let renewed = sqlx::query(
        "update commit_statuses set claimed_at = now()
         where app_id = $1 and commit_sha = $2 and claimed_at = $3",
    )
    .bind(app_id)
    .bind(sha)
    .bind(claimed_at)
    .execute(&state.db)
    .await?
    .rows_affected()
        == 1;
    if !renewed {
        return Ok(false);
    }
    // Read the truth again: another copy may have posted it, or the maker switched it off.
    let mut qb = QueryBuilder::new(
        "select a.slug, a.repo_url, coalesce(s.wait_until > now(), false),
                r.id, r.status::text, p.posted_status::text, ",
    );
    push_may_post(&mut qb, state.config.catalog, "p.commit_sha", NEWEST_AT);
    qb.push(
        "
         from apps a
         join app_commit_status s on s.app_id = a.id
         join commit_statuses p on p.app_id = a.id and p.commit_sha = ",
    )
    .push_bind(sha)
    .push(
        "
         join lateral (select id, status from releases
                       where app_id = a.id and commit_sha = p.commit_sha
                       order by (status = 'withdrawn'), created_at desc, id desc limit 1) r on true
         where a.id = ",
    )
    .push_bind(app_id);
    type Row = (String, String, bool, Uuid, String, Option<String>, bool);
    let row: Option<Row> = qb.build_query_as().fetch_optional(&state.db).await?;
    let Some((slug, repo_url, waiting, release_id, status, posted, may_post)) = row else {
        give_back(&state.db, app_id, sha).await?;
        return Ok(false);
    };
    if waiting || !may_post || posted.as_deref() == Some(status.as_str()) {
        give_back(&state.db, app_id, sha).await?;
        return Ok(false);
    }
    let Some((gh_state, description)) = wanted(&status) else {
        // Given back, it would be claimed again every round: stop it instead. The maker can do
        // nothing about it, so the app shows no problem.
        tracing::warn!(state = %status, "results on GitHub: a release state with no status");
        sqlx::query(
            "update commit_statuses set stopped_at = now(), last_code = 0, claimed_at = null
             where app_id = $1 and commit_sha = $2",
        )
        .bind(app_id)
        .bind(sha)
        .execute(&state.db)
        .await?;
        return Ok(true);
    };

    // No installation or no token: the whole app waits. Not an attempt.
    let Some(repo) = slop_provenance::github_repo_path(&repo_url).filter(|r| plain_repo(r)) else {
        app_waits(&state.db, app_id, sha, WAIT_SECS).await?;
        return Ok(false);
    };
    let Some(mut token) = token_for(state, &repo, false).await else {
        app_waits(&state.db, app_id, sha, WAIT_SECS).await?;
        return Ok(false);
    };
    if token.statuses_write == Some(false) {
        // Permissions are fixed when a token is made: ask for a new one in case the maker
        // accepted since.
        let Some(again) = token_for(state, &repo, true).await else {
            app_waits(&state.db, app_id, sha, WAIT_SECS).await?;
            return Ok(false);
        };
        if again.statuses_write == Some(false) {
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "update app_commit_status
                 set permission = false, permission_checked_at = now(),
                     wait_until = now() + make_interval(secs => $2)
                 where app_id = $1",
            )
            .bind(app_id)
            .bind(WAIT_SECS as f64)
            .execute(&mut *tx)
            .await?;
            give_back(&mut *tx, app_id, sha).await?;
            tx.commit().await?;
            return Ok(false);
        }
        token = again;
    }

    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{repo}/statuses/{sha}"))?;
    let body = json!({
        "state": gh_state,
        "description": description,
        "context": CONTEXT,
        "target_url": format!("{}/app/{slug}", state.config.public_url.as_str().trim_end_matches('/')),
    });
    let mut answer = send(state, &url, &token.token, &body, &repo).await;
    if answer.code == 401 {
        // Once per commit: the cached token may have ended early. A second 401 is a failure.
        let Some(fresh) = token_for(state, &repo, true).await else {
            app_waits(&state.db, app_id, sha, WAIT_SECS).await?;
            return Ok(false);
        };
        answer = send(state, &url, &fresh.token, &body, &repo).await;
    }
    let code = answer.code;
    let now_unix = OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
    let wait = wait_for(
        code,
        answer.retry_after,
        answer.remaining,
        answer.reset,
        now_unix,
    );
    match (code, wait) {
        (200..=299, _) => {
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "update commit_statuses
                 set posted_status = $3::release_status, posted_release_id = $4, posted_at = now(),
                     attempts = 0, next_attempt_at = null, claimed_at = null, last_code = $5,
                     stopped_at = null
                 where app_id = $1 and commit_sha = $2",
            )
            .bind(app_id)
            .bind(sha)
            .bind(&status)
            .bind(release_id)
            .bind(i32::from(code))
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "update app_commit_status
                 set permission = true, permission_checked_at = now(), problem_code = null,
                     problem_at = null
                 where app_id = $1 and (permission is distinct from true or problem_code is not null)",
            )
            .bind(app_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(true)
        }
        (_, Some(secs)) => {
            tracing::info!(%repo, code, wait = secs, "results on GitHub: rate limited");
            app_waits(&state.db, app_id, sha, secs).await?;
            Ok(false)
        }
        // A refusal, or a moved repository (its new address is never read or followed): stop.
        (300..=499, None) if code != 401 => {
            tracing::info!(%repo, code, "results on GitHub: refused");
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "update commit_statuses set stopped_at = now(), last_code = $3, claimed_at = null
                 where app_id = $1 and commit_sha = $2",
            )
            .bind(app_id)
            .bind(sha)
            .bind(i32::from(code))
            .execute(&mut *tx)
            .await?;
            app_problem(&mut tx, app_id, code).await?;
            tx.commit().await?;
            Ok(true)
        }
        // GitHub's errors, no answer (0) and a second 401: try again later, 5 times in all.
        _ => {
            tracing::info!(%repo, code, "results on GitHub: failed");
            let mut tx = state.db.begin().await?;
            let backoff: Vec<f64> = BACKOFF_SECS.iter().map(|&s| s as f64).collect();
            let stopped: Option<bool> = sqlx::query_scalar(
                "update commit_statuses
                 set attempts = least(attempts + 1, $3), last_code = $4, claimed_at = null,
                     stopped_at = case when attempts + 1 >= $3 then now() end,
                     next_attempt_at = case when attempts + 1 < $3
                         then now() + make_interval(secs => ($5::float8[])[attempts + 1]) end
                 where app_id = $1 and commit_sha = $2
                 returning stopped_at is not null",
            )
            .bind(app_id)
            .bind(sha)
            .bind(MAX_ATTEMPTS)
            .bind(i32::from(code))
            .bind(&backoff)
            .fetch_optional(&mut *tx)
            .await?;
            if stopped == Some(true) {
                app_problem(&mut tx, app_id, code).await?;
            }
            tx.commit().await?;
            Ok(true)
        }
    }
}

/// The app's problem for the maker: GitHub's last refusal (0: no answer after every retry).
async fn app_problem(conn: &mut PgConnection, app_id: Uuid, code: u16) -> sqlx::Result<()> {
    sqlx::query(
        "update app_commit_status set problem_code = $2, problem_at = now() where app_id = $1",
    )
    .bind(app_id)
    .bind(i32::from(code))
    .execute(conn)
    .await?;
    Ok(())
}

/// Gives a claim back without anything else changing.
async fn give_back<'c>(
    conn: impl sqlx::PgExecutor<'c>,
    app_id: Uuid,
    sha: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "update commit_statuses set claimed_at = null where app_id = $1 and commit_sha = $2",
    )
    .bind(app_id)
    .bind(sha)
    .execute(conn)
    .await?;
    Ok(())
}

/// Nothing is posted for the app for `secs` seconds, and the claim is given back.
async fn app_waits(db: &sqlx::PgPool, app_id: Uuid, sha: &str, secs: u64) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "update app_commit_status set wait_until = now() + make_interval(secs => $2)
         where app_id = $1",
    )
    .bind(app_id)
    .bind(secs as f64)
    .execute(&mut *tx)
    .await?;
    give_back(&mut *tx, app_id, sha).await?;
    tx.commit().await
}

/// The repository's installation token, or None (logged) when there is none to have.
async fn token_for(state: &AppState, repo: &str, fresh: bool) -> Option<InstallationToken> {
    match github_app::installation_only(state, repo, fresh).await {
        Ok(Some(token)) => Some(token),
        Ok(None) => {
            tracing::info!(%repo, "results on GitHub: the App isn't installed");
            None
        }
        // A reqwest error can carry the address: only its status and whether it timed out.
        Err(err) => {
            match err.downcast_ref::<reqwest::Error>() {
                Some(e) => tracing::warn!(
                    %repo,
                    code = ?e.status().map(|s| s.as_u16()),
                    timeout = e.is_timeout(),
                    "results on GitHub: no installation token"
                ),
                None => {
                    tracing::warn!(%repo, error = %err, "results on GitHub: no installation token")
                }
            }
            None
        }
    }
}

/// What GitHub answered a status: its HTTP status (0: no answer) and the rate-limit headers.
struct Answer {
    code: u16,
    retry_after: Option<u64>,
    remaining: Option<u64>,
    reset: Option<u64>,
}

async fn send(
    state: &AppState,
    url: &Url,
    token: &str,
    body: &serde_json::Value,
    repo: &str,
) -> Answer {
    // `http` follows no redirects.
    let res = state
        .http
        .post(url.clone())
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "slop-api")
        .timeout(POST_TIMEOUT)
        .json(body)
        .send()
        .await;
    match res {
        Ok(res) => {
            let header = |name: &str| {
                res.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
            };
            // A code outside HTTP's range counts as no answer (what's recorded is 0 to 599).
            let code = res.status().as_u16();
            Answer {
                code: if (100..=599).contains(&code) { code } else { 0 },
                retry_after: header("retry-after"),
                remaining: header("x-ratelimit-remaining"),
                reset: header("x-ratelimit-reset"),
            }
        }
        Err(e) => {
            tracing::info!(
                %repo,
                code = ?e.status().map(|s| s.as_u16()),
                timeout = e.is_timeout(),
                "results on GitHub: no answer"
            );
            Answer {
                code: 0,
                retry_after: None,
                remaining: None,
                reset: None,
            }
        }
    }
}

/// How long GitHub asked SLAPorSLOP to wait, in seconds (1 to 3600), or None when the answer isn't
/// a rate limit: a 429, or a 403 with no requests left or a `retry-after` (as checks.rs
/// `ask_github`): `retry-after`, else until `x-ratelimit-reset`, else 10 minutes.
fn wait_for(
    status: u16,
    retry_after: Option<u64>,
    remaining: Option<u64>,
    reset: Option<u64>,
    now: u64,
) -> Option<u64> {
    let limited =
        status == 429 || (status == 403 && (remaining == Some(0) || retry_after.is_some()));
    limited.then(|| {
        retry_after
            .or_else(|| reset.map(|at| at.saturating_sub(now)))
            .unwrap_or(600)
            .clamp(1, 3600)
    })
}

/// Whether `owner/name` has no `.` or `..` part, which would change the statuses address.
fn plain_repo(repo: &str) -> bool {
    repo.split('/').all(|part| part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [&str; 7] = [
        "fetching",
        "awaiting_review",
        "approved",
        "rejected",
        "failed",
        "revoked",
        "withdrawn",
    ];

    #[test]
    fn every_release_state_has_a_status() {
        let banned = [
            "safe",
            "secure",
            "virus",
            "trusted",
            "verified",
            "guarantee",
        ];
        for status in STATES {
            let (state, sentence) = wanted(status).unwrap_or_else(|| panic!("{status}"));
            assert!(
                ["error", "failure", "pending", "success"].contains(&state),
                "{status}: {state}"
            );
            assert!(sentence.chars().count() <= 140, "{sentence}");
            for text in [sentence, CONTEXT] {
                let lower = text.to_lowercase();
                for word in banned {
                    assert!(!lower.contains(word), "{text} says {word}");
                }
            }
        }
        assert_eq!(wanted("approved").unwrap().0, "success");
        assert_eq!(wanted("revoked").unwrap().0, "failure");
        for unknown in ["", "listed", "Approved", "paused"] {
            assert_eq!(wanted(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn waits_grow_and_end() {
        assert_eq!(BACKOFF_SECS.len(), MAX_ATTEMPTS as usize - 1);
        assert!(BACKOFF_SECS.windows(2).all(|w| w[0] < w[1]));
        assert!(BACKOFF_SECS[0] > 0);
    }

    #[test]
    fn rate_limit_answers_say_how_long() {
        let now = 1_000_000;
        // Not a rate limit.
        assert_eq!(wait_for(201, None, None, None, now), None);
        assert_eq!(wait_for(403, None, Some(10), None, now), None);
        assert_eq!(wait_for(403, None, None, Some(now + 50), now), None);
        assert_eq!(wait_for(502, Some(30), None, None, now), None);
        // retry-after first, then the reset time, then ten minutes; 1 second to an hour.
        assert_eq!(
            wait_for(429, Some(120), None, Some(now + 50), now),
            Some(120)
        );
        assert_eq!(wait_for(403, Some(120), None, None, now), Some(120));
        assert_eq!(wait_for(403, None, Some(0), Some(now + 50), now), Some(50));
        assert_eq!(wait_for(429, None, None, None, now), Some(600));
        assert_eq!(wait_for(429, Some(0), None, None, now), Some(1));
        assert_eq!(wait_for(429, None, None, Some(now - 5), now), Some(1));
        assert_eq!(
            wait_for(403, None, Some(0), Some(now + 9000), now),
            Some(3600)
        );
    }

    #[test]
    fn statuses_go_only_to_plain_repositories() {
        assert!(plain_repo("octo/game"));
        assert!(plain_repo("octo/.github"));
        assert!(plain_repo("octo/game.js"));
        for bad in ["./game", "../game", "octo/..", "octo/.", "../.."] {
            assert!(!plain_repo(bad), "{bad}");
        }
    }
}
