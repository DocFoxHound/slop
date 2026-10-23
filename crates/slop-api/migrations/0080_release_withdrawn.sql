-- Makers withdraw a release of theirs before it's reviewed (bug b21, owner's decision 2026-10-23):
-- one they didn't publish, or one sent by mistake. It never reaches players.
alter type release_status add value 'withdrawn';
