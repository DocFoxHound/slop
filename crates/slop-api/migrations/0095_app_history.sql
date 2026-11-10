-- History for makers (owner decision 2026-11-10, docs/DESIGN.md §11, docs/METRICS.md §3): for each
-- app and each UTC day, how many accounts played it, how many played it for the first time, and
-- how many times it was added to a library. Counts only: no account ids and no breakdowns.
-- Counted at the moment it happens (pulse.rs launch, library.rs add); a launch only for an account
-- with "Share my playtime" on; never for a private app, a dedicated server, or the maker's own
-- account.
create table app_daily_counts (
    app_id          uuid not null references apps (id) on delete cascade,
    day             date not null,
    players         integer not null default 0 check (players >= 0),
    first_plays     integer not null default 0 check (first_plays >= 0),
    library_adds    integer not null default 0 check (library_adds >= 0),
    primary key (app_id, day)
);

comment on table app_daily_counts is 'purpose: the maker''s History panel: per app and UTC day, how many accounts that share their playtime played it and played it for the first time, and how many times it was added to a library; counts per app and day; holds no account';

-- From which UTC day an app's numbers are real, so "not counted" is never shown as zero.
-- history_from: the day the app was made, or the day it became public if it was private first.
-- players_from: the first day players per day were counted (the day this shipped, for older apps).
-- An app made after this gets today for both. An app that goes public gets that day for both
-- (admin.rs). Existing rows get the day this migration runs for both; the back-fill below then
-- moves history_from back for public apps.
alter table apps
    add column history_from date not null default ((now() at time zone 'UTC')::date),
    add column players_from date not null default ((now() at time zone 'UTC')::date);
alter table apps add constraint apps_history_dates check (players_from >= history_from);

comment on column apps.history_from is 'purpose: the first UTC day the maker''s History covers (made, or made public); not personal data';
comment on column apps.players_from is 'purpose: the first UTC day players per day were counted for this app; not personal data';

-- Purposes that grow with this (docs/RECORD-OF-PROCESSING.md): a launch and a library add now
-- also count in the maker's daily totals, and "Share my playtime" decides whether a launch does.
comment on column users.share_playtime is 'purpose: the user''s "Share my playtime" setting (PRIV-9); off means no Pulse tokens and no launch counted in makers'' daily totals (app_daily_counts)';
comment on table user_app_usage is 'purpose: the user''s playtime and launch days, shown to them and used to weight votes and for recommendations; the first launch of a UTC day also counts in a public app''s daily totals for its maker (app_daily_counts) while "Share my playtime" is on';
comment on table library_entries is 'purpose: the user''s library, shown to them and used for recommendations; an add also counts in a public app''s daily totals for its maker (app_daily_counts), and a remove the same UTC day takes it back';

-- Back-fill from the rows that still exist. It can only count what is still there: a library
-- entry removed since, and the usage and library rows of an account closed since, are gone
-- (docs/METRICS.md §6). Players per day can't be rebuilt (only the last launch day is kept), so
-- that column starts at 0 today and is shown as "not counted" before players_from.
-- First plays are back-filled only for accounts that share their playtime today. Nothing from
-- the time an app was private is counted, not even earlier on the day it became public: the
-- live count starts at that moment too (pulse.rs, library.rs).
-- The two statements between the markers are run again by a test (history_tests.rs): keep each
-- one a single statement that ends with a semicolon at the end of a line.
-- backfill:start
update apps a
set history_from = least(a.players_from, coalesce(
        (select max((l.at at time zone 'UTC')::date) from audit_log l
         where l.action = 'app.make_public' and l.subject = a.slug),
        (a.created_at at time zone 'UTC')::date))
where a.visibility = 'public';
insert into app_daily_counts (app_id, day, first_plays, library_adds)
select s.app_id, s.day, sum(s.first_plays)::int, sum(s.library_adds)::int
from (
    select u.app_id, u.user_id, u.first_launched_at as at,
           (u.first_launched_at at time zone 'UTC')::date as day, 1 as first_plays, 0 as library_adds
    from user_app_usage u
    union all
    select e.app_id, e.user_id, e.added_at, (e.added_at at time zone 'UTC')::date, 0, 1
    from library_entries e
) s
join apps a on a.id = s.app_id
join users pu on pu.id = s.user_id
where a.visibility = 'public'
  and a.branch_kind is distinct from 'server'
  and s.user_id <> a.maker_id
  and s.day >= a.history_from
  and not exists (select 1 from audit_log l
                  where l.action = 'app.make_public' and l.subject = a.slug and l.at > s.at)
  and (s.first_plays = 0 or pu.share_playtime)
group by s.app_id, s.day;
-- backfill:end
