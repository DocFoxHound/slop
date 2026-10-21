-- Donations to a community build or mod are shared with the original's maker (decided
-- 2026-10-21): 30% goes to the original as its own direct donation, paid first, then the rest to
-- the branch's author. Two payments, because Slop never holds or splits money (PAYMENTS §5.3).
alter table donations
    -- This donation is the original's share of a donation to that branch.
    add column share_for uuid references apps (id),
    -- This donation to a branch came after that share was paid (each share is used once).
    add column share_donation_id uuid unique references donations (id);

comment on column donations.share_for is 'purpose: links the original''s share to the branch donation it was paid with';
comment on column donations.share_donation_id is 'purpose: the paid share to the original that allowed this branch donation';
