-- Checks, then Publish (2026-11). Every release goes through SLAPorSLOP's automatic checks and
-- ends 'ready' (its maker presses Publish), on hold ('awaiting_review': a scanner or Microsoft
-- Defender flagged a file, Defender's check is missing, or it lowers the age rating) or 'failed'. Publishing makes it
-- 'approved': players get it. A hold has a log: SLAPorSLOP's own lines, and a conversation between
-- the maker and SLAPorSLOP staff.

-- When a release was published and by whom. "What players get" orders by published_at: checks
-- don't finish in the order releases were sent, so a held 1.3 cleared after 1.4 passed must not
-- jump ahead of it.
alter table releases
    add column published_at timestamptz,
    add column published_by text;
comment on column releases.published_at is
    'purpose: when its maker published the release (players get the newest published one)';
comment on column releases.published_by is
    'purpose: accountability: who published the release (the maker''s account ID)';

-- Releases approved before this change were published when they were approved.
update releases set published_at = coalesce(reviewed_at, created_at)
where status in ('approved', 'revoked');

-- A release written as 'approved' without a publish time (simulations, seeds, test fixtures, and
-- any older code path) counts as published when it was reviewed, or now.
create function releases_published_at() returns trigger language plpgsql as $$
begin
    if new.status = 'approved' and new.published_at is null then
        new.published_at := coalesce(new.reviewed_at, now());
    end if;
    return new;
end $$;

create trigger releases_published_at before insert or update on releases
    for each row execute function releases_published_at();

create index releases_published on releases (app_id, published_at desc) where status = 'approved';

comment on column apps.auto_update is
    'no longer used (2026-11): every release is published by its maker';

-- A release that lowers the age rating (or drops a content label) from the last published one is
-- held for SLAPorSLOP staff (AGE-7: a lowered rating waits for a person). Staff accepting it is
-- recorded on the release, so its checks can end ready and Publish can check it was accepted.
alter table releases
    add column rating_accepted_at timestamptz,
    add column rating_accepted_by text;
comment on column releases.rating_accepted_at is
    'purpose: when SLAPorSLOP staff accepted the lower age rating this release brings (AGE-7)';
comment on column releases.rating_accepted_by is
    'purpose: accountability: which staff member (name and account ID) accepted the lower rating';
-- What staff accepted: the rating players were shown then (from), the one they'd be shown after
-- publishing (to) and the content labels it drops. A change worse than that (the published
-- rating went up meanwhile, say) is held again.
alter table releases
    add column rating_accepted_from smallint,
    add column rating_accepted_to smallint,
    add column rating_accepted_labels text[];
comment on column releases.rating_accepted_from is
    'purpose: the age rating players were shown when SLAPorSLOP staff accepted this release''s lower one';
comment on column releases.rating_accepted_to is
    'purpose: the age rating SLAPorSLOP staff accepted for this release';
comment on column releases.rating_accepted_labels is
    'purpose: the content labels SLAPorSLOP staff accepted this release dropping';

-- A store rating SLAPorSLOP staff set (admin console): makers can't lower the store page below
-- it, and publishing a release rated lower keeps it (AGE-7).
alter table apps
    add column min_age_floor smallint check (min_age_floor in (0, 3, 7, 12, 16, 18)),
    add column content_labels_floor text[] not null default '{}'
        check (content_labels_floor <@ array['romance', 'suggestive', 'violence', 'horror',
                                             'strong_language', 'gambling_themes']::text[]);
comment on column apps.min_age_floor is
    'purpose: the lowest age rating the store page may show, set when SLAPorSLOP staff change it (AGE-7)';
comment on column apps.content_labels_floor is
    'purpose: content labels the store page keeps, set when SLAPorSLOP staff change them (AGE-7)';

-- How many times the fetcher has started checking a release: one that keeps failing after its
-- files were fetched stops after 3 tries, failed with a plain reason, instead of being fetched
-- again forever.
alter table releases add column fetch_attempts smallint not null default 0;

-- Cleared without a reason: a staff member may clear a finding as a false positive with no note,
-- so "cleared" is the time, not the reason. Keeping a re-scanned live release records who kept it.
alter table release_files
    add column rescan_cleared_at timestamptz,
    add column rescan_cleared_by text;
comment on column release_files.rescan_cleared_at is
    'purpose: when a reviewer kept a live release a re-scan flagged (judged a false positive)';
comment on column release_files.rescan_cleared_by is
    'purpose: accountability for a reviewer keeping a flagged release (RECORD-OF-PROCESSING)';
update release_files set cleared_at = coalesce(cleared_at, now())
where cleared_at is null and (cleared_reason is not null or cleared_by is not null);
update release_files set rescan_cleared_at = now()
where rescan_cleared_at is null and rescan_cleared_reason is not null;

-- Microsoft Defender is judged apart from SLAPorSLOP's scanner. `cleared_at` is staff judging a
-- file's findings a false positive; `defender_cleared_at` is set with it when Defender had
-- flagged that file then (a Defender report that flags it later still holds it, and a report
-- with detections is never replaced, slop_build.rs). A missing Defender check is never cleared:
-- staff waive it explicitly, per file, signed in themselves (`defender_waived_at`), for a
-- repository Slop Check can't run in (it runs on GitHub Actions only).
alter table release_files
    add column defender_cleared_at timestamptz,
    add column defender_waived_at timestamptz,
    add column defender_waived_by text;
comment on column release_files.defender_cleared_at is
    'purpose: when SLAPorSLOP staff judged Microsoft Defender''s detections in this file a false positive';
comment on column release_files.defender_waived_at is
    'purpose: when SLAPorSLOP staff waived a missing Microsoft Defender check of this file';
comment on column release_files.defender_waived_by is
    'purpose: accountability: which staff member (name and account ID) waived the Defender check';
-- Before, clearing a Windows file covered Defender too, flagged or missing: those decisions stand.
update release_files set defender_cleared_at = cleared_at
where cleared_at is not null and platform = 'windows'
  and jsonb_array_length(coalesce(defender -> 'detections', '[]')) > 0;
update release_files set defender_waived_at = cleared_at, defender_waived_by = cleared_by
where cleared_at is not null and platform = 'windows'
  and (defender is null or lower(defender ->> 'sha256') is distinct from lower(sha256));

-- A release on hold, or a failed release whose maker asked SLAPorSLOP staff about it: one row per
-- release. `kinds` says what holds it; `needs_staff` puts it in the staff queue (a scanner or
-- Defender finding, or the maker writing to staff). The maker can have SLAPorSLOP run its own
-- scanner again on the same files, a few times, spaced out.
create table release_holds (
    release_id          uuid primary key references releases (id) on delete cascade,
    app_id              uuid not null references apps (id) on delete cascade,
    kinds               text[] not null check (
                            cardinality(kinds) > 0
                            and kinds <@ array['scan', 'defender', 'defender_missing',
                                               'rating_lowered', 'question']),
    status              text not null default 'open' check (
                            status in ('open', 'passed', 'cleared', 'rejected', 'replaced',
                                       'withdrawn', 'answered')),
    needs_staff         boolean not null,
    maker_asked_at      timestamptz,
    rescans_used        smallint not null default 0 check (rescans_used between 0 and 3),
    rescan_requested_at timestamptz,
    rescan_started_at   timestamptz,
    -- When a re-run last started (spacing counts from it, whether or not it got to scan).
    last_rescan_at      timestamptz,
    -- A re-run that couldn't download anything is given back, once per hold.
    rescan_given_back   boolean not null default false,
    opened_at           timestamptz not null default now(),
    closed_at           timestamptz,
    closed_by           text,
    constraint release_holds_closed check ((status = 'open') = (closed_at is null))
);
comment on table release_holds is
    'purpose: what holds a release back from its maker publishing it, and whether SLAPorSLOP staff need to look; kept with its release and deleted with the app';
comment on column release_holds.maker_asked_at is
    'purpose: when the maker last wrote to SLAPorSLOP staff about the release (orders the staff queue)';
comment on column release_holds.closed_by is
    'purpose: accountability: who or what closed the hold (a staff member''s name and account ID, or slop)';

-- The staff queue: open holds that need staff, the maker's questions first, then the oldest.
create index release_holds_staff on release_holds (maker_asked_at desc nulls last, opened_at)
    where status = 'open' and needs_staff;
-- Re-runs of the scanner the maker asked for, waiting for the fetcher.
create index release_holds_rescan on release_holds (rescan_requested_at)
    where rescan_requested_at is not null;
create index release_holds_app on release_holds (app_id);

-- A hold's log: what SLAPorSLOP did (system lines, author_id null) and the conversation between
-- the maker and SLAPorSLOP staff. Plain text, shown as text, never HTML.
create table release_hold_messages (
    id                  uuid primary key,
    release_id          uuid not null references release_holds (release_id) on delete cascade,
    -- The maker or the staff member who wrote it; null for SLAPorSLOP's own lines (and for a
    -- message whose author's account was closed).
    author_id           uuid references users (id) on delete set null,
    from_staff          boolean not null,
    -- Written by the maker's AI agent (a publish token), shown to staff as such.
    by_agent            boolean not null default false,
    kind                text not null check (kind in ('message', 'system')),
    body                text not null check (char_length(body) between 1 and 4000),
    -- A system line's facts, e.g. a re-scan's verdicts before and after.
    detail              jsonb,
    created_at          timestamptz not null default now()
);
comment on table release_hold_messages is
    'purpose: the log of a held or failed release: SLAPorSLOP''s own lines and the conversation between its maker and SLAPorSLOP staff (human review on request, CON-16); kept with the release, deleted with the app, the maker''s own messages blanked when their account closes, and in the account export';
comment on column release_hold_messages.author_id is
    'purpose: who wrote the message (the maker or a staff member); null for SLAPorSLOP''s own lines';
comment on column release_hold_messages.body is
    'purpose: the message as its author wrote it (plain text), or SLAPorSLOP''s line';

create index release_hold_messages_release on release_hold_messages (release_id, created_at);
create index release_hold_messages_author on release_hold_messages (author_id) where author_id is not null;

-- Every re-run of the scanner on a held release, for a cap across SLAPorSLOP (each one downloads
-- the files again and keeps the scanner busy). Old rows are deleted (cleanup.rs).
create table release_rescan_runs (
    id                  bigint generated always as identity primary key,
    release_id          uuid not null references releases (id) on delete cascade,
    started_at          timestamptz not null default now()
);
comment on table release_rescan_runs is
    'purpose: counts re-runs of the scanner on held releases, for a daily cap; kept two days';
create index release_rescan_runs_started on release_rescan_runs (started_at);
