-- Catalog: categories, tags and apps (DESIGN §5, §8).

create table categories (
    id                  smallint primary key,
    slug                text not null unique,
    name                text not null,
    description         text not null,
    position            smallint not null
);

create table tags (
    id                  smallint generated always as identity primary key,
    slug                text not null unique,
    name                text not null,
    category_id         smallint not null references categories (id)
);

create index tags_category_id on tags (category_id);

insert into categories (id, slug, name, description, position) values
    (1, 'games', 'Games', 'Things you play', 1),
    (2, 'tools', 'Tools', 'Things that get work done', 2),
    (3, 'creative', 'Creative', 'Make art, music and words', 3),
    (4, 'utilities', 'Utilities', 'Keep your machine happy', 4),
    (5, 'learning', 'Learning', 'Get smarter, slowly', 5),
    (6, 'play-style', 'Play style', 'How you play', 6),
    (7, 'vibe', 'Vibe', 'How it feels', 7);

insert into tags (slug, name, category_id) values
    ('roguelike', 'Roguelike', 1), ('puzzle', 'Puzzle', 1), ('platformer', 'Platformer', 1),
    ('idle', 'Idle', 1), ('simulation', 'Simulation', 1), ('visual-novel', 'Visual Novel', 1),
    ('strategy', 'Strategy', 1), ('racing', 'Racing', 1), ('card-game', 'Card Game', 1),
    ('tycoon', 'Tycoon', 1),
    ('productivity', 'Productivity', 2), ('developer-tools', 'Developer Tools', 2),
    ('file-management', 'File Management', 2), ('note-taking', 'Note-taking', 2),
    ('automation', 'Automation', 2), ('finance', 'Finance', 2),
    ('music', 'Music', 3), ('pixel-art', 'Pixel Art', 3), ('3d', '3D', 3), ('writing', 'Writing', 3),
    ('generative-art', 'Generative Art', 3), ('video', 'Video', 3),
    ('system-monitor', 'System Monitor', 4), ('launcher', 'Launcher', 4), ('backup', 'Backup', 4),
    ('privacy', 'Privacy', 4),
    ('language', 'Language', 5), ('math', 'Math', 5), ('typing', 'Typing', 5), ('kids', 'Kids', 5),
    ('single-player', 'Single-player', 6), ('co-op', 'Co-op', 6), ('multiplayer', 'Multiplayer', 6),
    ('controller-support', 'Controller Support', 6), ('touch-controls', 'Touch Controls', 6),
    ('offline', 'Offline', 6), ('short-sessions', 'Short Sessions', 6),
    ('cozy', 'Cozy', 7), ('retro', 'Retro', 7), ('dark', 'Dark', 7), ('minimalist', 'Minimalist', 7),
    ('chaotic', 'Chaotic', 7), ('relaxing', 'Relaxing', 7), ('funny', 'Funny', 7);

create type app_kind as enum ('game', 'app');
create type app_stage as enum ('coming_soon', 'beta', 'released');
-- draft: only the maker sees it. listed: in the store. delisted: hidden by moderation.
-- Missed stage deadlines and dead projects are computed from dates, not stored (DESIGN §8).
create type app_status as enum ('draft', 'listed', 'delisted');

create table apps (
    id                  uuid primary key,
    slug                text not null unique
                        check (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and char_length(slug) between 3 and 60),
    title               text not null check (char_length(title) between 1 and 80),
    summary             text not null check (char_length(summary) between 1 and 200),
    description_md      text not null default '' check (char_length(description_md) <= 20000),
    kind                app_kind not null,
    stage               app_stage not null,
    stage_entered_at    timestamptz not null default now(),
    status              app_status not null default 'draft',
    maker_id            uuid not null references users (id),
    repo_url            text not null,
    license_spdx        text not null,
    -- Minimum age from the content-rating questionnaire (AGE-7). Moves to releases when
    -- Publishing lands, since content can change between versions.
    min_age             smallint not null default 0 check (min_age in (0, 3, 7, 12, 16, 18)),
    last_commit_at      timestamptz,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    search              tsvector generated always as (
                            setweight(to_tsvector('english', title), 'A') ||
                            setweight(to_tsvector('english', summary), 'B') ||
                            setweight(to_tsvector('english', description_md), 'C')
                        ) stored
);

create index apps_maker_id on apps (maker_id);
create index apps_status_updated on apps (status, updated_at desc);
create index apps_search on apps using gin (search);

create table app_tags (
    app_id              uuid not null references apps (id) on delete cascade,
    tag_id              smallint not null references tags (id),
    primary key (app_id, tag_id)
);

create index app_tags_tag_id on app_tags (tag_id);
