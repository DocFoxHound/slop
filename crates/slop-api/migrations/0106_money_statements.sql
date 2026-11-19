-- Yearly money statements for makers and creatives (decided 2026-11-19, PAYMENTS §8): totals of
-- what was paid into their own Stripe accounts, for their records. Not a tax form.
--
-- Money is a user's when it was paid into a Stripe account that was theirs. The account a payment
-- went to is on every payment record, but `donation_accounts` and `creatives` hold only the
-- account in use now: a disconnect deletes the row, and a switch replaces the ID. So each Stripe
-- account a user has had is recorded here, once, by the first user it was saved for, and kept
-- after it stops being used. Going through the account (rather than an app's maker or a job's
-- creative) also counts money the way Stripe does: what reached that account.
create table stripe_account_owners (
    stripe_account_id   text primary key,
    user_id             uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now()
);

create index stripe_account_owners_user on stripe_account_owners (user_id);

comment on table stripe_account_owners is
    'purpose: every Stripe account a maker or creative has received money in, so their statements keep it after a disconnect or switch (PAYMENTS §8); kept with the payment records';

-- Recorded whenever an account is saved for a user, whichever code saves it. The first owner
-- stays: a later row naming the same account never moves its history to someone else.
create function slop_note_stripe_account() returns trigger language plpgsql as $$
declare
    account text := to_jsonb(new) ->> tg_argv[0];
    owner uuid := (to_jsonb(new) ->> tg_argv[1])::uuid;
begin
    if account is not null and owner is not null then
        insert into stripe_account_owners (stripe_account_id, user_id) values (account, owner)
        on conflict (stripe_account_id) do nothing;
    end if;
    return new;
end
$$;

create trigger donation_accounts_owner after insert or update of stripe_account_id on donation_accounts
    for each row execute function slop_note_stripe_account('stripe_account_id', 'user_id');
create trigger creatives_owner after insert or update of stripe_account_id on creatives
    for each row execute function slop_note_stripe_account('stripe_account_id', 'user_id');
create trigger stripe_account_changes_owner after insert or update of new_account_id on stripe_account_changes
    for each row execute function slop_note_stripe_account('new_account_id', 'user_id');
create trigger bounty_claims_owner after insert on bounty_claims
    for each row execute function slop_note_stripe_account('stripe_account_id', 'claimed_by');

-- The accounts so far: those in use, those a switch moved to, then those only payments still name
-- (a disconnected account, or one switched away from), through the app or job they were for.
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select stripe_account_id, user_id, created_at from donation_accounts
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select stripe_account_id, user_id, created_at from creatives where stripe_account_id is not null
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select new_account_id, user_id, requested_at from stripe_account_changes where new_account_id is not null
order by requested_at
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select stripe_account_id, claimed_by, created_at from bounty_claims order by created_at
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select d.stripe_account_id, a.maker_id, d.created_at from donations d join apps a on a.id = d.app_id
order by d.created_at
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select s.stripe_account_id, a.maker_id, s.created_at from server_donations s join apps a on a.id = s.app_id
order by s.created_at
on conflict do nothing;
insert into stripe_account_owners (stripe_account_id, user_id, created_at)
select p.stripe_account_id, j.creative_id, p.created_at
from payments p join job_payments jp on jp.id = p.job_payment_id join jobs j on j.id = jp.job_id
where p.stripe_account_id is not null
order by p.created_at
on conflict do nothing;

-- A statement reads a user's payments by account, and the refunds and disputes recorded against
-- them (a People job's name no account of their own).
create index payments_account on payments (stripe_account_id, created_at);
create index payments_refund_of on payments (refund_of) where refund_of is not null;
