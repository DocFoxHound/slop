-- Admin API tokens for agents (agent.rs), 2026-10-20: a third level, "moderate", for suspending
-- and reinstating accounts, withdrawing releases and taking apps out of the store.
alter table admin_tokens drop constraint admin_tokens_scope_check;
alter table admin_tokens add constraint admin_tokens_scope_check check (scope in ('read', 'review', 'moderate'));
