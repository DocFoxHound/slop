-- Staff roles and two-person approval (SECURITY §8.4, staff.rs), built 2026-10-20.

-- What each staff member (users.is_admin) may do. Staff with no role can only look.
create table staff_roles (
    user_id             uuid not null references users (id) on delete cascade,
    role                text not null check (role in ('owner', 'moderator', 'reviewer', 'support', 'finance', 'security')),
    granted_by          text not null,
    granted_at          timestamptz not null default now(),
    primary key (user_id, role)
);

comment on table staff_roles is 'purpose: limiting what each staff member can do (SECURITY §8.4)';

-- Today's admins become owners.
insert into staff_roles (user_id, role, granted_by)
select id, 'owner', 'migration 0038' from users where is_admin;

-- Sensitive actions wait here until enough staff approve them (SLOP_APPROVALS_REQUIRED, 1 while
-- Slop has one person). The requester is the first approval.
create table staff_actions (
    id                  uuid primary key,
    -- release.revoke, app.delist, report.act (a suspension), account.suspend, account.lift, staff.role
    kind                text not null,
    -- One line for people: what it does to whom.
    summary             text not null check (char_length(summary) <= 500),
    payload             jsonb not null,
    requested_by        uuid references users (id) on delete set null,
    requested_actor     text not null,
    approvals_needed    smallint not null check (approvals_needed >= 1),
    status              text not null default 'pending' check (status in ('pending', 'done', 'failed', 'cancelled', 'expired')),
    result              text,
    requested_at        timestamptz not null default now(),
    decided_at          timestamptz
);

create index staff_actions_pending on staff_actions (requested_at) where status = 'pending';

create table staff_action_approvals (
    action_id           uuid not null references staff_actions (id) on delete cascade,
    -- The person (a token counts as its admin, so one person can't approve twice).
    user_id             uuid not null references users (id) on delete cascade,
    actor               text not null,
    approved_at         timestamptz not null default now(),
    primary key (action_id, user_id)
);

comment on table staff_actions is 'purpose: two-person approval of sensitive staff actions, and their record';
