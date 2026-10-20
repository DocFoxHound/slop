-- Read-only tokens for private repositories outside GitHub (repo_tokens.rs), built 2026-10-20.
create table repo_tokens (
    app_id              uuid primary key references apps (id) on delete cascade,
    -- The repository it was saved for: moving the app to another repository makes it unused.
    repo_url            text not null,
    -- gitlab, bitbucket or gitea: how it's sent.
    kind                text not null check (kind in ('gitlab', 'bitbucket', 'gitea')),
    -- Nonce and ciphertext (ChaCha20-Poly1305, key SLOP_REPO_TOKEN_KEY, the app's ID as
    -- associated data). Never returned by the API.
    sealed              bytea not null,
    created_by          uuid references users (id) on delete set null,
    created_at          timestamptz not null default now(),
    last_used_at        timestamptz
);

comment on table repo_tokens is 'purpose: reading a maker''s private repository on GitLab, Bitbucket or Gitea to fetch, check and share their releases';
comment on column repo_tokens.created_by is 'purpose: which maker saved the token (accountability)';
