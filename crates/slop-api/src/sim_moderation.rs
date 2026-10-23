// SPDX-License-Identifier: AGPL-3.0-or-later
//! The sandbox's reports, moderation decisions and appeals (moderation.rs), and the agreements
//! simulated users accepted (legal.rs). Decisions go through the real `moderation::decide`, so the
//! admin panel, statements of reasons and appeals can be tried with realistic data.

use serde::Serialize;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{moderation, sim::Rng};

const MODERATOR: &str = "Sim moderator (sandbox)";

/// What reporters write, per reason.
fn details(reason: &str) -> &'static str {
    match reason {
        "malware" => {
            "My antivirus flagged the Windows build, and it kept a process running after I quit."
        }
        "source_mismatch" => {
            "The download is much bigger than a build of the repository, and has files that aren't in it."
        }
        "scam" => "The page asks people to message the maker for a 'premium key'.",
        "copyright" => "The title screen music is from a commercial soundtrack.",
        "license" => "It bundles a GPL library but the repository says MIT.",
        "gambling" => "There's a daily spin for random rewards that can be bought with real money.",
        "misleading" => "The screenshots show features the game doesn't have.",
        "broken" => "Crashes straight after the splash screen on Linux.",
        "harassment" => {
            "The review insults the creative personally instead of talking about the work."
        }
        "private_info" => "The review includes the creative's home town and phone number.",
        "fake_review" => {
            "This reviewer never seems to have used the work; it reads like a copy-paste."
        }
        "spam" => "Same message posted over and over.",
        "threat" => "This message threatens to 'find' the other person.",
        _ => "Something isn't right here; please take a look.",
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ModerationGenerated {
    pub acceptances: u64,
    pub reports: usize,
    pub decisions: usize,
    pub appeals: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct ModerationReport {
    pub reports_by_status: Vec<(String, i64)>,
    pub open_overdue: i64,
    pub decisions_by_action: Vec<(String, i64)>,
    pub appeals_by_status: Vec<(String, i64)>,
    /// Should be 0: every actioned report points at a decision.
    pub actioned_without_decision: i64,
    pub suspended_accounts: i64,
}

pub async fn write(
    db: &PgPool,
    rng: &mut Rng,
    now: OffsetDateTime,
) -> anyhow::Result<ModerationGenerated> {
    // Every simulated user accepted the agreements in effect when they joined.
    let acceptances = sqlx::query(
        "insert into legal_acceptances (user_id, kind, version, accepted_at)
         select u.id, d.kind, d.version, greatest(u.created_at, d.effective_at)
         from users u cross join legal_documents d
         where u.display_name like 'Sim %' and d.kind in ('terms', 'publisher', 'people')
           and d.effective_at <= now()
         on conflict do nothing",
    )
    .execute(db)
    .await?
    .rows_affected();
    let mut out = ModerationGenerated {
        acceptances,
        ..Default::default()
    };

    let reporters: Vec<Uuid> =
        sqlx::query_scalar("select user_id from sim_users order by user_id limit 400")
            .fetch_all(db)
            .await?;
    if reporters.is_empty() {
        return Ok(out);
    }
    // (kind, subject id, account it's about, label)
    let mut subjects: Vec<(&str, Uuid, Uuid, String)> = Vec::new();
    let apps: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "select id, maker_id, title from apps where status = 'listed' order by md5(id::text) limit 14",
    )
    .fetch_all(db)
    .await?;
    subjects.extend(
        apps.into_iter()
            .map(|(id, maker, t)| ("app", id, maker, format!("App: {t}"))),
    );
    let reviews: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "select r.job_id, r.client_id, 'Review of ' || c.name || ' by ' || u.display_name
         from creative_reviews r join creatives c on c.user_id = r.creative_id join users u on u.id = r.client_id
         where r.hidden_at is null order by r.vote, md5(r.job_id::text) limit 10",
    )
    .fetch_all(db)
    .await?;
    subjects.extend(reviews.into_iter().map(|(id, u, l)| ("review", id, u, l)));
    let profiles: Vec<(Uuid, String)> = sqlx::query_as(
        "select user_id, 'People profile: ' || name || ' (@' || handle || ')' from creatives
         where portfolio_set_id is not null and hidden_reason is null order by md5(handle) limit 5",
    )
    .fetch_all(db)
    .await?;
    subjects.extend(profiles.into_iter().map(|(id, l)| ("profile", id, id, l)));

    let mut delisted = 0;
    for (i, (kind, id, about, label)) in subjects.iter().enumerate() {
        let reasons: Vec<&moderation::Reason> = moderation::REASONS
            .iter()
            .filter(|r| {
                r.kinds.contains(kind)
                    && !matches!(
                        r.code,
                        "child_abuse" | "intimate_image" | "threat" | "sexual" | "illegal"
                    )
            })
            .collect();
        let reason = *rng.pick(&reasons);
        // One to three people report the same thing.
        let count = 1 + rng.below(3);
        let age = Duration::hours(2 + rng.below(24 * 9) as i64);
        // Older reports have mostly been handled; a few are overdue.
        let handled = age > Duration::days(2) && !rng.chance(0.15);
        let mut ids = Vec::new();
        for n in 0..count {
            let reporter = reporters[(i * 7 + n * 31 + rng.below(50)) % reporters.len()];
            if reporter == *about {
                continue;
            }
            let created = now - age - Duration::minutes(n as i64 * 47);
            let due = created
                + if reason.priority < 2 {
                    Duration::hours(24)
                } else {
                    Duration::days(7)
                };
            let rid = Uuid::now_v7();
            sqlx::query(
                "insert into reports (id, reference, subject_kind, subject_id, subject_user_id, subject_label,
                                      reason, details, reporter_id, priority, due_at, created_at)
                 values ($1, 'R-SIM' || substr(md5($1::text), 1, 2) || '-' || upper(substr(md5($1::text), 3, 4)),
                         $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 on conflict do nothing",
            )
            .bind(rid)
            .bind(kind)
            .bind(id)
            .bind(about)
            .bind(label)
            .bind(reason.code)
            .bind(details(reason.code))
            .bind(reporter)
            .bind(reason.priority)
            .bind(due)
            .bind(created)
            .execute(db)
            .await?;
            ids.push(rid);
            out.reports += 1;
        }
        if !handled || ids.is_empty() {
            continue;
        }
        let decided = now - age + Duration::hours(3 + rng.below(30) as i64);
        // About half are real problems; the rest are dismissed with a reason.
        let rule = reason.rule;
        let act = rule.is_some() && rng.chance(0.55);
        if !act {
            sqlx::query(
                "update reports set status = 'dismissed', decided_by = $2, decided_at = $3,
                        outcome = 'We looked at this and it doesn''t break Slop''s rules, so it stays up.'
                 where id = any($1)",
            )
            .bind(&ids)
            .bind(MODERATOR)
            .bind(decided)
            .execute(db)
            .await?;
            continue;
        }
        let action = match *kind {
            "app" if delisted < 2 && reason.priority < 2 => {
                delisted += 1;
                "delist_app"
            }
            "review" => "hide_review",
            "profile" if rng.chance(0.4) => "hide_profile",
            _ => "warn",
        };
        let facts = match action {
            "delist_app" => format!("{} We confirmed this on the current release.", details(reason.code)),
            "hide_review" => "The review breaks the conduct rules; the rest of the creative's reviews are unaffected.".to_string(),
            "hide_profile" => "The profile breaks the rules until it's fixed; edit it and appeal to have it shown again.".to_string(),
            _ => format!("{} Please fix this; repeated problems can lead to a suspension.", details(reason.code)),
        };
        let mut tx = db.begin().await?;
        let decision = moderation::decide(
            &mut tx,
            &moderation::NewDecision {
                user_id: *about,
                subject_kind: kind,
                subject_id: *id,
                subject_label: label,
                action,
                rule: rule.unwrap_or("misleading"),
                law: None,
                facts: &facts,
                decided_by: MODERATOR,
            },
        )
        .await?
        .id;
        sqlx::query("update moderation_decisions set created_at = $2 where id = $1")
            .bind(decision)
            .bind(decided)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "update reports set status = 'actioned', decided_by = $2, decided_at = $3, decision_id = $4,
                    outcome = 'Thanks: we acted on this. The person it''s about has been told why.'
             where id = any($1)",
        )
        .bind(&ids)
        .bind(MODERATOR)
        .bind(decided)
        .bind(decision)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        out.decisions += 1;

        // Some people appeal; a person keeps most decisions and reverses a few.
        if rng.chance(0.45) {
            let appealed = decided + Duration::hours(5 + rng.below(40) as i64);
            sqlx::query(
                "update moderation_decisions set appeal = $2, appealed_at = $3, appeal_status = 'pending' where id = $1",
            )
            .bind(decision)
            .bind("I think this was a mistake: I've explained the context below and changed what was wrong.")
            .bind(appealed.min(now))
            .execute(db)
            .await?;
            out.appeals += 1;
            if appealed < now - Duration::days(1) {
                let reverse = rng.chance(0.3);
                if reverse {
                    sqlx::query("update creative_reviews set hidden_at = null, hidden_reason = null where job_id = $1 and $2 = 'hide_review'")
                        .bind(id)
                        .bind(action)
                        .execute(db)
                        .await?;
                    sqlx::query("update creatives set hidden_reason = null where user_id = $1 and $2 = 'hide_profile'")
                        .bind(id)
                        .bind(action)
                        .execute(db)
                        .await?;
                    sqlx::query(
                        "update apps set status = 'listed' where id = $1 and $2 = 'delist_app'",
                    )
                    .bind(id)
                    .bind(action)
                    .execute(db)
                    .await?;
                }
                sqlx::query(
                    "update moderation_decisions set appeal_status = $2, appeal_reason = $3, appeal_decided_by = $4,
                            appeal_decided_at = $5, reversed_at = case when $6 then $5 end
                     where id = $1",
                )
                .bind(decision)
                .bind(if reverse { "reversed" } else { "upheld" })
                .bind(if reverse {
                    "With the context in the appeal, this doesn't break the rule. Reversed."
                } else {
                    "We looked again; the decision stands for the reasons given."
                })
                .bind(MODERATOR)
                .bind(appealed + Duration::hours(10))
                .bind(reverse)
                .execute(db)
                .await?;
            }
        }
    }
    Ok(out)
}

pub async fn report(db: &PgPool) -> anyhow::Result<ModerationReport> {
    let by = |sql: &'static str| async move {
        sqlx::query_as::<_, (String, i64)>(sqlx::AssertSqlSafe(sql))
            .fetch_all(db)
            .await
    };
    Ok(ModerationReport {
        reports_by_status: by("select status, count(*) from reports group by 1 order by 1").await?,
        open_overdue: sqlx::query_scalar("select count(*) from reports where status = 'open' and due_at < now()")
            .fetch_one(db)
            .await?,
        decisions_by_action: by("select action, count(*) from moderation_decisions group by 1 order by 1").await?,
        appeals_by_status: by(
            "select appeal_status, count(*) from moderation_decisions where appeal_status is not null group by 1 order by 1",
        )
        .await?,
        actioned_without_decision: sqlx::query_scalar(
            "select count(*) from reports where status = 'actioned' and decision_id is null",
        )
        .fetch_one(db)
        .await?,
        suspended_accounts: sqlx::query_scalar("select count(*) from users where suspended_at is not null")
            .fetch_one(db)
            .await?,
    })
}
