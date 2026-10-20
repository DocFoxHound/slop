-- Teams (DESIGN §14), built 2026-10-20.
--
-- Player teams: anyone creates or joins one (open, or by invitation). Members share picks and
-- game nights, and can see Private Slop shared with the team.
-- Official teams: developer studios verified through their GitHub organization (teams.rs checks
-- it daily). They show the apps they publish, and can never receive Private Slop.

create table teams (
    id                  uuid primary key,
    slug                text not null unique check (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and char_length(slug) between 3 and 40),
    name                text not null check (char_length(name) between 1 and 60),
    kind                text not null check (kind in ('player', 'official')),
    about               text not null default '' check (char_length(about) <= 2000),
    join_policy         text not null default 'invite' check (join_policy in ('open', 'invite')),
    created_by          uuid not null references users (id),
    created_at          timestamptz not null default now(),
    -- Official teams: the GitHub organization, and an optional website domain proved by DNS.
    github_org          text check (github_org ~ '^[A-Za-z0-9-]{1,39}$'),
    dns_domain          text check (char_length(dns_domain) <= 253),
    dns_token           text,
    -- Set while every check passes; cleared when one stops passing.
    verified_at         timestamptz,
    checked_at          timestamptz,
    -- The last checks: [{check, ok, detail}].
    checks              jsonb not null default '[]',
    -- Hidden by a moderation decision.
    hidden_reason       text,
    check ((kind = 'official') = (github_org is not null))
);

create unique index teams_one_per_org on teams (lower(github_org)) where kind = 'official';

comment on column teams.name is 'purpose: the team''s public name, chosen by its members';
comment on column teams.about is 'purpose: the team''s public description';

create table team_members (
    team_id             uuid not null references teams (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    role                text not null default 'member' check (role in ('owner', 'admin', 'member')),
    joined_at           timestamptz not null default now(),
    primary key (team_id, user_id)
);

create index team_members_user on team_members (user_id);

comment on table team_members is 'purpose: who belongs to a team (shown to members, and for official teams publicly), and their role';

create table team_invites (
    team_id             uuid not null references teams (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    invited_by          uuid not null references users (id),
    created_at          timestamptz not null default now(),
    primary key (team_id, user_id)
);

-- Picks: apps members recommend to the team.
create table team_picks (
    id                  uuid primary key,
    team_id             uuid not null references teams (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    note                text not null default '' check (char_length(note) <= 280),
    created_at          timestamptz not null default now(),
    unique (team_id, app_id)
);

-- Game nights.
create table team_events (
    id                  uuid primary key,
    team_id             uuid not null references teams (id) on delete cascade,
    app_id              uuid references apps (id) on delete set null,
    title               text not null check (char_length(title) between 1 and 100),
    starts_at           timestamptz not null,
    created_by          uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now()
);

create index team_events_team on team_events (team_id, starts_at);

-- Private Slop shared with a player team (never an official one: DESIGN §13).
create table app_team_shares (
    app_id              uuid not null references apps (id) on delete cascade,
    team_id             uuid not null references teams (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (app_id, team_id)
);

-- Teams can be reported and hidden.
alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review', 'team'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review', 'team'));
alter table moderation_decisions drop constraint moderation_decisions_action_check;
alter table moderation_decisions add constraint moderation_decisions_action_check
    check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn', 'hide_team'));
