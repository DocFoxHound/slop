-- Changed terms and policies are announced once, in the bell (and by email, unless switched off),
-- instead of a banner on every page that each person had to dismiss, browser by browser
-- (owner request 2026-10-30). legal.rs `announce` sends them and marks each version announced;
-- versions already published are announced on the next start-up, if they're still news.
alter table legal_documents add column announced_at timestamptz;
comment on column legal_documents.announced_at is
    'purpose: when the people this version concerns were sent its notification (legal.rs announce); null: not yet';

alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends', 'ops', 'security', 'agents', 'policies'));
