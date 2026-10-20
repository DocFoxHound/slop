-- Slop's own source check of each release (source_scan.rs) and the daily dependency watch
-- (dep_watch.rs).
alter table releases
    add column repo_scan jsonb,
    add column repo_scanned_at timestamptz,
    add column deps_checked_at timestamptz;

-- The packages a release's lockfiles name, for the dependency watch.
create table release_packages (
    release_id uuid not null references releases (id) on delete cascade,
    ecosystem text not null,
    name text not null,
    version text not null,
    primary key (release_id, ecosystem, name, version)
);

-- Advisories OSV lists for a release's packages, from when Slop first saw them.
create table release_advisories (
    release_id uuid not null references releases (id) on delete cascade,
    vuln_id text not null,
    ecosystem text not null,
    package text not null,
    version text not null,
    summary text not null default '',
    fixed text,
    malicious boolean not null default false,
    -- Slop Build's check already showed it when the release was built.
    known_at_build boolean not null default false,
    found_at timestamptz not null default now(),
    primary key (release_id, vuln_id, ecosystem, package)
);
