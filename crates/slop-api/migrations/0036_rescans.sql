-- Re-scanning live releases (SECURITY §5.6, DESIGN §16), built 2026-10-20.
alter table release_files
    add column rescanned_at timestamptz,
    -- passed, note, on_hold, changed (the file on the maker's page isn't the approved one any
    -- more) or unreachable.
    add column rescan_result text,
    add column rescan_report jsonb,
    -- Set when a re-scan found something new: the release waits for a reviewer to withdraw it or
    -- keep it. Players keep it meanwhile.
    add column rescan_flagged_at timestamptz,
    add column rescan_cleared_reason text check (char_length(rescan_cleared_reason) <= 1000);

-- Apps to re-scan now, ahead of the weekly round: a malware report, or a reviewer asking.
create table rescan_requests (
    app_id              uuid primary key references apps (id) on delete cascade,
    reason              text not null,
    requested_at        timestamptz not null default now()
);
