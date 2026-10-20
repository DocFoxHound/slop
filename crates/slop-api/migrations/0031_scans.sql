-- The scan pipeline (DESIGN §10, SECURITY §5.2), built 2026-10-20.

alter table release_files
    -- Slop's scanner's full report: engines and versions, findings, notable capabilities.
    add column scan_report jsonb,
    -- Microsoft Defender's check of a Windows build, from Slop Build or Slop Check (run in the
    -- maker's own GitHub Actions, proved by the run's OIDC token): the file's hash, Defender's
    -- versions, what it found, and the Authenticode signer if the program is signed.
    add column defender jsonb,
    -- A reviewer cleared a finding they judged a false positive, and why (audit-logged).
    add column cleared_by text,
    add column cleared_reason text check (char_length(cleared_reason) <= 1000),
    add column cleared_at timestamptz;

-- Defender checks from Slop Check (builds made outside Slop Build), by the checked file's hash:
-- they can arrive before or after Slop fetches the release.
create table defender_checks (
    app_id              uuid not null references apps (id) on delete cascade,
    sha256              text not null,
    report              jsonb not null,
    created_at          timestamptz not null default now(),
    primary key (app_id, sha256)
);

-- Slop Build's source checks: possible secrets (file and line, never the value) and dependencies
-- with known vulnerabilities. Shown to the maker and the reviewer.
alter table releases add column source_checks jsonb;

comment on column release_files.cleared_by is 'purpose: accountability for a reviewer overriding a scanner (RECORD-OF-PROCESSING)';
comment on column release_files.cleared_reason is 'purpose: telling the maker and later reviewers why a finding was cleared';
