-- Desktop sign-in (SECURITY §4.1-4.2): the app opens the system browser, the signed-in user
-- approves the device on the website, and the browser hands a one-time code back to the app
-- on a loopback address (RFC 8252). The app swaps it for an access token and a rotating
-- refresh token.

-- A session is now either a website cookie session or a signed-in desktop app (one row per
-- device, so the devices page lists both and can revoke either).
alter table sessions add column kind text not null default 'web' check (kind in ('web', 'desktop'));
alter table sessions alter column token_hash drop not null;
alter table sessions add constraint sessions_web_has_token check (kind <> 'web' or token_hash is not null);

create table desktop_auth_requests (
    id                  uuid primary key,
    -- PKCE S256 challenge from the app; the code is only redeemable with the matching verifier.
    code_challenge      text not null,
    redirect_port       integer not null check (redirect_port between 1024 and 65535),
    state               text not null,
    device_name         text not null,
    approved_by         uuid references users (id) on delete cascade,
    code_hash           bytea unique,
    expires_at          timestamptz not null
);

comment on column desktop_auth_requests.device_name is 'purpose: shown on the approval page and the devices page';

-- Short-lived bearer tokens for the desktop app (15 minutes), stored hashed.
create table access_tokens (
    token_hash          bytea primary key,
    session_id          uuid not null references sessions (id) on delete cascade,
    expires_at          timestamptz not null
);

create index access_tokens_session_id on access_tokens (session_id);

-- Rotating refresh tokens. Each use returns a new one; presenting an already-used token means
-- it was copied, so the whole session (token family) is revoked.
create table refresh_tokens (
    token_hash          bytea primary key,
    session_id          uuid not null references sessions (id) on delete cascade,
    used_at             timestamptz,
    expires_at          timestamptz not null
);

create index refresh_tokens_session_id on refresh_tokens (session_id);
