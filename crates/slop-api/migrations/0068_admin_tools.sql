-- The rest of the admin panel (built 2026-10-22): freezing an app by hand, staff-made store
-- shelves, and legal documents published from the panel. Profile fixes and removing staff need no
-- new tables: they're in the audit log.

-- A freeze a staff member placed (admin_tools.rs): the daily repository check never lifts it and
-- it doesn't take the app out of the store after 14 days. Null for the automatic freeze.
alter table apps add column frozen_by text;
comment on column apps.frozen_by is 'purpose: which staff member froze the app by hand (audit-log name), so the automatic check leaves it alone';

-- Shelves staff make for the front page, beside the automatic ones (shelves.rs).
create table store_shelves (
    id                  uuid primary key,
    slug                text not null unique check (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and char_length(slug) between 2 and 40),
    title               text not null check (char_length(title) between 1 and 60),
    blurb               text not null default '' check (char_length(blurb) <= 160),
    -- Shown on the front page, in this order; off keeps it as a page of its own only.
    on_front            boolean not null default true,
    position            integer not null default 0,
    created_by          uuid references users (id) on delete set null,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

comment on column store_shelves.created_by is 'purpose: which staff member made the shelf (shown only to staff)';

create table store_shelf_apps (
    shelf_id            uuid not null references store_shelves (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    position            integer not null,
    primary key (shelf_id, app_id)
);

create index store_shelf_apps_app on store_shelf_apps (app_id);

-- The automatic front-page shelves staff have hidden (shelves.rs `BUILT_IN`).
create table hidden_front_shelves (
    key                 text primary key,
    hidden_by           uuid references users (id) on delete set null,
    hidden_at           timestamptz not null default now()
);

comment on column hidden_front_shelves.hidden_by is 'purpose: which staff member hid the shelf (shown only to staff)';

-- Legal documents published from the admin panel rather than the files in the repository
-- (legal.rs). Null for a version that came from a file.
alter table legal_documents add column published_by text;
comment on column legal_documents.published_by is 'purpose: which staff member published this version from the admin panel (audit-log name)';
