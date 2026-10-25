-- Where each file of a release is while Slop checks it (releases.rs, fetch_release), so its maker
-- (and their AI agent) see "checking file 2 of 4: scanning for malware" instead of one step that
-- seems stuck for half an hour. Files are checked one at a time; each one's start and end also
-- give an honest estimate of how long the rest will take. Nothing here is personal data. The
-- API's role gets these columns through its table grants (0050_app_role.sql).
alter table release_files
    add column check_stage text check (check_stage in ('downloading', 'signature', 'scanning')),
    add column check_started_at timestamptz,
    add column checked_at timestamptz;
comment on column release_files.check_stage is
    'purpose: what Slop is doing with this file right now (downloading, signature, scanning); null when not being checked';
comment on column release_files.check_started_at is
    'purpose: when Slop started checking this file, for its maker''s progress view and time estimates';
comment on column release_files.checked_at is
    'purpose: when Slop finished checking this file (size, hash, signature and scan recorded)';
