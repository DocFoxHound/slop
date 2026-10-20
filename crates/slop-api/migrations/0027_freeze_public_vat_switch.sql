-- Built 2026-10-20: the rest of the "not yet" items from the core features.

-- Frozen apps (SECURITY §12.2, DESIGN §17 decision 11): Slop can no longer read the repository
-- (access removed, repository deleted, or a public app's repository made private). A frozen app
-- takes no new releases; after 14 days it's delisted (public) or no longer shared (private).
-- Installed copies keep working. Everything goes back to normal when access returns.
alter table apps
    add column frozen_at timestamptz,
    add column frozen_reason text,
    -- Delisted because it stayed frozen, so it's listed again when access returns.
    add column frozen_delisted boolean not null default false,
    add column repo_checked_at timestamptz,
    -- Private Slop the owner asked to make public (SECURITY §12.3): waits for a reviewer.
    add column public_requested_at timestamptz;

create index apps_frozen on apps (frozen_at) where frozen_at is not null;

-- Switching or disconnecting the Stripe account that receives donations (SECURITY §9,
-- PAYMENTS §9): needs a fresh sign-in, waits 72 hours, and is announced by email straight away
-- so a hijacked account can't quietly redirect donations. The owner can cancel it meanwhile.
create table stripe_account_changes (
    id                  uuid primary key,
    user_id             uuid not null references users (id),
    kind                text not null check (kind in ('switch', 'disconnect')),
    -- The new account (switch only), set up through Stripe's onboarding while the change waits.
    new_account_id      text,
    -- The new account's Stripe status; the switch waits until it's active.
    new_account_status  text not null default 'pending',
    requested_at        timestamptz not null default now(),
    effective_at        timestamptz not null,
    cancelled_at        timestamptz,
    applied_at          timestamptz,
    check ((kind = 'switch') = (new_account_id is not null))
);

create unique index stripe_account_changes_one_open on stripe_account_changes (user_id)
    where cancelled_at is null and applied_at is null;

comment on table stripe_account_changes is 'purpose: the delay and record for changing where a maker''s donations are paid (SECURITY §9); kept with payment records';

-- VAT on Slop's fee (LEGAL §3.5, decided 2026-10-17): the 5% includes VAT. Individuals in the
-- EU and UK owe VAT at their country's standard rate; VAT-registered businesses give a VAT
-- number and account for it under the reverse charge. Null: no VAT on Slop's fee there.
alter table country_policies add column vat_rate_bp integer check (vat_rate_bp between 0 and 3000);

update country_policies set vat_rate_bp = r.bp from (values
    ('GB', 2000), ('AT', 2000), ('BE', 2100), ('BG', 2000), ('HR', 2500), ('CY', 1900),
    ('CZ', 2100), ('DK', 2500), ('EE', 2400), ('FI', 2550), ('FR', 2000), ('DE', 1900),
    ('GR', 2400), ('HU', 2700), ('IE', 2300), ('IT', 2200), ('LV', 2100), ('LT', 2100),
    ('LU', 1700), ('MT', 1800), ('NL', 2100), ('PL', 2300), ('PT', 2300), ('RO', 2100),
    ('SK', 2300), ('SI', 2200), ('ES', 2100), ('SE', 2500)
) as r(code, bp) where country_code = r.code;

alter table users
    add column vat_number text check (vat_number ~ '^[A-Z]{2}[A-Z0-9+*]{2,13}$'),
    add column vat_checked_at timestamptz;

comment on column users.vat_number is 'purpose: a VAT-registered maker or creative''s number, so Slop''s fee is reverse-charged and shown on their fee statements (LEGAL §3.5)';

-- Notices about your own apps (frozen, made public) and new chat messages (chat.rs).
alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat'));

-- Disconnecting the donations account ends the maker's server donations.
alter table server_donations drop constraint server_donations_cancelled_by_check;
alter table server_donations add constraint server_donations_cancelled_by_check
    check (cancelled_by in ('donor', 'servers_shut_down', 'stripe', 'maker_disconnected'));

-- Card testing (PAYMENTS §9): after a few unfinished checkouts in a day, starting another needs
-- a proof-of-work check, solved by the donor's browser in a second or two. Nothing is sent to a
-- third party. Each challenge works once and for 10 minutes.
create table donation_challenges (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    prefix              text not null,
    difficulty          smallint not null,
    expires_at          timestamptz not null,
    used_at             timestamptz
);

create index donation_challenges_expiry on donation_challenges (expires_at);
