-- Phone and browser notifications (web push), built 2026-10-20 (DESIGN §19).
--
-- A push carries nothing: the phone wakes, fetches the newest notification from Slop with its own
-- sign-in, and shows it. So the push service (Google, Apple, Mozilla, Microsoft) learns only that
-- something happened, never what.
create table push_subscriptions (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    -- The browser's push service address for this device.
    endpoint            text not null unique check (char_length(endpoint) <= 1000),
    created_at          timestamptz not null default now(),
    last_pushed_at      timestamptz,
    failures            integer not null default 0
);

create index push_subscriptions_user on push_subscriptions (user_id);

comment on table push_subscriptions is 'purpose: waking the user''s phone or browser when there''s a new notification; deleted when they switch it off, the push service drops it, or the account closes';

-- When a notification was last pushed. A merged notification (more messages in the same chat)
-- is pushed again after a couple of minutes, not on every message.
alter table notifications add column pushed_at timestamptz;
