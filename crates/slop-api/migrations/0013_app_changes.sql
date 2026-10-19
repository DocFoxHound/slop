-- Makers manage their apps (decided 2026-10-19).
--
-- A change that would have players download something different is checked before it happens:
-- choosing which approved release players get, and moving the app to another repository once
-- players have it. Choosing a release goes through the same rule as new releases (automatic
-- once a person has approved one, otherwise a person); a repository move always waits for a
-- person. Everything else on the app the maker changes directly.

alter table apps
    -- The release players get, when the maker holds the app at one. Null: the newest approved.
    add column pinned_release_id uuid references releases (id) on delete set null,
    -- When the app last moved repository. Automatic approval needs a person's approval after it.
    add column repo_changed_at timestamptz;

comment on column apps.pinned_release_id is 'purpose: the release a maker chose to hold players at (null follows the newest approved release)';

create table app_changes (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    -- release: which release players get. repo: the app's repository.
    kind                text not null check (kind in ('release', 'repo')),
    -- kind = release: the release to hold players at, or null to follow the newest approved.
    release_id          uuid references releases (id) on delete cascade,
    -- kind = repo: the new repository.
    repo_url            text,
    -- The maker's reason, for the reviewer.
    note                text,
    status              text not null default 'pending'
                        check (status in ('pending', 'approved', 'rejected', 'cancelled')),
    -- Shown to the maker: why it was rejected, or why it waits for a person.
    status_reason       text,
    requested_by        uuid not null references users (id),
    created_at          timestamptz not null default now(),
    -- An admin, or 'slop' for an automatic approval.
    reviewed_by         text,
    reviewed_at         timestamptz,
    check ((kind = 'repo') = (repo_url is not null)),
    check (kind = 'release' or release_id is null)
);

comment on table app_changes is 'purpose: makers'' requests to change what players download, and their review';

-- One open request of each kind per app; a new one replaces it.
create unique index app_changes_one_pending on app_changes (app_id, kind) where status = 'pending';
create index app_changes_app on app_changes (app_id, created_at desc);
