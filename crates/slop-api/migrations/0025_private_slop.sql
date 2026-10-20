-- Private Slop (DESIGN §13, SECURITY §12), built 2026-10-20: apps only their owner and the people
-- (and, with teams, player teams) they share them with can see. Never in the store, search or
-- recommendations; no store listing, donations or voting.

alter table apps add column visibility text not null default 'public' check (visibility in ('public', 'private'));

create table app_shares (
    app_id              uuid not null references apps (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (app_id, user_id)
);

create index app_shares_user on app_shares (user_id);

comment on table app_shares is 'purpose: who a Private Slop app is shared with (access control, SECURITY §12)';

alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams'));
