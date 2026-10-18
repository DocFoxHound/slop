-- Releases, review, revocation, the library and the audit log.
--
-- Phase 1 (ARCHITECTURE §8): scanning starts semi-automatic. Slop fetches each release file once
-- from the maker's release page, records its SHA-256 and size and keeps nothing else; a person
-- reviews the release; approval signs the release manifest (SECURITY §5.3).

create type release_status as enum ('fetching', 'awaiting_review', 'approved', 'rejected', 'failed', 'revoked');

create table releases (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    version             text not null check (version ~ '^[A-Za-z0-9][A-Za-z0-9._+-]{0,39}$'),
    commit_sha          text not null check (commit_sha ~ '^[0-9a-f]{40}$'),
    status              release_status not null default 'fetching',
    -- Shown to the maker for failed, rejected and revoked releases.
    status_reason       text,
    -- True once the commit was found in the linked repo (GitHub only, for now).
    commit_verified     boolean not null default false,
    submitted_by        uuid not null references users (id),
    created_at          timestamptz not null default now(),
    fetch_started_at    timestamptz,
    reviewed_at         timestamptz,
    -- The signed manifest (slop_manifest::Signed), set on approval.
    signed_manifest     jsonb,
    revoked_at          timestamptz,
    unique (app_id, version)
);

create index releases_app_id on releases (app_id, created_at desc);
create index releases_status on releases (status) where status in ('fetching', 'awaiting_review');

create table release_files (
    release_id          uuid not null references releases (id) on delete cascade,
    platform            text not null check (platform in ('windows', 'mac', 'linux', 'android')),
    kind                text not null check (kind in ('archive', 'installer', 'apk')),
    url                 text not null,
    file_name           text not null,
    exec                text,
    args                text[] not null default '{}',
    size_bytes          bigint,
    sha256              text check (sha256 ~ '^[0-9a-f]{64}$'),
    primary key (release_id, platform)
);

-- Append-only record of moderation and review decisions (LEGAL-GUIDANCE §15).
create table audit_log (
    id                  bigint generated always as identity primary key,
    at                  timestamptz not null default now(),
    actor               text not null,
    action              text not null,
    subject             text not null,
    detail              jsonb not null default '{}'
);

create function audit_log_append_only() returns trigger language plpgsql as $$
begin
    raise exception 'audit_log is append-only';
end $$;

create trigger audit_log_no_update before update or delete on audit_log
    for each statement execute function audit_log_append_only();

-- A user's library (DESIGN §6). Adding is free and instant; installing happens on the device.
create table library_entries (
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    added_at            timestamptz not null default now(),
    primary key (user_id, app_id)
);

comment on table library_entries is 'purpose: the user''s library, shown to them and used for recommendations';
