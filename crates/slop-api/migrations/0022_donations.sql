-- Donations and monthly server donations (decided 2026-10-17, built 2026-10-20: DESIGN §12,
-- PAYMENTS §4–6 and §8, LEGAL §3, LEGAL-GUIDANCE PAY-1 to PAY-17).
--
-- Every donation is a direct charge on the maker's own Stripe account with Slop's 5% fee
-- (PAY-1). Donations give nothing back: no column here is ever exposed to apps (PAY-5).

-- PAY-12: where the donor covers card processing (one all-in total) or, switched off for a
-- country, the donor's amount is the total and processing comes out of what the maker receives.
alter table country_policies add column donor_covers_fees boolean not null default true;

-- PAY-11: one published processing rate per currency, the same for every payment method. What's
-- left over or short after Stripe's actual fee is the maker's.
create table processing_rates (
    currency            text primary key check (currency in ('usd', 'eur', 'gbp')),
    percent_bp          integer not null check (percent_bp between 0 and 1000),
    fixed_minor         integer not null check (fixed_minor between 0 and 100)
);

insert into processing_rates (currency, percent_bp, fixed_minor) values
    ('usd', 290, 30), ('eur', 290, 25), ('gbp', 290, 20);

-- A maker's Stripe account for receiving donations (PAYMENTS §4). The same Stripe account as their
-- People profile when they have one.
create table donation_accounts (
    user_id             uuid primary key references users (id),
    stripe_account_id   text not null unique,
    stripe_status       text not null default 'pending' check (stripe_status in ('pending', 'active', 'restricted')),
    -- PAY-9: "Donations may be taxable income for you. Slop doesn't give tax advice."
    tax_notice_ack_at   timestamptz not null,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

comment on column donation_accounts.stripe_account_id is 'purpose: paying donations straight to the maker (PAYMENTS §3); identity and bank details stay at Stripe';
comment on column donation_accounts.tax_notice_ack_at is 'purpose: proof the maker saw the tax notice before enabling donations (PAY-9)';

alter table apps
    add column donations_on boolean not null default false,
    -- What the maker suggests, before Slop's fee: the middle preset.
    add column donation_suggested_minor integer not null default 500 check (donation_suggested_minor between 100 and 50000),
    add column donation_currency text not null default 'usd' check (donation_currency in ('usd', 'eur', 'gbp')),
    -- Server donations (PAYMENTS §6): only for apps that declare servers, with what they pay for.
    add column runs_servers boolean not null default false,
    add column servers_statement text check (char_length(servers_statement) <= 300),
    add column servers_shut_down_at timestamptz;

-- Monthly server donations: a Stripe subscription on the maker's account.
create table server_donations (
    id                  uuid primary key,
    app_id              uuid not null references apps (id),
    donor_id            uuid not null references users (id),
    stripe_account_id   text not null,
    currency            text not null,
    donation_minor      integer not null check (donation_minor between 100 and 50000),
    slop_fee_minor      integer not null,
    total_minor         integer not null,
    -- PAY-14: exactly what the donor was shown and confirmed.
    consent_text        text not null,
    status              text not null default 'pending' check (status in ('pending', 'active', 'cancelled')),
    checkout_session_id text unique,
    stripe_subscription_id text unique,
    simulated           boolean not null default false,
    created_at          timestamptz not null default now(),
    started_at          timestamptz,
    next_charge_at      timestamptz,
    cancelled_at        timestamptz,
    cancelled_by        text check (cancelled_by in ('donor', 'servers_shut_down', 'stripe')),
    -- PAY-16: before the first renewal, then once a year.
    first_reminder_at   timestamptz,
    last_yearly_reminder_at timestamptz
);

create unique index server_donations_one_each on server_donations (donor_id, app_id) where status in ('pending', 'active');
create index server_donations_app on server_donations (app_id) where status = 'active';

comment on table server_donations is 'purpose: the donor''s monthly server donation: amounts, consent copy (PAY-14), status; payment records kept 10 years';

-- Every donation payment: one-off, or one month of a server donation.
create table donations (
    id                  uuid primary key,
    kind                text not null check (kind in ('one_off', 'monthly')),
    app_id              uuid not null references apps (id),
    donor_id            uuid not null references users (id),
    server_donation_id  uuid references server_donations (id),
    stripe_account_id   text not null,
    currency            text not null,
    -- What the donor chose for the maker (before Slop's fee), Slop's fee, the processing addition,
    -- and the total charged.
    donation_minor      integer not null check (donation_minor > 0),
    slop_fee_minor      integer not null,
    processing_minor    integer not null,
    total_minor         integer not null,
    donor_covers_fees   boolean not null,
    -- PAY-6: the donor's name and message go to the maker only if the donor chooses.
    share_name          boolean not null default false,
    message             text check (char_length(message) <= 500),
    status              text not null default 'pending'
                        check (status in ('pending', 'paid', 'refunded', 'partially_refunded', 'expired')),
    checkout_session_id text unique,
    -- The charge (one-off) or invoice (monthly).
    stripe_object_id    text unique,
    refunded_minor      integer not null default 0,
    simulated           boolean not null default false,
    created_at          timestamptz not null default now(),
    paid_at             timestamptz
);

create index donations_app on donations (app_id, paid_at) where status <> 'pending';
create index donations_donor on donations (donor_id, created_at desc);

comment on table donations is 'purpose: the donor''s donation history and receipts, the maker''s totals, Slop''s fee records; kept 10 years (LEGAL-GUIDANCE §15)';
comment on column donations.message is 'purpose: a private message to the maker, only when the donor shares it (PAY-6)';

alter table payments drop constraint payments_kind_check;
alter table payments add constraint payments_kind_check check (kind in ('people_job', 'refund', 'dispute', 'donation'));
alter table payments add column donation_id uuid references donations (id);

alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations'));
