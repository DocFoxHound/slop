-- Accessibility details (label.rs, DESIGN §5): what the maker says the app offers, and players
-- who launched it confirming or disputing each.
alter table apps add column accessibility text[] not null default '{}'
    check (accessibility <@ array['remap_controls', 'colorblind', 'subtitles', 'text_size', 'one_handed',
                                  'difficulty', 'no_flashing', 'screen_reader', 'controller']::text[]);

create table accessibility_reports (
    user_id             uuid not null references users (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    feature             text not null,
    -- true: it works as the maker says; false: it doesn't.
    works               boolean not null,
    updated_at          timestamptz not null default now(),
    primary key (user_id, app_id, feature)
);

create index accessibility_reports_app on accessibility_reports (app_id, feature);

comment on table accessibility_reports is 'purpose: a player''s confirmation or dispute of an app''s accessibility feature, counted on its store page';
