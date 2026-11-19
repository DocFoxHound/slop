-- Release dates (owner request 2026-11-19): a Coming soon or Beta app's maker may say when they
-- expect to release it, to an exact day or only a month, for the store's "Releasing soon" shelf
-- and the app's tiles and page (DESIGN §5, §8). Not personal data: it's about the app.
--
-- A month is kept as its first day. Players stop seeing a date once it has passed (a day once
-- it's over, a month once it is, in UTC); the maker is told to update it (catalog.rs). Moving
-- to Released clears it (admin::move_stage): a released app is out.
alter table apps
    add column release_date date,
    add column release_date_precision text check (release_date_precision in ('day', 'month')),
    add constraint apps_release_date_precision
        check ((release_date is null) = (release_date_precision is null)),
    add constraint apps_release_date_month_start
        check (release_date_precision is distinct from 'month' or extract(day from release_date) = 1),
    add constraint apps_release_date_upcoming
        check (release_date is null or stage <> 'released');

-- The Releasing soon shelf reads the few apps with a date.
create index apps_release_date on apps (release_date) where release_date is not null;
