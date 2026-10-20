-- Alerts for Slop's admins (ops.rs), built 2026-10-20: stale backups, failed deploys, scanner
-- failures, urgent and overdue reports, revocations, admin changes, payment mismatches.
alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends', 'ops'));

-- One row per alert, so each is sent once and repeated at most daily while it lasts.
create table ops_alerts (
    key                 text primary key,
    title               text not null,
    detail              text not null,
    link                text not null,
    first_seen          timestamptz not null default now(),
    last_sent           timestamptz,
    resolved_at         timestamptz
);

-- Where the checks got to in the audit log and the reports.
create table ops_cursor (
    name                text primary key,
    value               bigint not null
);
