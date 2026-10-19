-- People, part 2 (decided 2026-10-19, DESIGN §15.2–15.7): exact-wording acknowledgements,
-- inquiries, recorded job conversations the creative opens, changes to the agreement as new
-- documents, an optional sign-off on the direction, a preview before the final payment with the
-- final files after it, problems with mediation, and disputes about reviews.

alter table jobs drop constraint jobs_status_check;
alter table jobs add constraint jobs_status_check check (status in (
    'requested', 'offered', 'awaiting_payment', 'in_progress', 'awaiting_sign_off', 'delivered',
    'final_files', 'completed', 'declined', 'cancelled', 'withdrawn'));

alter table jobs
    -- "Ask about this first": the client asked for a conversation before any offer.
    add column inquiry boolean not null default false,
    -- The client's acknowledgement that the creative delivers exactly what's written (on the
    -- request, and again on accepting).
    add column request_ack_at timestamptz,
    add column exact_terms_ack_at timestamptz,
    -- The current due date (the offer's, moved by accepted changes).
    add column due_date date,
    add column signed_off_at timestamptz,
    add column final_files_at timestamptz,
    -- Conversations: the client asks, the creative opens and closes them.
    add column conversation_open boolean not null default false,
    add column conversation_requested_at timestamptz,
    add column conversation_request_note text check (char_length(conversation_request_note) <= 300),
    add column conversation_closed_at timestamptz,
    -- Each side's acknowledgement that the conversation is recorded.
    add column client_recording_ack_at timestamptz,
    add column creative_recording_ack_at timestamptz;

comment on column jobs.conversation_request_note is 'purpose: why the client asked for a conversation; part of the job record';

-- Recorded job conversations (decided 2026-10-19, DESIGN §15.5). Both sides keep the
-- conversation: Slop holds the text only while it's open and for 30 days after it closes (to
-- download), then deletes it and keeps each message's fingerprint, a SHA-256 chained to the one
-- before, which proves a downloaded copy is unchanged.
create table job_messages (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    author_id           uuid not null references users (id),
    -- Null once deleted (cleanup.rs).
    body                text check (char_length(body) between 1 and 4000),
    sha256              text not null,
    created_at          timestamptz not null default now()
);

create index job_messages_job on job_messages (job_id, created_at);

comment on table job_messages is 'Recorded job conversations (DESIGN §15.5): kept with the job record for contract and legal purposes';
comment on column job_messages.body is 'purpose: relaying the job conversation and letting both sides download it; deleted 30 days after the conversation closes';
comment on column job_messages.sha256 is 'purpose: proving a downloaded copy of the conversation is unchanged (not the content)';

-- Changes to the agreement: each is its own document, with its own price and date (new work).
create table job_changes (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    number              integer not null,
    -- The client's request, if the client asked.
    requested_note      text check (char_length(requested_note) <= 2000),
    status              text not null check (status in ('requested', 'offered', 'accepted', 'declined', 'withdrawn')),
    title               text check (char_length(title) <= 100),
    scope_md            text check (char_length(scope_md) <= 4000),
    extra_price_minor   integer check (extra_price_minor >= 0),
    new_due_date        date,
    decline_note        text check (char_length(decline_note) <= 1000),
    text                text,
    sha256              text,
    created_at          timestamptz not null default now(),
    offered_at          timestamptz,
    decided_at          timestamptz,
    unique (job_id, number)
);

create unique index job_changes_one_open on job_changes (job_id) where status in ('requested', 'offered');

comment on column job_changes.requested_note is 'purpose: the client''s change request, part of the contract record';
comment on column job_changes.text is 'purpose: the accepted Change to Agreement (HUM-16)';

-- A change's extra price is its own payment.
alter table job_payments drop constraint job_payments_job_id_part_key;
alter table job_payments drop constraint job_payments_part_check;
alter table job_payments add constraint job_payments_part_check check (part in ('full', 'deposit', 'final', 'change'));
alter table job_payments add column change_id uuid references job_changes (id);
create unique index job_payments_one_part on job_payments (job_id, part) where part <> 'change';

-- Problems with a job: each side states its case and proposes outcomes; both must agree. A
-- mediator may suggest an outcome, which binds no one (LEGAL §4.1).
create table job_problems (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    opened_by           uuid not null references users (id),
    statement           text not null check (char_length(statement) between 1 and 4000),
    reply               text check (char_length(reply) <= 4000),
    replied_at          timestamptz,
    status              text not null default 'open' check (status in ('open', 'resolved', 'unresolved', 'withdrawn')),
    mediator_suggestion text check (char_length(mediator_suggestion) <= 4000),
    suggested_by        text,
    suggested_at        timestamptz,
    closing_note        text check (char_length(closing_note) <= 2000),
    created_at          timestamptz not null default now(),
    closed_at           timestamptz
);

create unique index job_problems_one_open on job_problems (job_id) where status = 'open';

comment on column job_problems.statement is 'purpose: the opener''s account of a problem with a job, for the other side and a mediator';
comment on column job_problems.reply is 'purpose: the other side''s account';

-- A side's own copy of the conversation, submitted as evidence for a problem or a review dispute,
-- checked against the fingerprints.
create table job_evidence (
    id                  uuid primary key,
    job_id              uuid not null references jobs (id) on delete cascade,
    submitted_by        uuid not null references users (id),
    transcript          text not null check (char_length(transcript) <= 400000),
    -- Messages in it that matched Slop's fingerprints, and ones that didn't.
    verified_messages   integer not null,
    unverified_messages integer not null,
    created_at          timestamptz not null default now()
);

comment on column job_evidence.transcript is 'purpose: a party''s own copy of the job conversation, submitted by them for a problem or review dispute';

create table job_problem_proposals (
    id                  uuid primary key,
    problem_id          uuid not null references job_problems (id) on delete cascade,
    proposed_by         uuid not null references users (id),
    -- refund (amount), cancel (end the job, keeping what's paid), redo (the creative redoes the
    -- named part at no charge), other.
    kind                text not null check (kind in ('refund', 'cancel', 'redo', 'other')),
    amount_minor        integer check (amount_minor > 0),
    note                text not null check (char_length(note) between 1 and 1000),
    status              text not null default 'pending' check (status in ('pending', 'accepted', 'rejected', 'withdrawn')),
    created_at          timestamptz not null default now(),
    decided_at          timestamptz
);

create index job_problem_proposals_problem on job_problem_proposals (problem_id, created_at);

-- Reviews can be hidden for breaking the rules, or carry a mediator's note (never hidden for
-- being negative: HUM-12, HUM-18).
alter table creative_reviews
    add column hidden_at timestamptz,
    add column hidden_reason text,
    add column mediator_note text check (char_length(mediator_note) <= 1000);

create table review_disputes (
    job_id              uuid primary key references creative_reviews (job_id) on delete cascade,
    -- rules (insults, private information), not_about_job, extortion, false_fact.
    reason              text not null check (reason in ('rules', 'not_about_job', 'extortion', 'false_fact')),
    statement           text not null check (char_length(statement) between 1 and 2000),
    client_statement    text check (char_length(client_statement) <= 2000),
    status              text not null default 'pending' check (status in ('pending', 'removed', 'noted', 'kept')),
    decision_reason     text,
    decided_by          text,
    created_at          timestamptz not null default now(),
    decided_at          timestamptz
);

comment on column review_disputes.statement is 'purpose: the creative''s case that a review breaks the rules (LEGAL §4.5)';
comment on column review_disputes.client_statement is 'purpose: the reviewer''s answer to the dispute';

alter table review_notes drop constraint review_notes_subject_kind_check;
alter table review_notes add constraint review_notes_subject_kind_check
    check (subject_kind in ('release', 'change', 'media', 'listing', 'portfolio', 'problem', 'review_dispute'));
