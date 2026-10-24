-- Profile pictures from a linked sign-in (avatars.rs, built 2026-10-24). Someone picks which
-- linked sign-in's picture to show, or keeps the letter. Slop keeps its own small copy (re-encoded
-- to a 128-pixel PNG) and serves it, so people looking at it never contact Google, Discord or the
-- others. GitHub and GitLab pictures are re-read daily; the others change only at a sign-in.
--
-- The API's own role gets these like every table: slop_grant_app_role (0050) runs after each
-- migration, and its default privileges cover new tables.

-- Where each linked sign-in's picture is, as the provider said at the last sign-in.
alter table identities add column picture_url text check (char_length(picture_url) <= 2048);
comment on column identities.picture_url is 'purpose: offering that sign-in''s picture on the account page and keeping a chosen one current; updated at every sign-in with it; removed when the account closes';

alter table pending_signups add column picture_url text check (char_length(picture_url) <= 2048);
comment on column pending_signups.picture_url is 'purpose: carried into the new account''s sign-in; deleted when the row expires';

-- The picture shown for an account; no row: the letter.
create table user_avatars (
    user_id             uuid primary key references users (id) on delete cascade,
    -- The linked sign-in it comes from. Unlinking that sign-in removes it.
    provider            text not null,
    provider_subject    text not null,
    -- Where Slop reads it (an allowed host of that provider only).
    source_url          text not null check (char_length(source_url) <= 2048),
    -- Slop's copy: a 128×128 PNG, decoded and re-encoded, so no metadata or other content survives.
    image               bytea not null check (octet_length(image) <= 262144),
    -- The start of the copy's SHA-256: its URL changes when the picture does.
    version             text not null,
    -- For conditional requests to the host.
    etag                text,
    last_modified       text,
    fetched_at          timestamptz not null default now(),
    checked_at          timestamptz not null default now(),
    -- Checks in a row that didn't get a picture; the last copy stays meanwhile.
    failures            integer not null default 0,
    foreign key (provider, provider_subject) references identities (provider, provider_subject) on delete cascade
);

create index user_avatars_due on user_avatars (checked_at);
create index user_avatars_identity on user_avatars (provider, provider_subject);

comment on table user_avatars is 'purpose: the profile picture the account chose to show from a linked sign-in, shown wherever its name is; deleted when they switch back to the letter, unlink that sign-in, close the account or staff remove it';
comment on column user_avatars.image is 'purpose: Slop''s re-encoded copy of the picture, served instead of the provider''s, so viewers never contact the provider';
comment on column user_avatars.source_url is 'purpose: where the picture is re-read to keep it current';
