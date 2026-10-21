-- Sandbox mode (DESIGN §6, SECURITY §6.2): players can run an app in the Slop app with no access
-- to the rest of their computer. Makers declare that their app works that way (it keeps local
-- saves, settings and logs only in SLOP_SAVE_DIR); players report apps that don't.
alter table apps
    add column sandbox_ready boolean not null default false;
comment on column apps.sandbox_ready is 'purpose: the maker''s declaration that the app keeps local saves only in SLOP_SAVE_DIR and works in sandbox mode, shown on its store page';

create table sandbox_reports (
    app_id uuid not null references apps (id) on delete cascade,
    user_id uuid not null references users (id) on delete cascade,
    created_at timestamptz not null default now(),
    primary key (app_id, user_id)
);
comment on table sandbox_reports is 'purpose: players who found an app doesn''t work in sandbox mode; only the count is shown, so others know before trying';
comment on column sandbox_reports.user_id is 'purpose: one report per player, and lets them take it back';
comment on column sandbox_reports.created_at is 'purpose: when it was reported, so old reports can be weighed against newer releases';
