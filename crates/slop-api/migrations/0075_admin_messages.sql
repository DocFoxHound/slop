-- Messages from AI agents to Slop's admins (admin_messages.rs, built 2026-10-23): a maker's agent
-- or a staff member's agent reports an error or a bug in Slop, or suggests something. Plain text,
-- threaded, and limited per account so a hijacked agent can't flood the admins.

create table admin_threads (
    id                  uuid primary key,
    -- The account whose agent wrote it.
    sender_id           uuid not null references users (id) on delete cascade,
    -- The kind of token it came with: a maker's agent or a staff member's agent.
    via                 text not null check (via in ('maker_agent', 'admin_agent')),
    -- One staff member it's for (they and owners see it); null: every admin.
    to_staff_id         uuid references users (id) on delete set null,
    category            text not null check (category in ('suggestion', 'report', 'other')),
    subject             text not null check (char_length(subject) between 1 and 120),
    status              text not null default 'open' check (status in ('open', 'closed')),
    created_at          timestamptz not null default now(),
    last_message_at     timestamptz not null default now(),
    closed_at           timestamptz
);

create index admin_threads_sender on admin_threads (sender_id, created_at desc);
create index admin_threads_activity on admin_threads (status, last_message_at desc);
create index admin_threads_closed on admin_threads (closed_at) where status = 'closed';

comment on table admin_threads is 'purpose: a message thread from an account''s AI agent to Slop''s admins (errors, bug reports, suggestions); seen by the sender''s agents and staff; closed threads deleted after a year';

create table admin_thread_messages (
    id                  uuid primary key,
    thread_id           uuid not null references admin_threads (id) on delete cascade,
    author_id           uuid not null references users (id) on delete cascade,
    -- A staff member's reply (true) or the sender's message (false).
    from_staff          boolean not null,
    -- Written with an AI agent's token rather than signed in.
    by_agent            boolean not null,
    body                text not null check (char_length(body) between 1 and 4000),
    created_at          timestamptz not null default now()
);

create index admin_thread_messages_thread on admin_thread_messages (thread_id, created_at);

comment on table admin_thread_messages is 'purpose: the messages in a thread to Slop''s admins and staff replies; deleted with the thread';

-- Every post an account's agents made (a new thread or a reply), kept for a couple of days to
-- count the limits: 1 an hour and 5 a day per account, 60 new threads an hour across Slop. A
-- separate record, so deleting a thread doesn't give its sender their posts back.
create table admin_message_sends (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    new_thread          boolean not null,
    sent_at             timestamptz not null default now()
);

create index admin_message_sends_user on admin_message_sends (user_id, sent_at desc);
create index admin_message_sends_threads on admin_message_sends (sent_at) where new_thread;

comment on table admin_message_sends is 'purpose: when an account''s agents wrote to Slop''s admins, to enforce the per-account and site-wide limits; deleted after 2 days';

-- When each staff member last read each thread (unread marks in the admin panel).
create table admin_thread_reads (
    thread_id           uuid not null references admin_threads (id) on delete cascade,
    staff_id            uuid not null references users (id) on delete cascade,
    read_at             timestamptz not null default now(),
    primary key (thread_id, staff_id)
);

create index admin_thread_reads_staff on admin_thread_reads (staff_id);

comment on table admin_thread_reads is 'purpose: which threads to Slop''s admins a staff member has read';

-- Accounts whose agents may no longer write to the admins.
create table admin_message_blocks (
    user_id             uuid primary key references users (id) on delete cascade,
    reason              text not null check (char_length(reason) between 1 and 1000),
    blocked_by          text not null,
    blocked_at          timestamptz not null default now()
);

comment on table admin_message_blocks is 'purpose: accounts staff stopped from writing to Slop''s admins (abuse), with why and who; removed when unblocked or the account is closed';
