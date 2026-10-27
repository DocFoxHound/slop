// SPDX-License-Identifier: AGPL-3.0-or-later
//! What's left (`GET /v1/me/next-steps`, the `whats_left` tool for AI agents): every step still
//! open across the person's apps, their agreements and their People page, each saying who does it
//! — the person, their AI assistant, or SLAPorSLOP (a check or a review: nothing to do but wait) —
//! with a link to where it's done. An assistant ends a session with it, so nobody is left
//! wondering what's next.
//!
//! An agent's token for some apps sees only those; its People steps only when it handles People
//! jobs (maker_agents.rs).

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/next-steps", get(next_steps))
}

/// Who a step is for.
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Who {
    /// Only the person can: agreements, payments, choices that are theirs.
    You,
    /// Their AI assistant can, with SLAPorSLOP's tools (or the person on the website).
    Assistant,
    /// SLAPorSLOP is checking or reviewing: nothing to do but wait.
    Slaporslop,
}

#[derive(Serialize)]
struct Step {
    who: Who,
    /// The app it's about (its store address), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    what: String,
    /// Where it's done on the website, or where the assistant reads how.
    link: String,
    /// Worth doing, but it never blocks anything.
    optional: bool,
}

#[derive(Serialize)]
struct NextSteps {
    /// One sentence to pass on.
    summary: String,
    /// Things to do first (the person's, then the assistant's), then what's waiting on SLAPorSLOP,
    /// then the optional ones.
    steps: Vec<Step>,
}

#[derive(sqlx::FromRow)]
struct AppRow {
    slug: String,
    title: String,
    status: String,
    frozen_reason: Option<String>,
    frozen: bool,
    media_problem: Option<String>,
    media_pending: bool,
    has_icon: bool,
    public_requested: bool,
    latest_version: Option<String>,
    latest_status: Option<String>,
    latest_reason: Option<String>,
}

async fn next_steps(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<NextSteps>> {
    let site = |path: &str| {
        state
            .config
            .public_url
            .join(path)
            .map_or_else(|_| path.to_string(), |u| u.to_string())
    };
    let mut steps: Vec<Step> = Vec::new();
    let mut step = |who: Who, app: Option<&str>, what: String, link: String, optional: bool| {
        steps.push(Step {
            who,
            app: app.map(str::to_string),
            what,
            link,
            optional,
        });
    };
    let only_apps: Option<Vec<Uuid>> = user
        .agent
        .as_ref()
        .filter(|a| !a.app_ids.is_empty())
        .map(|a| a.app_ids.clone());

    let apps: Vec<AppRow> = sqlx::query_as(
        "select a.slug, a.title, a.status::text as status,
                a.frozen_reason, a.frozen_at is not null as frozen, a.media_problem,
                exists (select 1 from media_sets ms where ms.app_id = a.id and ms.status = 'pending') as media_pending,
                exists (select 1 from media_sets ms where ms.app_id = a.id and ms.icon is not null
                          and (ms.status = 'pending' or ms.id = a.media_set_id)) as has_icon,
                a.public_requested_at is not null as public_requested,
                r.version as latest_version, r.status::text as latest_status, r.status_reason as latest_reason
         from apps a
         left join lateral (select version, status, status_reason from releases
                            where app_id = a.id order by created_at desc limit 1) r on true
         where a.maker_id = $1 and ($2::uuid[] is null or a.id = any($2))
         order by a.updated_at desc",
    )
    .bind(user.id)
    .bind(&only_apps)
    .fetch_all(&state.db)
    .await?;

    // Agreements first: nothing else moves until they're accepted.
    if !apps.is_empty() || user.agent.is_some() {
        match crate::legal::require(&state.db, user.id, "publisher").await {
            Err(ApiError::TermsRequired(_)) => step(
                Who::You,
                None,
                "Accept the Publisher Agreement: SLAPorSLOP shows it on your AI agents page. Until then nothing can be published or changed.".into(),
                site("/account/agents"),
                false,
            ),
            other => other?,
        }
    }

    for a in &apps {
        let app = Some(a.slug.as_str());
        let name = &a.title;
        if a.status == "delisted" {
            step(
                Who::You,
                app,
                format!(
                    "{name} was taken out of the store: the decision, its reason and how to appeal are on your account's Moderation page."
                ),
                site("/account/moderation"),
                false,
            );
            continue;
        }
        if a.frozen {
            let reason = a.frozen_reason.as_deref().unwrap_or("it can't be read");
            step(
                Who::You,
                app,
                format!(
                    "SLAPorSLOP can't read {name}'s repository ({reason}). Make it public again or fix its address; no new release can go out until then."
                ),
                site(&format!("/publish/{}/manage", a.slug)),
                false,
            );
        }
        match (a.latest_status.as_deref(), a.latest_version.as_deref()) {
            (None, _) => step(
                Who::Assistant,
                app,
                format!(
                    "Submit {name}'s first release: a GitHub release (submit_github_release) or Slop Build's workflow. The guide's \"Submit a release\" says how."
                ),
                site("/publish/guide#3-submit-a-release"),
                false,
            ),
            (Some("fetching"), Some(v)) => step(
                Who::Slaporslop,
                app,
                format!(
                    "SLAPorSLOP is checking the files of {name} {v}: it takes minutes per file (list_releases shows how far it is)."
                ),
                site(&format!("/publish/{}/releases", a.slug)),
                false,
            ),
            (Some("awaiting_review"), Some(v)) => step(
                Who::Slaporslop,
                app,
                format!("{name} {v} passed SLAPorSLOP's checks and waits for a person's review."),
                site(&format!("/publish/{}/releases", a.slug)),
                false,
            ),
            (Some("rejected" | "failed"), Some(v)) => {
                let why = a.latest_reason.as_deref().unwrap_or("see the release");
                step(
                    Who::Assistant,
                    app,
                    format!(
                        "{name} {v} didn't go through ({why}). Fix what SLAPorSLOP found (get_fixes lists each problem with exact instructions), then submit a new release."
                    ),
                    site(&format!("/publish/{}/releases#fixes", a.slug)),
                    false,
                );
            }
            (Some("revoked"), Some(v)) => step(
                Who::You,
                app,
                format!(
                    "SLAPorSLOP withdrew {name} {v}: the reason is on the Releases page. Submit a fixed release, or appeal from your account's Moderation page."
                ),
                site(&format!("/publish/{}/releases", a.slug)),
                false,
            ),
            _ => {}
        }
        if let Some(problem) = &a.media_problem {
            step(
                Who::Assistant,
                app,
                format!(
                    "{name}'s pictures couldn't be read: {problem}. Fix .slop/media.json or the pictures, push, and run check_pictures."
                ),
                site("/publish/guide#pictures-and-videos"),
                false,
            );
        }
        if a.media_pending {
            step(
                Who::Slaporslop,
                app,
                format!("{name}'s new pictures wait for a person's review."),
                site(&format!("/publish/{}", a.slug)),
                false,
            );
        }
        if a.public_requested {
            step(
                Who::Slaporslop,
                app,
                format!("Making {name} public waits for a reviewer."),
                site(&format!("/publish/{}/manage", a.slug)),
                false,
            );
        }
        if !a.has_icon && a.media_problem.is_none() {
            step(
                Who::Assistant,
                app,
                format!(
                    "{name} has no icon yet: add a square 512×512 PNG to .slop/media.json (\"icon\": \"icon.png\") from the app's own art, then run check_pictures."
                ),
                site("/publish/guide#pictures-and-videos"),
                true,
            );
        }
    }

    // The People page, for the person and for agents that handle People jobs.
    let people = user.agent.as_ref().is_none_or(|a| a.people);
    let creative: Option<(bool, bool)> = if people {
        sqlx::query_as(
            "select taking_work, stripe_status = 'active' from creatives where user_id = $1",
        )
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
    } else {
        None
    };
    if let Some((taking_work, can_be_paid)) = creative {
        if let Err(ApiError::TermsRequired(_)) =
            crate::legal::require(&state.db, user.id, "people").await
        {
            step(
                Who::You,
                None,
                "Accept the new People Terms: SLAPorSLOP shows them in your People page's editor."
                    .into(),
                site("/people/me"),
                false,
            );
        }
        let status = crate::people::page_status(&state.db, user.id).await?;
        match status.state {
            "draft" => step(
                Who::You,
                None,
                match &status.problem {
                    Some(p) => format!("Your People page needs a portfolio that can be read: {p}. Your AI assistant can fix it with the portfolio guide; then press Check my portfolio now."),
                    None => "Add your portfolio to your People page: your AI assistant can build it with the portfolio guide (/portfolio.md); then press Check my portfolio now.".into(),
                },
                site("/people/me#portfolio"),
                false,
            ),
            "waiting" => step(
                Who::Slaporslop,
                None,
                "Your portfolio waits for a person's review; your People page goes public when it's approved.".into(),
                site("/people/me#portfolio"),
                false,
            ),
            "turned_down" => step(
                Who::You,
                None,
                format!(
                    "Your portfolio wasn't approved: {}. Fix it (your AI assistant can help), then press Check my portfolio now.",
                    status.reason.as_deref().unwrap_or("see your People page")
                ),
                site("/people/me#portfolio"),
                false,
            ),
            "hidden" => step(
                Who::You,
                None,
                "Your People page is hidden: why, and how to appeal, is on your account's Moderation page.".into(),
                site("/account/moderation"),
                false,
            ),
            _ => {
                if taking_work && !can_be_paid {
                    step(
                        Who::You,
                        None,
                        "Set up payments (Stripe) on your People page's editor, so makers can hire you.".into(),
                        site("/people/me"),
                        false,
                    );
                }
            }
        }
    }

    // The person's first, then the assistant's, then what's waiting; optional ones last.
    let rank = |s: &Step| {
        (
            s.optional,
            match s.who {
                Who::You => 0,
                Who::Assistant => 1,
                Who::Slaporslop => 2,
            },
        )
    };
    steps.sort_by_key(rank);
    let count = |who: Who| steps.iter().filter(|s| s.who == who && !s.optional).count();
    let (you, assistant, waiting) = (
        count(Who::You),
        count(Who::Assistant),
        count(Who::Slaporslop),
    );
    let things = |n: usize| {
        if n == 1 {
            "1 thing".to_string()
        } else {
            format!("{n} things")
        }
    };
    let mut parts = Vec::new();
    if you > 0 {
        parts.push(format!("{} for you", things(you)));
    }
    if assistant > 0 {
        parts.push(format!("{} for your AI assistant", things(assistant)));
    }
    if waiting > 0 {
        parts.push(format!("{} waiting on SLAPorSLOP", things(waiting)));
    }
    let summary = if parts.is_empty() {
        if apps.is_empty() && creative.is_none() {
            "Nothing yet: no apps and no People page. To publish, create a store page (create_store_page, or Publish → New app); to offer services, make a People page (People → Offer your services).".to_string()
        } else {
            "Nothing left to do: everything is done or in the store.".to_string()
        }
    } else {
        let optional = steps.iter().filter(|s| s.optional).count();
        format!(
            "{}{}.",
            join(&parts),
            if optional > 0 {
                format!(", and {optional} optional")
            } else {
                String::new()
            }
        )
    };
    Ok(Json(NextSteps { summary, steps }))
}

/// "a, b and c".
fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
