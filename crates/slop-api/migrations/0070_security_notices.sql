-- Security notices (security review, 2026-10-22): a passkey added to the account, a sign-in
-- linked, an account combined into this one. Their own category, so they're emailed even when
-- someone turns the others off.
alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends', 'ops', 'security'));
