-- Linux desktop builds are published per package format (.deb and .rpm): the updater installs
-- the same kind of package the app was installed from.
alter table desktop_builds drop constraint desktop_builds_platform_check;
alter table desktop_builds add constraint desktop_builds_platform_check
    check (platform in ('windows', 'mac', 'linux-deb', 'linux-rpm'));
