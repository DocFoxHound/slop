-- Browser games (DESIGN §8, decided 2026-11-07): a game that runs entirely on its maker's own
-- website. There's nothing to download: players add it to their library, and Play opens its
-- address in their browser. Its store page chooses the platform 'browser', alone. Each release
-- is a play address instead of files: SLAPorSLOP checks the address (https, a public site, the
-- bad-link lists, and that the page loads), a person approves it, and nothing is signed, since
-- SLAPorSLOP has no file to sign. 'browser' is never a file platform (release_files keeps its
-- list, and so does slop-manifest's Platform).
alter table apps drop constraint apps_platforms_check;
alter table apps add constraint apps_platforms_check
    check (platforms <@ array['windows', 'linux', 'linux-deb', 'linux-rpm', 'any', 'browser']);
-- A browser game plays only in the browser.
alter table apps add constraint apps_browser_alone
    check (not ('browser' = any(platforms)) or platforms = array['browser']);
comment on column apps.platforms is
    'purpose: the builds each release must include (windows, linux, linux-deb, linux-rpm; any for mods), or browser alone: it plays on the maker''s website';

-- The address a release of a browser game plays at, and what the automatic checks found. Plain
-- http passes only on test setups (releases.allow_http_downloads), checked in releases.rs. Not
-- personal data: it's the maker's public site.
alter table releases
    add column play_address text
        check (play_address is null or (play_address ~ '^https?://' and char_length(play_address) <= 2000)),
    add column address_checks jsonb;
-- There's nothing to sign for an address.
alter table releases add constraint releases_play_address_unsigned
    check (play_address is null or signed_manifest is null);
comment on column releases.play_address is
    'purpose: a browser game''s play address in this release, which Play opens once a person approves it (the maker''s site, not personal data)';
comment on column releases.address_checks is
    'purpose: what SLAPorSLOP''s automatic checks of play_address found (https, public site, bad-link lists, the page loading, redirects), for the maker and the reviewer';
-- The address Play opens: the newest reviewed one (catalog.rs PLAY_ADDRESS).
create index releases_play_address on releases (app_id, reviewed_at desc) where play_address is not null;

-- Playing a browser game is recorded like a launch, and votes and bug reports say where.
alter table user_app_usage drop constraint user_app_usage_last_platform_check;
alter table user_app_usage add constraint user_app_usage_last_platform_check
    check (last_platform in ('windows', 'linux', 'browser'));
alter table vote_events drop constraint vote_events_platform_check;
alter table vote_events add constraint vote_events_platform_check
    check (platform in ('windows', 'linux', 'browser'));
alter table bug_reports drop constraint bug_reports_platform_check;
alter table bug_reports add constraint bug_reports_platform_check
    check (platform in ('windows', 'linux', 'mac', 'android', 'browser'));
