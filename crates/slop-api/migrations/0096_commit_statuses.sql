-- SLAPorSLOP's result on the maker's GitHub commit (owner decision 2026-11-10): for a public app
-- in the store whose maker switched it on, each state of a release is posted once as a commit
-- status named "SLAPorSLOP" on the release's commit, with the GitHub App's installation token
-- only. Off for the whole server unless SLOP_GITHUB_STATUSES is set. commit_status.rs compares
-- what each commit should say with what was last posted, so nothing that changes a release's
-- state (the reviewers' pages, the staff command line, the fetcher) has to tell it.

-- A row means the maker switched it on for this app at some time. Switching it off sets
-- switched_off_at and keeps the row, so a "Passed" that is later withdrawn can still be
-- corrected on GitHub (commit_status.rs). Who switched it is in audit_log (app.commit_status),
-- so no account is named here.
create table app_commit_status (
    app_id                uuid primary key references apps (id) on delete cascade,
    switched_on_at        timestamptz not null default now(),
    switched_off_at       timestamptz,
    permission            boolean,
    permission_checked_at timestamptz,
    wait_until            timestamptz,
    problem_code          integer check (problem_code between 0 and 599),
    problem_at            timestamptz,
    check ((problem_code is null) = (problem_at is null))
);
comment on table app_commit_status is
    'purpose: the apps whose maker switched on SLAPorSLOP''s result as a commit status on GitHub; kept after switching off so a passed result that is later withdrawn can be corrected; deleted with the app, when its repository address changes, or when the maker''s account closes';
comment on column app_commit_status.switched_on_at is
    'purpose: only releases sent after this get a status';
comment on column app_commit_status.switched_off_at is
    'purpose: when the maker switched it off (null: on); after that only the correction of a passed result that no longer holds is posted';
comment on column app_commit_status.permission is
    'purpose: whether GitHub''s last installation token for this repository carried "statuses: write", so the maker is told exactly what to accept on GitHub; null: GitHub hasn''t said';
comment on column app_commit_status.permission_checked_at is
    'purpose: when permission was last read from GitHub';
comment on column app_commit_status.wait_until is
    'purpose: nothing is posted for this app before then (the App isn''t installed, the permission isn''t accepted, or GitHub asked SLAPorSLOP to slow down)';
comment on column app_commit_status.problem_code is
    'purpose: the HTTP status of GitHub''s last refusal for this app (0: no answer after every retry), shown to the maker; cleared by the next accepted status or when the maker checks again';
comment on column app_commit_status.problem_at is
    'purpose: when that refusal happened';

-- One row per commit SLAPorSLOP has posted for or is trying to. It belongs to the app's row above
-- and goes with it, so a commit claimed while that row is being deleted (a new repository address,
-- the account closing) can't stay behind and be followed again later.
create table commit_statuses (
    app_id            uuid not null references apps (id) on delete cascade,
    commit_sha        text not null check (commit_sha ~ '^[0-9a-f]{40}$'),
    posted_status     release_status,
    posted_release_id uuid references releases (id) on delete set null,
    posted_at         timestamptz,
    attempts          integer not null default 0 check (attempts between 0 and 5),
    next_attempt_at   timestamptz,
    claimed_at        timestamptz,
    last_code         integer check (last_code between 0 and 599),
    stopped_at        timestamptz,
    primary key (app_id, commit_sha),
    foreign key (app_id) references app_commit_status (app_id) on delete cascade,
    check ((posted_status is null) = (posted_at is null))
);
comment on table commit_statuses is
    'purpose: what SLAPorSLOP last posted on each commit, to post each state once and retry; the maker''s data; deleted with the app, when its repository address changes, or when the maker''s account closes';
comment on column commit_statuses.commit_sha is
    'purpose: the commit on the maker''s repository the status is posted on';
comment on column commit_statuses.posted_status is
    'purpose: the release state GitHub was last told for this commit; a status is posted only when the release''s state differs from it';
comment on column commit_statuses.posted_release_id is
    'purpose: the release that state belonged to, to show its version to the maker';
comment on column commit_statuses.posted_at is
    'purpose: when GitHub accepted the last status, shown to the maker';
comment on column commit_statuses.attempts is
    'purpose: failed tries since the last accepted status (GitHub errors and no answer); 5 stops';
comment on column commit_statuses.next_attempt_at is
    'purpose: when the next try is due after a failed one';
comment on column commit_statuses.claimed_at is
    'purpose: which API copy is posting this commit right now; a claim older than 5 minutes is free again';
comment on column commit_statuses.last_code is
    'purpose: the HTTP status of GitHub''s last answer for this commit (0: no answer)';
comment on column commit_statuses.stopped_at is
    'purpose: GitHub refused, or every retry failed: nothing more is posted for this commit until the maker checks again, except that correcting a passed result whose approval is gone is tried again after a day';

create index commit_statuses_posted on commit_statuses (app_id, posted_at desc)
    where posted_at is not null;

-- commit_status.rs asks, per app, which release speaks for each commit.
create index releases_app_commit on releases (app_id, commit_sha, created_at desc);

comment on table github_installations is 'purpose: which repositories have SLAPorSLOP''s GitHub App installed, to receive their release webhooks, use their API allowance and post commit statuses there when the maker switched that on';
