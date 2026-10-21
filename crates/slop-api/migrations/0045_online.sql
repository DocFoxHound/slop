-- What an app connects to online (DESIGN §10, SECURITY §6.1): servers Slop can't check. The store
-- page tells players and suggests sandbox mode; the source check notes networking code an app
-- doesn't declare.
alter table apps
    add column online text[] not null default '{}'
        check (online <@ array['own_servers', 'player_servers', 'peer_to_peer', 'other_services']::text[]),
    add column online_hosts text[] not null default '{}'
        check (cardinality(online_hosts) <= 10);
comment on column apps.online is 'purpose: what the app connects to online, shown to players on its store page';
comment on column apps.online_hosts is 'purpose: the maker''s server names, shown to players on the store page (not personal data)';
