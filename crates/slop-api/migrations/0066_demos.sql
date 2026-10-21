-- Browser demos (demos.rs, plan phase 15): a new game's web build, playable in the browser for
-- the first days after its release, then deleted. Served from a separate origin.
create table demos (
    app_id              uuid primary key references apps (id) on delete cascade,
    url                 text not null,
    sha256              text,
    size_bytes          bigint,
    -- fetching, live, failed (see detail) or ended.
    status              text not null default 'fetching' check (status in ('fetching', 'live', 'failed', 'ended')),
    detail              text,
    scan_result         text,
    ends_at             timestamptz not null,
    plays               integer not null default 0,
    created_at          timestamptz not null default now()
);

-- Bytes served per day, for the daily cap.
create table demo_traffic (
    day                 date primary key,
    bytes               bigint not null default 0
);
