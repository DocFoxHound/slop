-- People: creatives' profiles and portfolios, jobs with their Service Agreement, payments and
-- verified reviews (DESIGN §15, PAYMENTS §7, LEGAL §4, LEGAL-GUIDANCE §4).

-- The EU/UK consumer withdrawal period for distance service contracts (CRD Art. 9, UK CCR
-- 2013), per country (AGE-1: legal rules live in this table). Null: none.
alter table country_policies add column consumer_withdrawal_days smallint
    check (consumer_withdrawal_days between 1 and 60);
update country_policies set consumer_withdrawal_days = 14 where country_code <> 'US';

alter table users add column adult_confirmed_at timestamptz;
comment on column users.adult_confirmed_at is
    'purpose: proof the user confirmed they are 18+ before selling or hiring on People (LEGAL §6); no date of birth is stored';

-- review_notes can be about a portfolio too.
alter table review_notes drop constraint review_notes_subject_kind_check;
alter table review_notes add constraint review_notes_subject_kind_check
    check (subject_kind in ('release', 'change', 'media', 'listing', 'portfolio'));

create table creatives (
    user_id             uuid primary key references users (id) on delete cascade,
    handle              text not null unique
                        check (handle ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and char_length(handle) between 3 and 40),
    name                text not null check (char_length(name) between 1 and 60),
    kind                text not null default 'individual' check (kind in ('individual', 'studio')),
    team_size           smallint check (team_size between 2 and 500),
    headline            text not null default '' check (char_length(headline) <= 140),
    about_md            text not null default '' check (char_length(about_md) <= 6000),
    location            text not null default '' check (char_length(location) <= 60),
    skills              text[] not null default '{}',
    -- Required disclosure (DESIGN §15.1): none, assisted or heavy.
    ai_use              text not null check (ai_use in ('none', 'assisted', 'heavy')),
    ai_note             text not null default '' check (char_length(ai_note) <= 300),
    -- Offering services as a business (HUM-1).
    trader              boolean not null,
    taking_work         boolean not null default true,
    turnaround          text not null default '' check (char_length(turnaround) <= 40),
    -- Packages: [{id, name, description, price_minor, currency, days, skill}], up to 8.
    services            jsonb not null default '[]',
    -- Page customisation: a palette name, the section order, and links [{label, url}].
    accent              text not null default 'slime',
    sections            text[] not null default '{portfolio,services,reviews,worked_on,about}',
    links               jsonb not null default '[]',
    -- The portfolio, from a public GitHub repository (people/portfolio.rs).
    portfolio_repo      text,
    portfolio_set_id    uuid,
    portfolio_checked_at timestamptz,
    portfolio_problem   text,
    portfolio_order     text[] not null default '{}',
    -- Stripe Connect (PAYMENTS §4): only the account ID and its status.
    stripe_account_id   text unique,
    stripe_status       text not null default 'none'
                        check (stripe_status in ('none', 'pending', 'active', 'restricted')),
    -- Moderation (rule-breaking only, LEGAL §4.1).
    hidden_reason       text,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

comment on column creatives.name is 'purpose: shown on the creative''s public profile, credits and jobs';
comment on column creatives.headline is 'purpose: shown on the public profile';
comment on column creatives.about_md is 'purpose: shown on the public profile';
comment on column creatives.location is 'purpose: optional, shown on the public profile; chosen by the creative';
comment on column creatives.links is 'purpose: the creative''s own links, shown on the public profile';
comment on column creatives.trader is 'purpose: the trader declaration shown to clients (CRD Art. 6a, HUM-1)';
comment on column creatives.stripe_account_id is 'purpose: paying the creative directly (PAYMENTS §3); Stripe holds identity, tax and bank details';

create table portfolio_sets (
    id                  uuid primary key,
    creative_id         uuid not null references creatives (user_id) on delete cascade,
    repo_url            text not null,
    commit_sha          text not null check (commit_sha ~ '^[0-9a-f]{40}$'),
    banner              jsonb,
    avatar              jsonb,
    works               jsonb not null,
    fingerprint         text not null,
    status              text not null default 'pending'
                        check (status in ('pending', 'approved', 'rejected', 'superseded')),
    status_reason       text,
    source              text not null check (source in ('creative', 'sim')),
    created_at          timestamptz not null default now(),
    reviewed_by         text,
    reviewed_at         timestamptz
);

create unique index portfolio_sets_one_pending on portfolio_sets (creative_id) where status = 'pending';
create index portfolio_sets_creative on portfolio_sets (creative_id, created_at desc);

alter table creatives add constraint creatives_portfolio_set_fk
    foreign key (portfolio_set_id) references portfolio_sets (id);

create table jobs (
    id                  uuid primary key,
    creative_id         uuid not null references creatives (user_id),
    client_id           uuid not null references users (id),
    app_id              uuid not null references apps (id),
    skill               text not null,
    status              text not null check (status in (
                            'requested', 'offered', 'awaiting_payment', 'in_progress', 'delivered',
                            'completed', 'declined', 'cancelled', 'withdrawn')),
    -- What the client asked for: part of the contract record (HUM-17).
    request_md          text not null check (char_length(request_md) between 1 and 4000),
    wanted_by           date,
    -- The client said they're hiring as a business (no consumer withdrawal right).
    client_is_business  boolean not null default false,
    -- The current offer's terms (people/jobs.rs Offer) and how many times it was sent.
    offer               jsonb,
    offer_count         integer not null default 0,
    -- The agreed Service Agreement (HUM-16): exact text, its hash and template version.
    agreement_text      text,
    agreement_sha256    text,
    agreement_template  text,
    offered_at          timestamptz,
    accepted_at         timestamptz,
    -- EU/UK consumer withdrawal (HUM-2, HUM-3).
    withdrawal_until    timestamptz,
    start_now_ack_at    timestamptz,
    withdrawn_at        timestamptz,
    started_at          timestamptz,
    delivered_at        timestamptz,
    revisions_used      smallint not null default 0,
    completed_at        timestamptz,
    completed_automatically boolean not null default false,
    -- Set when the job is completed and fully paid: the credit and badge (HUM-14).
    credited_at         timestamptz,
    ended_at            timestamptz,
    ended_by            uuid references users (id),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    check (creative_id <> client_id)
);

create index jobs_creative on jobs (creative_id, updated_at desc);
create index jobs_client on jobs (client_id, updated_at desc);
create index jobs_app_credited on jobs (app_id) where credited_at is not null;
create index jobs_delivered on jobs (delivered_at) where status = 'delivered';

comment on column jobs.request_md is 'purpose: the client''s request, part of the contract record (HUM-16, kept 6 years after the account ends)';
comment on column jobs.agreement_text is 'purpose: the Service Agreement both sides accepted (HUM-16)';

-- The job's timeline: every step, with the short notes that go with it (HUM-17).
create table job_events (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    -- Null: Slop did it (automatic completion, a payment arriving).
    actor_id            uuid references users (id),
    kind                text not null,
    note                text check (char_length(note) <= 2000),
    links               text[] not null default '{}',
    offer               jsonb,
    created_at          timestamptz not null default now()
);

create index job_events_job on job_events (job_id, created_at);

comment on column job_events.note is 'purpose: contract record of the step (change requests, delivery notes); not chat (HUM-17)';
comment on column job_events.links is 'purpose: where the delivered files are, as given by the creative';

-- What's owed for a job: one payment, or two for "half now, half on delivery".
create table job_payments (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    part                text not null check (part in ('full', 'deposit', 'final')),
    -- When it's due: on accepting, or on delivery.
    due_on              text not null check (due_on in ('accept', 'delivery')),
    amount_minor        integer not null check (amount_minor > 0),
    currency            text not null check (currency in ('usd', 'eur', 'gbp')),
    status              text not null default 'waiting' check (status in ('waiting', 'due', 'paid')),
    checkout_session_id text unique,
    paid_at             timestamptz,
    unique (job_id, part)
);

-- Mirror of money that moved through Stripe (PAYMENTS §8). Only ever added to.
create table payments (
    id                  uuid primary key,
    kind                text not null check (kind in ('people_job', 'refund', 'dispute')),
    job_payment_id      uuid references job_payments (id),
    refund_of           uuid references payments (id),
    stripe_account_id   text,
    stripe_object_id    text unique,
    payer_id            uuid references users (id),
    amount_minor        integer not null,
    slop_fee_minor      integer not null default 0,
    currency            text not null,
    -- Made in the simulation sandbox or a development server without Stripe.
    simulated           boolean not null default false,
    created_at          timestamptz not null default now()
);

create index payments_job_payment on payments (job_payment_id);

comment on column payments.payer_id is 'purpose: who paid, for the job record, refunds and fraud checks (PAYMENTS §8–9)';

-- Stripe webhook events already handled (idempotency, PAYMENTS §8).
create table stripe_events (
    id                  text primary key,
    kind                text not null,
    received_at         timestamptz not null default now()
);

-- Verified reviews: one per completed job (HUM-11). The rating counts each client's newest.
create table creative_reviews (
    job_id              uuid primary key references jobs (id) on delete cascade,
    creative_id         uuid not null references creatives (user_id) on delete cascade,
    client_id           uuid not null references users (id) on delete cascade,
    vote                smallint not null check (vote in (-1, 1)),
    body                text not null default '' check (char_length(body) <= 2000),
    reply               text check (char_length(reply) <= 1000),
    replied_at          timestamptz,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

create index creative_reviews_creative on creative_reviews (creative_id, created_at desc);

comment on column creative_reviews.body is 'purpose: the client''s public review of the creative''s service (LEGAL §4.5)';
comment on column creative_reviews.reply is 'purpose: the creative''s public reply (LEGAL §4.5)';
