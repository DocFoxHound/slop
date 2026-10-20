-- Chat (DESIGN §19, SECURITY §8.5), built 2026-10-20.
--
-- Messages are end-to-end encrypted with MLS (RFC 9420) on people's devices; Slop is only the
-- delivery service. It stores public key packages, the encrypted messages waiting to be fetched,
-- and the least metadata that takes: who is in a chat, when messages were sent and their sizes.
-- Messages and attachments are deleted after 30 days.

-- Friends: asked by one side, accepted by the other. A friendship is one row, stored with the
-- lower user ID first.
create table friends (
    user_a              uuid not null references users (id) on delete cascade,
    user_b              uuid not null references users (id) on delete cascade,
    since               timestamptz not null default now(),
    primary key (user_a, user_b),
    check (user_a < user_b)
);

create index friends_b on friends (user_b);

create table friend_requests (
    from_id             uuid not null references users (id) on delete cascade,
    to_id               uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (from_id, to_id),
    check (from_id <> to_id)
);

create index friend_requests_to on friend_requests (to_id);

comment on table friends is 'purpose: the user''s friends list, which also decides who may message them (DESIGN §19)';
comment on table friend_requests is 'purpose: friend requests waiting for an answer';

create table user_blocks (
    user_id             uuid not null references users (id) on delete cascade,
    blocked_id          uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (user_id, blocked_id),
    check (user_id <> blocked_id)
);

comment on table user_blocks is 'purpose: people the user blocked: they can''t message or befriend the user';

-- Messages from anyone, not just friends, teammates and People job partners. Off by default, and
-- locked off in the UK under-18 view (DESIGN §19).
alter table users add column chat_from_anyone boolean not null default false;
comment on column users.chat_from_anyone is 'purpose: the user''s choice to accept messages from people they aren''t connected to';

-- Each signed-in Slop app that takes part in chat. Its MLS signing key's private half never
-- leaves the device. A user's first device (or one after a reset) stands alone; later ones are
-- approved by an existing device, which signs the new key, so a stolen Slop session can't add a
-- device that reads chats (SECURITY §8.5).
create table chat_devices (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    name                text not null check (char_length(name) between 1 and 60),
    signature_key       bytea not null unique check (length(signature_key) = 32),
    status              text not null default 'pending' check (status in ('pending', 'active', 'revoked')),
    approved_by         uuid references chat_devices (id),
    approval_sig        bytea,
    created_at          timestamptz not null default now(),
    approved_at         timestamptz,
    last_seen_at        timestamptz not null default now(),
    revoked_at          timestamptz
);

create index chat_devices_user on chat_devices (user_id) where status <> 'revoked';

comment on table chat_devices is 'purpose: the user''s devices in chat (name, public key), so others can encrypt to them; listed on the devices page';

-- Public MLS key packages, used up one at a time to add a device to a chat. The last-resort one
-- is reused once the others run out.
create table chat_key_packages (
    id                  uuid primary key,
    device_id           uuid not null references chat_devices (id) on delete cascade,
    key_package         bytea not null check (length(key_package) <= 4096),
    last_resort         boolean not null default false,
    created_at          timestamptz not null default now()
);

create index chat_key_packages_device on chat_key_packages (device_id, last_resort);

-- Chats: direct (two people), group (up to 50 people), a player team's chat, or the user's own
-- devices (keeping them in step). Group names live inside the encryption, not here.
create table chat_conversations (
    id                  uuid primary key,
    kind                text not null check (kind in ('direct', 'group', 'team', 'self')),
    team_id             uuid unique references teams (id) on delete cascade,
    -- Direct and self chats: one per pair or person ("a:b" or the user's ID).
    pair_key            text unique,
    created_by          uuid references users (id) on delete set null,
    created_at          timestamptz not null default now(),
    -- The MLS epoch Slop has seen: commits must build on it, messages must use it.
    epoch               bigint not null default 0,
    last_message_at     timestamptz,
    check ((kind = 'team') = (team_id is not null))
);

-- Who is in a direct, group or self chat (team chats follow team_members).
create table chat_members (
    conversation_id     uuid not null references chat_conversations (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    role                text not null default 'member' check (role in ('owner', 'member')),
    joined_at           timestamptz not null default now(),
    primary key (conversation_id, user_id)
);

create index chat_members_user on chat_members (user_id);

-- Each person's place in each chat: read up to, muted, hidden.
create table chat_member_state (
    conversation_id     uuid not null references chat_conversations (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    read_seq            bigint not null default 0,
    muted               boolean not null default false,
    -- Left a team chat's list without leaving the team; it comes back with a new message.
    hidden_at           timestamptz,
    primary key (conversation_id, user_id)
);

comment on table chat_members is 'purpose: who is in each chat (metadata Slop needs to deliver messages, SECURITY §8.5)';

-- Encrypted MLS messages and commits, in the order Slop accepted them. Deleted after 30 days.
create table chat_messages (
    seq                 bigserial primary key,
    conversation_id     uuid not null references chat_conversations (id) on delete cascade,
    sender_device_id    uuid references chat_devices (id) on delete set null,
    sender_user_id      uuid references users (id) on delete set null,
    kind                text not null check (kind in ('application', 'commit')),
    epoch               bigint not null,
    body                bytea not null check (length(body) <= 65536),
    -- Message franking (SECURITY §8.5): the sender's commitment to the plaintext, so a reported
    -- message can be checked as genuine without Slop reading any others.
    franking_tag        bytea check (length(franking_tag) = 32),
    created_at          timestamptz not null default now()
);

create index chat_messages_conversation on chat_messages (conversation_id, seq);
create index chat_messages_created on chat_messages (created_at);

comment on table chat_messages is 'purpose: encrypted messages waiting to be fetched; Slop can''t read them. Sender and time are kept 30 days';

-- MLS Welcomes: how a device joins a chat, one per device.
create table chat_welcomes (
    id                  uuid primary key,
    device_id           uuid not null references chat_devices (id) on delete cascade,
    conversation_id     uuid not null references chat_conversations (id) on delete cascade,
    welcome             bytea not null check (length(welcome) <= 262144),
    -- The commit that added the device: it reads messages after this one.
    after_seq           bigint not null,
    created_at          timestamptz not null default now()
);

create index chat_welcomes_device on chat_welcomes (device_id);

-- Attachments (pictures, GIFs, short videos, files), encrypted on the device before upload.
-- Slop can't see what's in them. Deleted after 30 days.
create table chat_attachments (
    id                  uuid primary key,
    conversation_id     uuid not null references chat_conversations (id) on delete cascade,
    uploader_id         uuid references users (id) on delete set null,
    size_bytes          integer not null,
    body                bytea not null,
    created_at          timestamptz not null default now()
);

create index chat_attachments_created on chat_attachments (created_at);

comment on table chat_attachments is 'purpose: encrypted attachments waiting to be fetched; Slop can''t read them; deleted after 30 days';

-- Encrypted history backups (SECURITY §8.5), encrypted on the device with a key only the user
-- holds (from their recovery code). Turning backups off deletes it.
create table chat_backups (
    user_id             uuid primary key references users (id) on delete cascade,
    body                bytea not null,
    updated_at          timestamptz not null default now()
);

comment on table chat_backups is 'purpose: the user''s chat history backup, encrypted with a key only they hold; Slop can''t read it';

-- Reports about chat messages carry the reporter's decrypted copy, checked against the franking tag.
alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review', 'team', 'chat_message'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review', 'team', 'chat_message'));

-- Friend requests and new chats get their own notifications.
alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends'));
