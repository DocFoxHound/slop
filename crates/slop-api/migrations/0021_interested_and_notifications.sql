-- Interested (DESIGN §5, §6, §8) and notifications with optional email (decided 2026-10-20).

-- Coming soon and Beta apps a user wants. Shown in their library, counted publicly as a demand
-- signal ("Most anticipated"), and used for "For you"; the user is told when the app moves on.
create table interests (
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    created_at          timestamptz not null default now(),
    primary key (user_id, app_id)
);

create index interests_app on interests (app_id);

comment on table interests is 'purpose: the user''s Interested list: their library, stage notifications, recommendations and public counts (only counts are public)';

-- What the user should know about, in Slop (the bell). Product messages only, never marketing
-- (LEGAL-GUIDANCE TERMS-3).
create table notifications (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    -- interested, jobs, moderation: each can have its email switched off.
    category            text not null check (category in ('interested', 'jobs', 'moderation')),
    title               text not null check (char_length(title) <= 200),
    body                text not null default '' check (char_length(body) <= 1000),
    -- A path in Slop.
    link                text not null check (link like '/%'),
    -- Unread notifications with the same key are one ("3 new messages in …"), not many.
    collapse_key        text,
    count               integer not null default 1,
    -- The email worker (notify.rs) emails pending ones the user wants by email, or skips them.
    email_status        text not null default 'pending' check (email_status in ('pending', 'queued', 'skipped')),
    created_at          timestamptz not null default now(),
    read_at             timestamptz
);

create index notifications_email_pending on notifications (created_at) where email_status = 'pending';

create index notifications_user on notifications (user_id, created_at desc);
create unique index notifications_one_unread on notifications (user_id, collapse_key)
    where read_at is null and collapse_key is not null;

comment on table notifications is 'purpose: telling the user about their jobs, Interested apps and moderation decisions; deleted 180 days after being read';

-- Emails waiting to be sent, and sent ones until they're cleaned up (cleanup.rs: 30 days).
create table email_outbox (
    id                  uuid primary key,
    to_address          text not null,
    subject             text not null,
    body                text not null,
    created_at          timestamptz not null default now(),
    attempts            integer not null default 0,
    last_error          text,
    sent_at             timestamptz,
    -- Given up after too many failures.
    failed_at           timestamptz
);

create index email_outbox_waiting on email_outbox (created_at) where sent_at is null and failed_at is null;

comment on table email_outbox is 'purpose: delivering product emails (job steps, decisions, report outcomes); deleted 30 days after sending';
comment on column email_outbox.to_address is 'purpose: where the email goes; deleted with the row 30 days after sending';

-- Which notification emails the user wants. Missing means on.
alter table users add column email_prefs jsonb not null default '{}';
comment on column users.email_prefs is 'purpose: the user''s choice of notification emails per category';
