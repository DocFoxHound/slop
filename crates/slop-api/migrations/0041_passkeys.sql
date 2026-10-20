-- Passkeys (SECURITY §4.1, §4.3; passkeys.rs), built 2026-10-20: sign in without a provider, and
-- confirm sensitive actions.
create table passkeys (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    -- The authenticator's credential ID, unique across Slop.
    credential_id       bytea not null unique,
    -- webauthn-rs's Passkey: the public key and signature counter (no secret).
    credential          jsonb not null,
    name                text not null check (char_length(name) between 1 and 60),
    created_at          timestamptz not null default now(),
    last_used_at        timestamptz
);

create index passkeys_user on passkeys (user_id);

comment on table passkeys is 'purpose: signing in and confirming sensitive actions with a passkey; holds public keys only';

-- A ceremony in progress: what the browser must sign, for five minutes.
create table webauthn_challenges (
    id                  uuid primary key,
    -- register, sign_in or step_up
    kind                text not null check (kind in ('register', 'sign_in', 'step_up')),
    user_id             uuid references users (id) on delete cascade,
    session_id          uuid references sessions (id) on delete cascade,
    state               jsonb not null,
    expires_at          timestamptz not null
);

-- When a session last confirmed with a passkey (step-up, session.rs).
alter table sessions add column stepped_up_at timestamptz;
