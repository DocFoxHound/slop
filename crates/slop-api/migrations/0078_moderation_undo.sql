-- Undoing moderation on appeal puts things back as they were (moderation.rs apply, 2026-10-23).

-- A hidden bounty: the status it had and the time it had left, so reversing the decision
-- restores both (at least 7 days, so it doesn't expire at once).
alter table bounties
    add column hidden_from text
        check (hidden_from in ('open', 'claimed', 'paying', 'paid', 'expired')),
    add column hidden_left interval;

-- Pledges released because staff hid the bounty: their backers are told if it comes back.
alter table bounty_pledges add column released_by_hide boolean not null default false;

-- A delisting reversed on appeal while the app couldn't be listed (pictures waiting, no approved
-- release): it waits under Ready to list until a reviewer lists it the usual way.
alter table apps add column relist_after_appeal boolean not null default false;
