-- Passkeys for publishers and creatives (SECURITY §4.1): after a 30-day grace period, publishing
-- and payout changes need a passkey used in the last 12 hours, so a stolen sign-in elsewhere
-- (a provider account, a session) can't publish or redirect money.
alter table users
    add column passkey_due_at timestamptz,
    add column passkey_reminders smallint not null default 0,
    add column passkey_grace_until timestamptz;
comment on column users.passkey_due_at is 'purpose: when this publisher or creative must have a passkey (30 days after they first published or set up as a creative), to remind them before and enforce after';
comment on column users.passkey_reminders is 'purpose: how many passkey reminders were sent (1: a week before, 2: a day before), so each goes once';
comment on column users.passkey_grace_until is 'purpose: staff let someone who lost their passkeys publish without one until then, after checking it''s them (an audit log entry says who and why)';

alter table sessions add column passkey_at timestamptz;
comment on column sessions.passkey_at is 'purpose: when this session last proved a passkey (signing in with one, or confirming with one), for actions that need a passkey';
update sessions set passkey_at = stepped_up_at where stepped_up_at is not null;

-- Publishers and creatives already on Slop get 30 days from now.
update users u set passkey_due_at = now() + interval '30 days'
where not exists (select 1 from passkeys p where p.user_id = u.id)
  and (exists (select 1 from apps a where a.maker_id = u.id)
       or exists (select 1 from creatives c where c.user_id = u.id));
