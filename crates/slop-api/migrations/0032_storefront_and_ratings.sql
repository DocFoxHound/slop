-- The store front page and ratings for makers (DESIGN §5, METRICS §3, §5), built 2026-10-20.

-- The Featured carousel: apps staff pick for the top of the front page, in order.
create table featured_apps (
    app_id              uuid primary key references apps (id) on delete cascade,
    position            integer not null,
    -- One line saying why it's featured, shown on the slide.
    blurb               text not null default '' check (char_length(blurb) <= 160),
    added_by            uuid references users (id) on delete set null,
    created_at          timestamptz not null default now()
);

comment on column featured_apps.added_by is 'purpose: which staff member featured an app (accountability)';

-- Content labels for themes that are allowed but some players want to know about (DESIGN §5:
-- sexually explicit content is banned; non-explicit romance and suggestive themes are labelled).
alter table apps add column content_labels text[] not null default '{}'
    check (content_labels <@ array['romance', 'suggestive', 'violence', 'horror', 'strong_language', 'gambling_themes']::text[]);

-- The platform a player last launched an app on, so votes can be broken down by platform for
-- the maker (METRICS §3).
alter table user_app_usage add column last_platform text check (last_platform in ('windows', 'linux'));
alter table vote_events add column platform text check (platform in ('windows', 'linux'));

comment on column user_app_usage.last_platform is 'purpose: the platform a vote was cast from, for the maker''s rating breakdown (aggregated only)';

create index library_entries_added on library_entries (app_id, added_at);
create index votes_app_updated on votes (app_id, updated_at);
