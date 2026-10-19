-- Ratings: 👍 Not Slop / 👎 Slop (DESIGN §7, METRICS §1.2 and §2), and "For you"
-- recommendations (DESIGN §5, METRICS §5) (built 2026-10-19).
--
-- Only players who launched the app through Slop (user_app_usage) can vote, once the app is
-- released. Every vote action is an event; the current vote per account is kept alongside, in
-- the same transaction. Each vote is weighted by the voter's playtime at the time (games with
-- Slop Pulse), or by the days they launched it (everything else).
--
-- user_id has no foreign key on purpose: when account deletion is built, it re-keys these rows
-- to a random ID so ratings stay honest (METRICS §6).

create table vote_events (
    id                  uuid primary key,
    user_id             uuid not null,
    app_id              uuid not null references apps (id) on delete cascade,
    -- 1 Not Slop, -1 Slop, 0 withdrawn.
    vote                smallint not null check (vote in (-1, 0, 1)),
    prev_vote           smallint check (prev_vote in (-1, 1)),
    -- The version players got when they voted ("did 1.4 fix it?").
    release_version     text,
    playtime_seconds    bigint not null default 0,
    days_launched       integer not null default 0,
    weight              real not null,
    created_at          timestamptz not null default now()
);

comment on table vote_events is 'purpose: every vote and change of vote, for weighted ratings over time and abuse checks (METRICS §1.2)';

create index vote_events_app on vote_events (app_id, created_at);
create index vote_events_user on vote_events (user_id, created_at);

create table votes (
    user_id             uuid not null,
    app_id              uuid not null references apps (id) on delete cascade,
    vote                smallint not null check (vote in (-1, 1)),
    weight              real not null,
    updated_at          timestamptz not null default now(),
    primary key (user_id, app_id)
);

comment on table votes is 'purpose: each account''s current vote per app; public on profiles by default (DESIGN §7)';

create index votes_app on votes (app_id);

-- "For you" can be switched off (LEGAL §7). Null: the default, which is on, except in the UK
-- under-18 view where profiling is off by default (PRIV-6).
alter table users add column personalize boolean;
comment on column users.personalize is 'purpose: the user''s "Recommendations based on my activity" setting (LEGAL §7, PRIV-6)';
