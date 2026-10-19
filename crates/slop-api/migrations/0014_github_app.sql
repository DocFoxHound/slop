-- Fewer, cheaper GitHub requests (decided 2026-10-19).
--
-- The release watcher asks GitHub "has this changed?" (ETag), and checks quiet apps less often.
-- Makers can install Slop's GitHub App on a repository: GitHub then tells Slop when a release is
-- published (webhook), and requests about that repository use the installation's own allowance
-- instead of Slop's shared token.

alter table apps
    -- The ETag of the last "latest release" answer the watcher fully handled.
    add column watch_etag text;

create table github_installations (
    -- owner/repo, lower case.
    repo_path           text primary key,
    installation_id     bigint not null,
    updated_at          timestamptz not null default now()
);

comment on table github_installations is 'purpose: which repositories have Slop''s GitHub App installed, to receive their release webhooks and use their API allowance';

create index github_installations_installation on github_installations (installation_id);
