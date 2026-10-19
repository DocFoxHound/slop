-- What an app runs on, chosen on its store page (decided 2026-10-19). Every release must bring
-- exactly these builds:
--   windows     a .zip of the app's files
--   linux       a .tar.gz of the app's files, which the Slop app installs and runs
--   linux-deb   a .deb package of the Linux build (Debian, Ubuntu)
--   linux-rpm   an .rpm package of the Linux build (Fedora, RHEL)
-- Linux needs at least one of the packages. Empty means not chosen yet (apps made before this).

alter table apps
    add column platforms text[] not null default '{}'
        check (platforms <@ array['windows', 'linux', 'linux-deb', 'linux-rpm']);

comment on column apps.platforms is 'purpose: the builds each release of the app must include';

alter table release_files drop constraint release_files_platform_check;
alter table release_files add constraint release_files_platform_check
    check (platform in ('windows', 'mac', 'linux', 'android', 'linux-deb', 'linux-rpm'));
