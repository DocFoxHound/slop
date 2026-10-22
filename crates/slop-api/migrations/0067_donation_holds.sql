-- Donation holds (donation_holds.rs): staff stop donations to one app, for example while a legal
-- complaint about it is looked into. 'review' pauses them while Slop looks; 'off' turns them off
-- until the maker fixes the problem. Either way the maker is told why and can ask for a review.
create table donation_holds (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    kind                text not null check (kind in ('review', 'off')),
    -- What's wrong and what would fix it, in words the maker reads.
    reason              text not null check (char_length(reason) between 1 and 4000),
    -- The ground, when there is one: a Content Policy rule, a law, a complaint reference.
    ground              text check (char_length(ground) <= 300),
    placed_by           text not null,
    placed_at           timestamptz not null default now(),
    lifted_by           text,
    lifted_at           timestamptz,
    lift_note           text check (char_length(lift_note) <= 4000)
);

create unique index donation_holds_one_open on donation_holds (app_id) where lifted_at is null;

comment on table donation_holds is
    'purpose: stopping donations to an app while a problem is looked into, and telling its maker why';

-- The maker's requests to lift a hold once they've fixed the problem, and staff's answers.
create table donation_hold_requests (
    id                  uuid primary key,
    hold_id             uuid not null references donation_holds (id) on delete cascade,
    message             text not null check (char_length(message) between 1 and 4000),
    created_at          timestamptz not null default now(),
    -- lifted, or kept (with the answer saying why).
    outcome             text check (outcome in ('lifted', 'kept')),
    answer              text check (char_length(answer) <= 4000),
    answered_by         text,
    answered_at         timestamptz
);

create unique index donation_hold_requests_one_open on donation_hold_requests (hold_id) where outcome is null;

comment on table donation_hold_requests is
    'purpose: the maker asking for donations to be restored after a fix, and the answer';
