-- The app maker's public reply to a review (reviews.rs), and staff removing one (moderation.rs).
alter table app_reviews
    add column reply text check (char_length(reply) between 1 and 2000),
    add column reply_at timestamptz,
    add column reply_hidden_at timestamptz,
    add column reply_hidden_reason text;

comment on column app_reviews.reply is 'purpose: the app maker''s public reply under the review (DESIGN §7)';

alter table moderation_decisions drop constraint moderation_decisions_action_check;
alter table moderation_decisions add constraint moderation_decisions_action_check
    check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn', 'hide_team', 'remove_reply'));
