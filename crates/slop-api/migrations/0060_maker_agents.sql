-- Makers' AI agents (maker_agents.rs): a maker token is a session of kind 'agent', limited to
-- maker work by maker_tokens.
alter table sessions drop constraint sessions_kind_check;
alter table sessions add constraint sessions_kind_check check (kind in ('web', 'desktop', 'agent'));

create table maker_tokens (
    session_id          uuid primary key references sessions (id) on delete cascade,
    name                text not null check (char_length(name) between 1 and 60),
    -- read: look at apps, releases, checks and reviews. publish: also edit store pages and
    -- submit releases (which still go through Slop's checks and review).
    scope               text not null check (scope in ('read', 'publish')),
    -- The apps it may touch; empty: all the maker's apps, and creating new ones.
    app_ids             uuid[] not null default '{}',
    -- May also handle People jobs: requests, quotes, changes and the job conversation.
    people              boolean not null default false
);

comment on table maker_tokens is 'purpose: what a maker''s AI agent token may do, shown to the maker on their account';
comment on column maker_tokens.name is 'purpose: lets the maker recognise the token (SECURITY §4.2)';

-- What changed in a release, in the maker's words (markdown), shown to players.
alter table releases add column notes text check (char_length(notes) <= 5000);

-- A community build offering its changes back to the original app (offers.rs): a pull request
-- on the original's repository, and the original maker's answer.
create table upstream_offers (
    id                  uuid primary key,
    branch_app_id       uuid not null references apps (id) on delete cascade,
    original_app_id     uuid not null references apps (id) on delete cascade,
    offered_by          uuid not null references users (id) on delete cascade,
    pull_request_url    text not null check (char_length(pull_request_url) <= 500),
    message             text not null check (char_length(message) between 1 and 2000),
    status              text not null default 'open' check (status in ('open', 'accepted', 'declined', 'withdrawn')),
    answer              text check (char_length(answer) <= 2000),
    created_at          timestamptz not null default now(),
    answered_at         timestamptz
);

create index upstream_offers_original on upstream_offers (original_app_id, created_at desc);
create index upstream_offers_branch on upstream_offers (branch_app_id, created_at desc);
create unique index upstream_offers_open_pr on upstream_offers (pull_request_url) where status = 'open';

comment on column upstream_offers.message is 'purpose: the community build maker''s note to the original''s maker';
comment on column upstream_offers.answer is 'purpose: the original maker''s reply to the community build''s maker';
