-- Linked sign-in accounts, combining Slop accounts, and proving repository ownership on any
-- host (built 2026-10-20; SECURITY §4.1, DESIGN §4 and §8).

-- More sign-in providers. One Slop account can have several, linked from a signed-in session
-- after signing in to each (never because an email matches).
alter table identities drop constraint identities_provider_check;
alter table identities add constraint identities_provider_check
    check (provider in ('github', 'google', 'gitlab', 'bitbucket'));
-- The account's name at the provider (e.g. a GitHub login), shown on the account page.
alter table identities add column login text check (char_length(login) <= 200);
comment on column identities.login is 'purpose: showing which provider account is linked, and checking repository ownership on hosts that name owners';

-- A link started from a signed-in session: the account and session it's for.
alter table oauth_flows add column link_user_id uuid references users (id) on delete cascade;
alter table oauth_flows add column link_session_id uuid;

-- Signing in to a provider account that already has its own Slop account, while linking: the
-- two can be combined once the user confirms.
create table pending_merges (
    token_hash          bytea primary key,
    into_user_id        uuid not null references users (id) on delete cascade,
    from_user_id        uuid not null references users (id) on delete cascade,
    session_id          uuid not null,
    provider            text not null,
    provider_subject    text not null,
    expires_at          timestamptz not null
);

-- A combined account: closed, pointing at the one it went into.
alter table users add column merged_into uuid references users (id);

-- Proving you own a repository on a host Slop can't ask (self-hosted Git, GitLab groups,
-- Bitbucket workspaces): a code in the repository's README.
create table repo_claims (
    user_id             uuid not null references users (id) on delete cascade,
    -- The repository's address, normalized (lower case, no trailing slash or .git).
    repo_key            text not null check (char_length(repo_key) <= 300),
    code                text not null,
    created_at          timestamptz not null default now(),
    verified_at         timestamptz,
    primary key (user_id, repo_key)
);

comment on table repo_claims is 'purpose: proving the user controls a repository before it''s linked to Slop';

alter table pending_signups add column login text;
