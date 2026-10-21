-- Slop-approved mods (Content Policy "Code from outside Slop"; DESIGN §9): add-ons a game loads,
-- published by anyone for any game. A mod is a branch of the game (parent_app_id) with the kind
-- 'addon': its own listing, repository, releases, checks, review and signature, shown only in the
-- Mods section of the game's page. Its releases are one .zip for every platform ('any'), with no
-- program to start: the Slop app installs it and tells the game where it is.
alter table apps drop constraint apps_branch_kind_check;
alter table apps add constraint apps_branch_kind_check
    check (branch_kind in ('mod', 'fix', 'translation', 'port', 'other', 'addon'));

alter table apps drop constraint apps_platforms_check;
alter table apps add constraint apps_platforms_check
    check (platforms <@ array['windows', 'linux', 'linux-deb', 'linux-rpm', 'any']);
-- 'any' is for mods only, and mods are 'any' only.
alter table apps add constraint apps_addon_platforms
    check ((branch_kind is not distinct from 'addon') = (platforms = array['any']));

alter table release_files drop constraint release_files_platform_check;
alter table release_files add constraint release_files_platform_check
    check (platform in ('windows', 'mac', 'linux', 'android', 'linux-deb', 'linux-rpm', 'any'));
