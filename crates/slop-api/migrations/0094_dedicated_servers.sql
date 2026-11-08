-- Dedicated servers linked to their games (owner decisions 2026-11-08, docs/DEDICATED-SERVERS.md):
-- a server is its own app (its own repository, releases, checks, review and signature) linked to
-- its game like a mod is, by parent_app_id, with the kind 'server'. It has no store entry of its
-- own: it's listed on its game's page, and players run it on their own computer or a host so
-- others can play. A game can have several (a world server and a lobby server, say), each from
-- the game's own maker, never from the game's repository.
alter table apps drop constraint apps_branch_kind_check;
alter table apps add constraint apps_branch_kind_check
    check (branch_kind in ('mod', 'fix', 'translation', 'port', 'other', 'addon', 'server'));

-- A server is a download for Windows and/or Linux: not a browser game, and not 'any' (mods only).
alter table apps add constraint apps_server_platforms
    check (branch_kind is distinct from 'server'
           or (cardinality(platforms) > 0 and not ('browser' = any(platforms))));

create index apps_servers on apps (parent_app_id) where branch_kind = 'server';

-- The Servers category lists games that have a dedicated server (and, until their makers link
-- them, servers published on their own before this).
update categories
set description = 'Games with a dedicated server you run on your own computer or a host'
where slug = 'servers';
