// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keeping records only as long as the law asks (LEGAL-GUIDANCE §15). Run with the hourly
//! cleanup (cleanup.rs); each rule deletes what's past its time and nothing else.
//!
//! | Record | Kept | Rule |
//! |---|---|---|
//! | Reports, and decisions with their appeals | 6 years after the decision (or report) | DSA, OSA, DMCA |
//! | Child-abuse reports | also until their preservation ends | 18 U.S.C. §2258A(h) |
//! | Law-enforcement requests | 6 years after the answer (or arrival) | CHAT-3 |
//! | Content removed on a terrorist content order | 6 months | Regulation (EU) 2021/784 Art. 6 |
//! | Agreement acceptances | 6 years after the account closed | contract claims |
//! | Crash logs in bug reports | 90 days (the report stays) | data minimisation |
//! | Closed threads from AI agents to the admins | a year after closing | data minimisation |
//!
//! Not here: payments and vote history (10 years) only grow, and the API's own database role
//! can't delete them (migration 0050), so their purge runs as the owner when the first records
//! reach 10 years (2036). Server logs keep no IP addresses in the database; the journal's own
//! retention is set on the machine (deploy/README.md).

use sqlx::PgPool;

/// What one round deleted, per rule.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Purged {
    pub reports: u64,
    pub decisions: u64,
    pub legal_requests: u64,
    pub removed_content: u64,
    pub acceptances: u64,
    pub bug_logs: u64,
    pub ai_messages: u64,
}

pub async fn purge(db: &PgPool) -> sqlx::Result<Purged> {
    let exec = |sql: &'static str| async move {
        sqlx::query(sql)
            .execute(db)
            .await
            .map(|r| r.rows_affected())
    };
    Ok(Purged {
        reports: exec(
            "delete from reports
             where coalesce(decided_at, created_at) < now() - interval '6 years'
               and (preserve_until is null or preserve_until < now())",
        )
        .await?,
        decisions: exec(
            "delete from moderation_decisions d
             where greatest(d.created_at, coalesce(d.appeal_decided_at, d.created_at)) < now() - interval '6 years'
               and not exists (select 1 from reports r where r.decision_id = d.id)",
        )
        .await?,
        legal_requests: exec(
            "delete from legal_requests where coalesce(answered_at, received_at) < now() - interval '6 years'",
        )
        .await?,
        removed_content: exec("delete from removed_content where preserve_until < now()").await?,
        acceptances: exec(
            "delete from legal_acceptances a using users u
             where u.id = a.user_id and u.closed_at < now() - interval '6 years'",
        )
        .await?,
        bug_logs: exec(
            "update bug_reports set log = null where log is not null and created_at < now() - interval '90 days'",
        )
        .await?,
        ai_messages: exec(
            "delete from admin_threads where status = 'closed' and closed_at < now() - interval '1 year'",
        )
        .await?,
    })
}
