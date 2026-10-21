-- Players' bug reports to an app's maker (bug_reports.rs, plan phase 13). Private between the
-- player, the maker and staff: a crash log can hold personal details, so its text is deleted
-- after 90 days (retention.rs).
alter table apps add column bug_reports boolean not null default true;

create table bug_reports (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    reporter_id         uuid not null references users (id) on delete cascade,
    -- The version and platform the player was running.
    version             text check (char_length(version) <= 40),
    platform            text check (platform in ('windows', 'linux', 'mac', 'android')),
    title               text not null check (char_length(title) between 1 and 120),
    body                text not null check (char_length(body) between 1 and 4000),
    -- The last lines of the game's output, when the player chose to send them.
    log                 text check (char_length(log) <= 16000),
    -- open, fixed (in a release), wont_fix, duplicate.
    status              text not null default 'open' check (status in ('open', 'fixed', 'wont_fix', 'duplicate')),
    maker_note          text check (char_length(maker_note) <= 2000),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

create index bug_reports_app on bug_reports (app_id, status, created_at desc);

comment on table bug_reports is 'purpose: a player''s bug report to an app''s maker, seen only by them, the maker and staff';
comment on column bug_reports.log is 'purpose: the game''s last output lines the player chose to send, to find the bug; deleted after 90 days';

-- "Me too": other players with the same problem, so repeats add to one report.
create table bug_report_votes (
    report_id           uuid not null references bug_reports (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (report_id, user_id)
);

comment on table bug_report_votes is 'purpose: counting players who have the same problem as a bug report';
