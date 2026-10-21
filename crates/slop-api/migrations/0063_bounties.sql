-- Fix bounties (bounties.rs, plan phase 12): players pledge toward a fix or feature; whoever
-- delivers it is paid straight from each backer's saved card, less Slop's 5%. Slop never holds
-- the money: a pledge is a card saved with Stripe, charged only when a delivery is confirmed.
create table bounties (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    created_by          uuid not null references users (id) on delete cascade,
    title               text not null check (char_length(title) between 1 and 100),
    body                text not null check (char_length(body) between 1 and 2000),
    currency            text not null check (currency in ('usd', 'eur', 'gbp')),
    -- open: taking pledges; claimed: a delivery waits for confirmation; paying: confirmed, cards
    -- being charged; paid: done; expired: nobody delivered in time; hidden: removed by staff.
    status              text not null default 'open'
        check (status in ('open', 'claimed', 'paying', 'paid', 'expired', 'hidden')),
    hidden_reason       text,
    created_at          timestamptz not null default now(),
    expires_at          timestamptz not null default now() + interval '180 days'
);

create index bounties_app on bounties (app_id, created_at desc);
create index bounties_open on bounties (status, created_at desc) where status in ('open', 'claimed');

comment on table bounties is 'purpose: a player''s public request for a fix or feature, with pledges toward it';

create table bounty_pledges (
    id                  uuid primary key,
    bounty_id           uuid not null references bounties (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    amount_minor        integer not null check (amount_minor between 100 and 50000),
    -- saving: at Stripe Checkout; active: card saved; withdrawn; due: to charge; paid; failed:
    -- the card was refused; released: the bounty ended without a delivery.
    status              text not null default 'saving'
        check (status in ('saving', 'active', 'withdrawn', 'due', 'paid', 'failed', 'released')),
    checkout_session_id text,
    payment_method_id   text,
    payment_intent_id   text,
    failure             text,
    simulated           boolean not null default false,
    created_at          timestamptz not null default now(),
    charged_at          timestamptz
);

create unique index bounty_pledges_one_each on bounty_pledges (bounty_id, user_id)
    where status in ('saving', 'active', 'due');

comment on table bounty_pledges is 'purpose: what a backer pledged and the Stripe references to charge it when a delivery is confirmed (PAYMENTS §5.4)';

-- Each backer's Stripe customer on Slop's platform account, holding their saved cards.
create table stripe_customers (
    user_id             uuid primary key references users (id) on delete cascade,
    customer_id         text not null unique,
    created_at          timestamptz not null default now()
);

comment on table stripe_customers is 'purpose: the Stripe customer that holds a backer''s saved card for bounty pledges; card details stay at Stripe';

create table bounty_claims (
    id                  uuid primary key,
    bounty_id           uuid not null references bounties (id) on delete cascade,
    claimed_by          uuid not null references users (id) on delete cascade,
    -- Where it's delivered: the app itself or a community build of it.
    app_id              uuid not null references apps (id) on delete cascade,
    release_id          uuid not null references releases (id) on delete cascade,
    -- The builder's Stripe account, paid directly.
    stripe_account_id   text not null,
    note                text not null check (char_length(note) between 1 and 2000),
    -- pending, accepted (by the app's maker or the backers), declined, withdrawn.
    status              text not null default 'pending'
        check (status in ('pending', 'accepted', 'declined', 'withdrawn')),
    decided_by          text check (decided_by in ('maker', 'backers')),
    created_at          timestamptz not null default now(),
    decided_at          timestamptz
);

create unique index bounty_claims_one_pending on bounty_claims (bounty_id) where status = 'pending';

comment on column bounty_claims.note is 'purpose: the builder''s explanation of what they delivered, shown on the bounty';

-- Backers' votes on a claim, when the app's maker doesn't answer or made the claim themselves.
create table bounty_votes (
    claim_id            uuid not null references bounty_claims (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    accept              boolean not null,
    created_at          timestamptz not null default now(),
    primary key (claim_id, user_id)
);

comment on table bounty_votes is 'purpose: a backer''s vote on whether a claimed delivery did what the bounty asked';

alter table payments drop constraint payments_kind_check;
alter table payments add constraint payments_kind_check
    check (kind in ('people_job', 'refund', 'dispute', 'donation', 'bounty'));
alter table payments add column bounty_pledge_id uuid references bounty_pledges (id);

alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review', 'team', 'chat_message', 'bounty'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review', 'team', 'chat_message', 'bounty'));
alter table moderation_decisions drop constraint moderation_decisions_action_check;
alter table moderation_decisions add constraint moderation_decisions_action_check
    check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn', 'hide_team', 'remove_reply', 'hide_bounty'));
