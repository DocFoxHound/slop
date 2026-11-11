-- Open requests (item 14): a board inside People. A maker says what work one of their store
-- projects needs (one skill, a title, a description, an optional date; no budget), and creatives
-- who can be hired put their name forward: a bare pointer to their People page, optionally naming
-- one of their own packages. Nothing here creates a job: hiring stays the hire request (jobs).
--
-- A request leaves the board when its maker closes it, after 30 days (people/work_requests.rs
-- `expire`, called hourly from cleanup.rs), when staff hide it (moderation.rs hide_work_request;
-- an appeal can put it back), or while its project is not in the store. Ended requests are
-- deleted 90 days after they end; hidden ones once the appeal window is over and no appeal about
-- them waits.
--
-- Also here: report kind `work_request` and decision action `hide_work_request`, and a new use of
-- blocks: two people who blocked one another are kept apart on the board.

create table work_requests (
    id              uuid primary key,
    user_id         uuid not null references users (id) on delete cascade,
    app_id          uuid not null references apps (id) on delete cascade,
    -- A slug from people::SKILLS (the list lives in code, as for creatives.skills).
    skill           text not null check (char_length(skill) between 1 and 40),
    title           text not null check (char_length(title) between 1 and 100),
    description_md  text not null check (char_length(description_md) between 1 and 2000),
    wanted_by       date,
    status          text not null default 'open' check (status in ('open', 'closed', 'expired')),
    created_at      timestamptz not null default now(),
    expires_at      timestamptz not null default (now() + interval '30 days'),
    ended_at        timestamptz,
    -- Set by a moderation decision (hide_work_request); cleared when an appeal overturns it.
    hidden_reason   text,
    hidden_at       timestamptz,
    constraint work_requests_ended check ((status = 'open') = (ended_at is null)),
    constraint work_requests_hidden check ((hidden_reason is null) = (hidden_at is null))
);

-- The board: open requests, newest first.
create index work_requests_board on work_requests (created_at desc, id desc)
    where status = 'open' and hidden_reason is null;
-- "Your requests", the cap of three, export, closing.
create index work_requests_user on work_requests (user_id, created_at desc);
create index work_requests_app on work_requests (app_id);

create table work_request_responses (
    request_id   uuid not null references work_requests (id) on delete cascade,
    creative_id  uuid not null references users (id) on delete cascade,
    -- One of the creative's own packages (creatives.services[].id, 8 hex characters), if named.
    service_id   text check (char_length(service_id) = 8),
    created_at   timestamptz not null default now(),
    -- Taken back: hidden from the maker, kept so that putting the name forward again tells nobody.
    withdrawn_at timestamptz,
    primary key (request_id, creative_id)
);

create index work_request_responses_creative on work_request_responses (creative_id, created_at desc);

comment on table work_requests is
    'purpose: open requests on People: what a maker says a project needs, shown publicly with the maker''s display name and the project; deleted 90 days after it ends (a hidden one 213 days after it was hidden, unless an appeal about it waits), or when the account closes';
comment on column work_requests.user_id is
    'purpose: the maker who posted the request; their display name is shown with it';
comment on column work_requests.title is
    'purpose: the request''s public title, written by the maker';
comment on column work_requests.description_md is
    'purpose: the request''s public text, written by the maker';
comment on column work_requests.wanted_by is
    'purpose: the day the maker hopes to have the work, shown with the request';
comment on column work_requests.hidden_reason is
    'purpose: why staff hid the request (the rule''s title), shown to its maker; kept through the appeal window';
comment on column work_requests.hidden_at is
    'purpose: when staff hid the request, to delete it once the appeal window is over and no appeal about it waits';

comment on table work_request_responses is
    'purpose: which creatives put their name forward for an open request, shown only to the request''s maker and an AI agent the maker gave People access; a name taken back leaves the maker''s list at once and the row is kept until the request is deleted; deleted with the request or when either account closes';
comment on column work_request_responses.creative_id is
    'purpose: the creative who put their name forward; the maker sees a link to their People page';
comment on column work_request_responses.service_id is
    'purpose: the package the creative named, shown to the maker and carried into the hire form';
comment on column work_request_responses.created_at is
    'purpose: when the name was put forward: the only order the maker''s list has';
comment on column work_request_responses.withdrawn_at is
    'purpose: when the creative took their name back: the row is hidden from the request''s maker at once and kept, so that putting the name forward again does not notify the maker a second time; deleted with the request';

-- Blocks get a new use (people/work_requests.rs push_not_blocked).
comment on table user_blocks is
    'purpose: people the user blocked: they can''t message or befriend the user, and on People neither is shown the other''s open requests or can put their name forward for them';

-- Notifications get a new use: the maker is told, under People jobs, that names were put forward.
comment on table notifications is
    'purpose: telling the user about their jobs, names put forward for their open requests on People, Interested apps and moderation decisions; deleted 180 days after being read';

comment on column reports.quoted is
    'purpose: the reported text as it was (a job message, a news post or an open request), for moderators; kept 6 years';

-- Reports and decisions learn the new kind and action. Lists as they stand after 0097 (item 10),
-- plus work_request / hide_work_request. notifications_category_check is not touched: the notice
-- to the maker uses the existing jobs category.
alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review', 'team', 'chat_message', 'bounty', 'news', 'work_request'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review', 'team', 'chat_message', 'bounty', 'news', 'work_request'));
alter table moderation_decisions drop constraint moderation_decisions_action_check;
alter table moderation_decisions add constraint moderation_decisions_action_check
    check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn', 'hide_team', 'remove_reply', 'hide_bounty', 'hide_news', 'hide_work_request'));
