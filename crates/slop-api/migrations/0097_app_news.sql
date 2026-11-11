-- Maker news (DESIGN §8; owner decision 2026-11-10). The maker of an app whose page is in the store
-- posts short news on that page (news.rs). People who marked the app Interested are told under the
-- bell, unless they switched news off for that app (interests.news). Bell only: never emailed and
-- never pushed (notify.rs BELL_ONLY; LEGAL-GUIDANCE TERMS-3). Staff can hide a post
-- (moderation.rs hide_news), and an appeal can put it back.

create table app_news (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    -- Plain text on one line. It is the body of the bell notice.
    headline            text not null check (char_length(headline) between 1 and 100
                                             and position(chr(10) in headline) = 0
                                             and position(chr(13) in headline) = 0),
    -- Markdown, shown only through the chat renderer (web/src/markdown.ts renderChat).
    body                text not null check (char_length(body) between 1 and 2000),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    -- Set by a moderation decision (hide_news); cleared when an appeal overturns it.
    hidden_at           timestamptz,
    hidden_reason       text
);

create index app_news_app on app_news (app_id, created_at desc, id desc);

comment on table app_news is 'purpose: a maker''s public news posts on their app''s store page; people who marked the app Interested and left news on are told under the bell (DESIGN §8); hidden ones are deleted 213 days after hiding (news.rs cleanup)';
comment on column app_news.hidden_reason is 'purpose: the rule a moderation decision hid the post under, shown to the maker';

-- One new post per app per 24 hours, also after the post is removed or hidden.
alter table apps add column news_posted_at timestamptz;
comment on column apps.news_posted_at is 'purpose: when the app''s maker last posted news, for the one post a day limit (news.rs)';

-- On for every mark, old and new; the person switches it per app.
alter table interests add column news boolean not null default true;
comment on column interests.news is 'purpose: whether the user is told under the bell about news from this app''s maker; switched per app by the user';
comment on table interests is 'purpose: the user''s Interested list: their library, stage notifications, maker news notifications, recommendations and public counts (only counts are public)';

-- Which post a news notice shows, so the notice goes or is re-worded with exactly that post
-- (notify.rs news_withdrawn, news_reworded, news_gone_for_app). A merged notice names the newest.
alter table notifications add column news_id uuid references app_news (id) on delete cascade;
comment on column notifications.news_id is 'purpose: the maker news post a news notice shows, so removing, hiding or re-wording the post changes the notice too';
create index notifications_news_post on notifications (news_id) where news_id is not null;

-- A reported news post is quoted for moderators like a job message.
comment on column reports.quoted is 'purpose: the reported text as it was (a job message or a news post), for moderators; kept 6 years';

alter table notifications drop constraint notifications_category_check;
alter table notifications add constraint notifications_category_check
    check (category in ('interested', 'jobs', 'moderation', 'donations', 'shared', 'teams', 'apps', 'chat', 'friends', 'ops', 'security', 'agents', 'policies', 'news'));

alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review', 'team', 'chat_message', 'bounty', 'news'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review', 'team', 'chat_message', 'bounty', 'news'));
alter table moderation_decisions drop constraint moderation_decisions_action_check;
alter table moderation_decisions add constraint moderation_decisions_action_check
    check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn', 'hide_team', 'remove_reply', 'hide_bounty', 'hide_news'));
