-- Slop Pulse (DESIGN §11): "N playing now" and player counts over time, from games that choose
-- to ping Slop. Optional for makers (decided 2026-10-18).
--
-- The desktop app gets a one-time launch token when a signed-in player starts an app and hands
-- it to the game (SLOP_PULSE_TOKEN). The game sends `start`, then a heartbeat about once a
-- minute, carrying nothing but the token (PRIV-8). Raw sessions are deleted within a day
-- (SECURITY §10); what's kept is per-app player counts per 5 minutes (no personal data) and each
-- player's own playtime.

alter table users add column share_playtime boolean not null default true;
comment on column users.share_playtime is 'purpose: the user''s "Share my playtime" setting (PRIV-9); off means no Pulse tokens';

create table pulse_sessions (
    id                  uuid primary key,
    token_hash          bytea not null unique,
    app_id              uuid not null references apps (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    issued_at           timestamptz not null default now(),
    started_at          timestamptz,
    last_beat_at        timestamptz,
    ended_at            timestamptz,
    beats               integer not null default 0
);

create index pulse_sessions_live on pulse_sessions (app_id, last_beat_at);
create index pulse_sessions_issued on pulse_sessions (issued_at);

comment on table pulse_sessions is 'purpose: live player counts; raw rows deleted within 24 hours (SECURITY §10)';

-- Players per app per 5-minute bucket: the history behind "peak this month" and charts.
create table pulse_counts (
    app_id              uuid not null references apps (id) on delete cascade,
    bucket              timestamptz not null,
    players             integer not null,
    primary key (app_id, bucket)
);

-- Each player's own use of each app: shown to them, and later used to weight their votes
-- (METRICS §2: playtime for games, days launched for other apps).
create table user_app_usage (
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    playtime_seconds    bigint not null default 0,
    days_launched       integer not null default 0,
    last_launch_day     date,
    first_launched_at   timestamptz not null default now(),
    last_played_at      timestamptz,
    primary key (user_id, app_id)
);

comment on table user_app_usage is 'purpose: the user''s playtime and launch days, shown to them and used to weight votes';
