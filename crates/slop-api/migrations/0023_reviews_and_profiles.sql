-- Written reviews with votes, and public profiles (DESIGN §7; LEGAL §7; LEGAL-GUIDANCE PRIV-6,
-- REV-2). Built 2026-10-20.

-- An optional written review alongside a player's 👍/👎. Shown only while they have a vote on the
-- app. Never hidden for being negative (REV-2): only by a moderation decision.
create table app_reviews (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    body                text not null check (char_length(body) between 1 and 4000),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    hidden_at           timestamptz,
    hidden_reason       text,
    unique (user_id, app_id)
);

create index app_reviews_app on app_reviews (app_id, updated_at desc) where hidden_at is null;

comment on table app_reviews is 'purpose: the player''s written review on an app page and, when their profile is public, on their profile';

-- Votes and reviews are public on profiles by default; null means that default, except in the UK
-- under-18 view, where they're private by default (PRIV-6).
alter table users add column profile_public boolean;
comment on column users.profile_public is 'purpose: the user''s choice to show their votes and reviews on their public profile';

-- Reports and decisions can be about an app review too.
alter table reports drop constraint reports_subject_kind_check;
alter table reports add constraint reports_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'app_review'));
alter table moderation_decisions drop constraint moderation_decisions_subject_kind_check;
alter table moderation_decisions add constraint moderation_decisions_subject_kind_check
    check (subject_kind in ('app', 'review', 'profile', 'message', 'account', 'app_review'));
