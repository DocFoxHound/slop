-- Updates without the paperwork (decided 2026-10-19).
--
-- Slop watches each app's GitHub repository for new published releases and submits them itself,
-- using the previous release's files as the pattern. Once a person has approved one release of
-- an app, later releases that pass every check are approved automatically. A withdrawal or
-- rejection since the last person's approval sends the next release back to a person.

alter table apps
    add column watch_releases boolean not null default true,
    add column auto_update boolean not null default true,
    -- The newest GitHub release Slop has looked at, so each is handled once.
    add column watch_last_tag text,
    add column watch_checked_at timestamptz,
    -- Why the last GitHub release couldn't be submitted, shown to the maker.
    add column watch_problem text;

comment on column apps.watch_releases is 'purpose: the maker''s "Pick up new GitHub releases" setting';
comment on column apps.auto_update is 'purpose: the maker''s "Automatic updates" setting';

alter table releases
    -- maker (the form), slop_build (CI) or github_release (Slop's watcher).
    add column source text not null default 'maker' check (source in ('maker', 'slop_build', 'github_release')),
    -- Who approved, rejected or withdrew it: an admin, or 'slop' for an automatic approval.
    add column reviewed_by text;
