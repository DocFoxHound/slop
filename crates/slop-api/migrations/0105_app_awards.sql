-- App of the Month (DESIGN §5, owner decisions 2026-11-19): staff nominate apps and vote, and the
-- owner confirms up to three winners a month. Each winner gets a small cash award that the owner
-- pays by hand as a donation to the app (never from SLAPorSLOP's own Stripe balance, PAY-1/PAY-2);
-- the amount is published with the badge. A month is always the first day of that month (UTC).

-- An app put forward for a month by a staff member.
create table award_nominations (
    month         date not null check (extract(day from month) = 1),
    app_id        uuid not null references apps (id) on delete cascade,
    nominated_by  uuid references users (id) on delete set null,
    -- Why, for the other staff (never shown publicly).
    note          text not null default '' check (char_length(note) <= 500),
    created_at    timestamptz not null default now(),
    primary key (month, app_id)
);
comment on column award_nominations.nominated_by is
    'purpose: which staff member nominated an app for App of the Month (accountability; an admin can''t nominate their own app)';

-- Staff votes on a month's nominations: at most three per staff member a month (awards.rs, under a
-- lock), never for an app they make.
create table award_votes (
    month       date not null,
    app_id      uuid not null,
    voter_id    uuid not null references users (id) on delete cascade,
    created_at  timestamptz not null default now(),
    primary key (month, app_id, voter_id),
    foreign key (month, app_id) references award_nominations (month, app_id) on delete cascade
);
create index award_votes_voter on award_votes (month, voter_id);
comment on column award_votes.voter_id is
    'purpose: which staff member voted for an App of the Month nomination (shown to staff; the three-vote limit)';

-- The month's winners, as the owner confirmed them: at most three (one per position).
create table app_awards (
    month          date not null check (extract(day from month) = 1),
    app_id         uuid not null references apps (id) on delete cascade,
    -- The app's maker when it won: their profile counts the award even if the app changes hands.
    maker_id       uuid not null references users (id) on delete cascade,
    position       smallint not null check (position between 1 and 3),
    -- The published award, which the owner donates to the app by hand.
    amount_minor   integer not null check (amount_minor between 100 and 50000),
    currency       text not null check (currency in ('usd', 'eur', 'gbp')),
    blurb          text not null default '' check (char_length(blurb) <= 160),
    confirmed_by   uuid references users (id) on delete set null,
    confirmed_at   timestamptz not null default now(),
    -- When the owner marked the prize donation made; the month's winners are fixed from then.
    prize_paid_at  timestamptz,
    primary key (month, app_id),
    unique (month, position)
);
create index app_awards_app on app_awards (app_id);
create index app_awards_maker on app_awards (maker_id);
comment on column app_awards.maker_id is
    'purpose: the maker an App of the Month award went to, for the badge on their public profile';
comment on column app_awards.confirmed_by is
    'purpose: which owner confirmed an App of the Month award (accountability)';
