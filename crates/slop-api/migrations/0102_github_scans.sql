-- Scans run on makers' GitHub runners (decided 2026-11-17: as many scans as possible run on the
-- maker's own GitHub Actions, not SLAPorSLOP's servers). Slop Build scans each file it builds, and
-- Slop Check each file of a release made elsewhere, with SLAPorSLOP's scanner image (ClamAV, YARA,
-- capa), and send the report with the run's OIDC token (slop_build.rs). Kept by file hash, like
-- Microsoft Defender's checks, and used when SLAPorSLOP fetches a file with that hash.
create table ci_scans (
    app_id      uuid not null references apps (id) on delete cascade,
    sha256      text not null check (sha256 ~ '^[0-9a-f]{64}$'),
    report      jsonb not null,
    received_at timestamptz not null default now(),
    primary key (app_id, sha256)
);
comment on table ci_scans is
    'purpose: malware scans of release files run on the maker''s GitHub runners, used instead of scanning them again';

-- A release whose files wait for their scans from GitHub (Slop Check sends them after the release
-- is published on GitHub, often after SLAPorSLOP has fetched it). The fetcher goes on when they've
-- all come, or scans the rest itself after SLOP_CI_SCAN_WAIT_MINUTES.
alter table releases add column scans_waiting_since timestamptz;
comment on column releases.scans_waiting_since is
    'purpose: going on with a release''s checks once the scans from the maker''s GitHub runners arrive';

-- A file waiting for its scan from GitHub says so on the release's progress (release_progress.rs).
alter table release_files drop constraint release_files_check_stage_check;
alter table release_files add constraint release_files_check_stage_check
    check (check_stage in ('downloading', 'signature', 'scanning', 'waiting_for_scan'));

-- The source check run on the maker's GitHub runner (slop-source-check, crates/slop-source-scan):
-- what it read in the repository at a commit, before the store page's licence is applied. Used
-- instead of downloading the repository's archive (source_scan.rs).
create table ci_source_scans (
    app_id      uuid not null references apps (id) on delete cascade,
    commit_sha  text not null check (commit_sha ~ '^[0-9a-f]{40}$'),
    raw         jsonb not null,
    tool        text not null check (char_length(tool) <= 100),
    received_at timestamptz not null default now(),
    primary key (app_id, commit_sha)
);
comment on table ci_source_scans is
    'purpose: source checks run on the maker''s GitHub runners, used instead of downloading the repository';
