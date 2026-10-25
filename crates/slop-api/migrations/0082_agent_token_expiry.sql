-- Makers' AI agent tokens last up to a year (maker_agents.rs), so their owners are told before
-- one expires: a week before, a day before, and when it has. Each warning is sent once.
alter table maker_tokens add column expiry_warning smallint not null default 0
    check (expiry_warning between 0 and 3);
comment on column maker_tokens.expiry_warning is
    'the last expiry warning sent to the owner: 0 none, 1 a week before, 2 a day before, 3 expired';

-- Their own category, so the owner can switch their emails off.
alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends', 'ops', 'security', 'agents'));
