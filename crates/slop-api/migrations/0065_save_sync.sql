-- Save sync (save_sync.rs): each player's saves for each app, end-to-end encrypted on their
-- devices; Slop stores ciphertext it can't read. The newest and the one before are kept.
create table save_blobs (
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    -- Counts up with each upload; a device uploads only on top of the one it last saw.
    generation          integer not null check (generation > 0),
    body                bytea not null check (length(body) <= 20971520 + 1024),
    -- Which device sent it (its own random name), so others know it's not theirs.
    device              text not null check (char_length(device) <= 64),
    created_at          timestamptz not null default now(),
    primary key (user_id, app_id, generation)
);

comment on table save_blobs is 'purpose: a player''s encrypted saves, so they follow them between their computers; Slop can''t read them (PRIV: end-to-end encrypted)';
