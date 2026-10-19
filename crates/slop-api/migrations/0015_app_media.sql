-- Store-page pictures and videos (decided 2026-10-19).
--
-- A maker can set a thumbnail (the store tile, cropped to 460:215 like a Steam capsule) and a
-- gallery of screenshots and videos shown as a carousel on the app's page. Pictures are
-- re-encoded by Slop, which drops their metadata (SECURITY §8). Videos are YouTube links; Slop
-- keeps a copy of the video's poster so nothing loads from YouTube until a player presses play.
-- Store-page content: changes go live straight away, like the description.

create table app_media (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    -- thumbnail: the store tile. image, video: gallery items, in `position` order.
    role                text not null check (role in ('thumbnail', 'image', 'video')),
    position            smallint not null default 0,
    -- role = video: the YouTube video.
    youtube_id          text check (youtube_id ~ '^[A-Za-z0-9_-]{11}$'),
    caption             text check (char_length(caption) <= 200),
    -- The picture (for a video, its poster), as Slop re-encoded it.
    content_type        text not null check (content_type in ('image/png', 'image/jpeg')),
    bytes               bytea not null,
    width               integer not null,
    height              integer not null,
    created_at          timestamptz not null default now(),
    check ((role = 'video') = (youtube_id is not null))
);

comment on table app_media is 'purpose: store-page thumbnails, screenshots and video posters uploaded by makers';

create unique index app_media_one_thumbnail on app_media (app_id) where role = 'thumbnail';
create index app_media_gallery on app_media (app_id, position) where role <> 'thumbnail';
