-- Store-page pictures come from the maker's repository and are reviewed; AI reviewers
-- (decided 2026-10-19).
--
-- Pictures are no longer uploaded to Slop (0015, never used). A maker lists them in
-- `.slop/media.json` in their repository; Slop reads that file at one commit, checks the
-- pictures, and links to them at that commit, so what was approved can't change afterwards.
-- Every new set of pictures waits for review (a person, or an AI reviewer with a token) before
-- it shows, and an app can't be put in the store while its pictures are waiting.

drop table app_media;

create table media_sets (
    id                  uuid primary key,
    app_id              uuid not null references apps (id) on delete cascade,
    -- The commit the pictures were read at; their links point at it.
    commit_sha          text not null,
    -- {path, url, sha256, width, height, format}, or null for no thumbnail.
    thumbnail           jsonb,
    -- [{kind: image|video, path?, url?, sha256?, width?, height?, format?, youtube_id?, caption?}]
    gallery             jsonb not null default '[]',
    -- Same pictures and captions give the same fingerprint, in any order: nothing new to review.
    fingerprint         text not null,
    -- pending: waiting for review. approved: was shown (the live one is apps.media_set_id).
    -- rejected: turned down. superseded: replaced by a newer set before it was reviewed.
    status              text not null default 'pending'
                        check (status in ('pending', 'approved', 'rejected', 'superseded')),
    status_reason       text,
    -- maker: they pressed "check". release: read at a release's commit.
    source              text not null check (source in ('maker', 'release')),
    created_at          timestamptz not null default now(),
    reviewed_by         text,
    reviewed_at         timestamptz
);

comment on table media_sets is 'purpose: store-page pictures and videos a maker listed in their repository, and their review';

create unique index media_sets_one_pending on media_sets (app_id) where status = 'pending';
create index media_sets_app on media_sets (app_id, created_at desc);

alter table apps
    -- The approved pictures the store shows. Null: none.
    add column media_set_id uuid references media_sets (id) on delete set null,
    -- The last time Slop read .slop/media.json, and what was wrong if it couldn't use it.
    add column media_checked_at timestamptz,
    add column media_problem text,
    -- The gallery order the maker chose on the website, as item keys ("image:<path>",
    -- "video:<youtube id>"). Items not in it follow in the order media.json lists them.
    -- Changing the order isn't reviewed: it only moves pictures that were.
    add column media_order text[] not null default '{}';

-- Notes reviewers leave on anything waiting for a decision: an AI reviewer's findings for a
-- person to read, or a person's own.
create table review_notes (
    id                  uuid primary key,
    subject_kind        text not null check (subject_kind in ('release', 'change', 'media', 'listing')),
    -- The release, change or media set; for a listing, the app.
    subject_id          uuid not null,
    -- Who wrote it, as the audit log names them.
    author              text not null,
    -- pass: found nothing wrong. concern: found something. unsure: a person should look.
    verdict             text check (verdict in ('pass', 'concern', 'unsure')),
    note                text not null check (char_length(note) <= 4000),
    created_at          timestamptz not null default now()
);

create index review_notes_subject on review_notes (subject_kind, subject_id, created_at);

-- Admins' API tokens, for AI reviewers. Only stored hashed. A token acts for the admin who made
-- it, only on the admin API, and stops working when it expires, is revoked, or its admin loses
-- admin rights.
create table admin_tokens (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    name                text not null check (char_length(name) between 1 and 60),
    -- read: see the queue and leave notes. review: also approve and reject.
    scope               text not null check (scope in ('read', 'review')),
    token_hash          bytea not null unique,
    created_at          timestamptz not null default now(),
    expires_at          timestamptz not null,
    last_used_at        timestamptz,
    revoked_at          timestamptz
);

comment on table admin_tokens is 'purpose: API access for automated reviewers acting for an admin';
comment on column admin_tokens.last_used_at is 'purpose: shows admins which tokens are in use, so unused ones can be revoked';
