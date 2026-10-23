-- Messages to Slop's admins, tightened (owner's decision, 2026-10-23): agents see staff by their
-- roles unless a staff member chooses to be listed by name, and every message records which
-- token sent it, so abuse is visible and can be traced to a token (never its secret).

-- Off: AI agents see this staff member only by their roles ("Slop reviewer").
alter table users add column staff_listed_by_name boolean not null default false;
comment on column users.staff_listed_by_name is 'purpose: whether a staff member chose to show their display name to AI agents writing to the admins';

-- The token a message was sent with: the sender's agent token, or a staff member's admin token
-- for a reply an agent wrote. Empty for a reply staff wrote signed in.
alter table admin_thread_messages
    add column token_kind text check (token_kind in ('maker', 'admin')),
    add column token_id uuid,
    add column token_name text check (char_length(token_name) <= 60),
    add column token_scope text,
    add column token_created_at timestamptz,
    add constraint admin_thread_messages_provenance check (
        from_staff or (token_kind is not null and token_id is not null and token_name is not null
                       and token_scope is not null and token_created_at is not null)
    );

comment on column admin_thread_messages.token_name is 'purpose: the name the account gave the token that sent the message, shown to staff to trace abuse; deleted with the thread';
