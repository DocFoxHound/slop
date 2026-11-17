-- Checks, then Publish (2026-11): a release that passed every check waits for its maker to press
-- Publish. 'ready' means exactly that: nothing is signed yet and players can't get it.
-- 'approved' now means published. Like 0080, the new value can't be used in the transaction that
-- adds it, so it has a migration of its own.
alter type release_status add value 'ready';
