-- AI tools that connect by being approved in the browser (agent_oauth.rs, SECURITY §8.4.3): the
-- MCP authorization spec's OAuth 2.1. The person adds SLAPorSLOP's MCP address to their AI tool,
-- the tool opens SLAPorSLOP in the browser, they approve it there, and the tool gets its tokens
-- without anyone handling one. What it may do is a maker token's (maker_tokens): the approval is
-- a session of kind 'agent', listed and removed like the tokens made on Account → AI agents.

-- The AI tools SLAPorSLOP knows: ones that registered themselves (RFC 7591), and ones named by the
-- https address of their own description (Client ID Metadata Documents), read from there.
create table agent_clients (
    id                  uuid primary key,
    -- What the tool sends as `client_id`: one SLAPorSLOP made when it registered, or the address of
    -- its description.
    client_id           text not null unique check (char_length(client_id) between 1 and 500),
    kind                text not null check (kind in ('registered', 'document')),
    -- The name it gives itself, shown on the approval page with where it sends the browser.
    name                text not null check (char_length(name) between 1 and 60),
    redirect_uris       text[] not null check (cardinality(redirect_uris) between 1 and 10),
    created_at          timestamptz not null default now(),
    -- When it last started an approval or used the token endpoint; registered tools never used
    -- are deleted after a day, others 30 days after their last use with nothing approved still
    -- working (cleanup).
    last_used_at        timestamptz,
    -- A description is read again from its address after this.
    refresh_after       timestamptz
);

-- An approval in progress: from the tool opening the browser until it swaps the code for tokens.
create table agent_auth_requests (
    id                  uuid primary key,
    client_id           uuid not null references agent_clients (id) on delete cascade,
    redirect_uri        text not null,
    -- The tool's own value, handed back to it unchanged.
    state               text check (char_length(state) <= 500),
    -- PKCE S256 challenge: the code is only redeemable with the matching verifier.
    code_challenge      text not null,
    -- The MCP address the tokens are for (RFC 8707).
    resource            text not null,
    -- The scopes the tool asked for: only a starting point for the choices on the page.
    requested_scope     text check (char_length(requested_scope) <= 500),
    expires_at          timestamptz not null,
    -- Filled in when the person approves: who, and what they chose (as for a maker token).
    approved_by         uuid references users (id) on delete cascade,
    code_hash           bytea unique,
    name                text check (char_length(name) between 1 and 60),
    scope               text check (scope in ('read', 'publish')),
    app_ids             uuid[],
    people              boolean,
    days                integer check (days between 1 and 365),
    -- Filled in when the code is swapped for tokens. The code works once: using it again ends
    -- the session it made (OAuth 2.1 §4.1.3).
    redeemed_at         timestamptz,
    session_id          uuid references sessions (id) on delete set null
);

create index agent_auth_requests_client on agent_auth_requests (client_id);

comment on column agent_auth_requests.approved_by is 'purpose: the account that approved the AI tool, until the tool has its tokens (minutes)';
comment on column agent_auth_requests.name is 'purpose: the name the person gave the connection, copied to maker_tokens.name';

-- An approved connection: the agent session it made, the tool, and where it was approved to.
create table agent_grants (
    session_id          uuid primary key references sessions (id) on delete cascade,
    client_id           uuid references agent_clients (id) on delete set null,
    redirect_uri        text not null,
    resource            text not null
);

create index agent_grants_client on agent_grants (client_id);

comment on table agent_grants is 'purpose: which AI tool an approved connection belongs to, shown on Account → AI agents';
