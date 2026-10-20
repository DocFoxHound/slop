-- Reports, moderation decisions with their statements of reasons and appeals, closing and
-- suspending accounts, and versioned legal documents with each user's acceptances (decided
-- 2026-10-20: DESIGN §4 and §16; LEGAL-GUIDANCE CON-3, CON-5, CON-8, CON-9, PRIV-3, §15).

-- Accounts --------------------------------------------------------------------------------------

alter table users
    -- The user closed their account: its sign-in is refused, its personal data is removed, and
    -- pages others rely on (a creative's page, job records, reviews) are frozen as they were.
    add column closed_at timestamptz,
    -- A moderation decision suspended the account. The user can still sign in to read the
    -- decision, appeal it, download their data or close the account; nothing else.
    add column suspended_at timestamptz;

comment on column users.closed_at is 'purpose: the account was closed by its owner; the sign-in stays refused (DESIGN §4)';
comment on column users.suspended_at is 'purpose: the account is suspended by a moderation decision (CON-9)';

-- Legal documents and acceptances ----------------------------------------------------------------

-- Each version of each document, as published (legal.rs loads them from crates/slop-api/legal/).
create table legal_documents (
    kind                text not null check (kind in ('terms', 'privacy', 'content', 'publisher', 'people')),
    version             text not null check (version ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}[a-z]?$'),
    title               text not null,
    -- What changed from the version before, in plain words.
    summary             text not null,
    body_md             text not null,
    sha256              text not null,
    published_at        timestamptz not null default now(),
    -- Changes take effect 15 days after they're published (P2B Regulation Art. 3; TERMS).
    effective_at        timestamptz not null,
    primary key (kind, version)
);

-- No cascade: acceptances are kept for the life of the account plus six years (LEGAL-GUIDANCE §15).
create table legal_acceptances (
    user_id             uuid not null references users (id),
    kind                text not null,
    version             text not null,
    accepted_at         timestamptz not null default now(),
    primary key (user_id, kind, version),
    foreign key (kind, version) references legal_documents (kind, version)
);

comment on table legal_acceptances is 'purpose: proof of which version of each agreement the user accepted, and when (LEGAL-GUIDANCE §15)';

-- Moderation decisions ----------------------------------------------------------------------------

-- Every restriction Slop places on someone's content or account, with the statement of reasons
-- they're shown (DSA Art. 17, CON-9) and their appeal (DSA Art. 20, CON-16).
create table moderation_decisions (
    id                  uuid primary key,
    reference           text not null unique,
    -- Whose content or account it is.
    user_id             uuid not null references users (id),
    subject_kind        text not null check (subject_kind in ('app', 'review', 'profile', 'message', 'account')),
    subject_id          uuid not null,
    -- What it was called at the time (titles and names change).
    subject_label       text not null,
    action              text not null check (action in ('delist_app', 'hide_review', 'hide_profile', 'suspend_account', 'warn')),
    -- The rule broken: a section of the Content Policy (moderation.rs RULES).
    rule                text not null,
    -- The law, when it's the law rather than only Slop's rules.
    law                 text check (char_length(law) <= 300),
    facts               text not null check (char_length(facts) between 1 and 4000),
    -- Whether automated tools made or helped make the decision. Today every decision is a person's.
    automated           boolean not null default false,
    decided_by          text not null,
    created_at          timestamptz not null default now(),
    -- When the affected user first opened it.
    seen_at             timestamptz,
    appeal              text check (char_length(appeal) between 1 and 4000),
    -- A copyright counter-notice (17 U.S.C. §512(g)(3)): name, address, phone and the statements.
    counter_notice      jsonb,
    appealed_at         timestamptz,
    appeal_status       text check (appeal_status in ('pending', 'upheld', 'reversed')),
    appeal_reason       text check (char_length(appeal_reason) <= 2000),
    appeal_decided_by   text,
    appeal_decided_at   timestamptz,
    reversed_at         timestamptz
);

create index moderation_decisions_user on moderation_decisions (user_id, created_at desc);
create index moderation_decisions_appeals on moderation_decisions (appealed_at) where appeal_status = 'pending';

comment on column moderation_decisions.facts is 'purpose: the statement of reasons shown to the affected user (DSA Art. 17); kept 6 years';
comment on column moderation_decisions.appeal is 'purpose: the affected user''s appeal (DSA Art. 20); kept 6 years';
comment on column moderation_decisions.counter_notice is 'purpose: a copyright counter-notice: the name, address and phone 17 U.S.C. §512(g)(3) requires, passed to the complainant';

-- Reports -----------------------------------------------------------------------------------------

-- Notices from anyone about an app, a review, a People profile or a job message (DSA Art. 16,
-- CON-8). Kept 6 years with the decision, its reason and the time taken (LEGAL-GUIDANCE §15).
create table reports (
    id                  uuid primary key,
    -- Shown to the reporter as the acknowledgement.
    reference           text not null unique,
    subject_kind        text not null check (subject_kind in ('app', 'review', 'profile', 'message')),
    subject_id          uuid not null,
    -- The account the report is about (the maker, reviewer, creative or author).
    subject_user_id     uuid references users (id),
    subject_label       text not null,
    -- A reported message as it was: job conversations are deleted 30 days after they close.
    quoted              text check (char_length(quoted) <= 4000),
    reason              text not null,
    details             text not null check (char_length(details) between 1 and 4000),
    reporter_id         uuid references users (id),
    -- Signed-out reporters give a name and email (optional for child abuse reports).
    reporter_name       text check (char_length(reporter_name) <= 100),
    reporter_email      text check (char_length(reporter_email) <= 254),
    -- Copyright notices (17 U.S.C. §512(c)(3)): the work, the owner, contact details, signature.
    copyright           jsonb,
    -- 0 urgent (child abuse, threats, intimate images), 1 serious, 2 everything else.
    priority            smallint not null check (priority between 0 and 2),
    due_at              timestamptz not null,
    status              text not null default 'open' check (status in ('open', 'actioned', 'dismissed')),
    -- What the reporter is told.
    outcome             text check (char_length(outcome) <= 2000),
    decided_by          text,
    decided_at          timestamptz,
    decision_id         uuid references moderation_decisions (id),
    -- Child abuse reports are preserved for a year (18 U.S.C. §2258A(h)), never deleted with the content.
    preserve_until      timestamptz,
    -- Where a report was passed on (NCMEC, the NCA, the police) and its reference.
    external_reference  text check (char_length(external_reference) <= 200),
    created_at          timestamptz not null default now()
);

create index reports_open on reports (priority, due_at) where status = 'open';
create index reports_subject on reports (subject_kind, subject_id);
create index reports_reporter on reports (reporter_id, created_at desc);
create unique index reports_one_open_each on reports (reporter_id, subject_kind, subject_id)
    where status = 'open' and reporter_id is not null;

comment on column reports.details is 'purpose: the reporter''s explanation, for moderators (DSA Art. 16); kept 6 years';
comment on column reports.quoted is 'purpose: the reported job message as it was, for moderators; kept 6 years';
comment on column reports.reporter_name is 'purpose: identifying a signed-out reporter (DSA Art. 16(2)(c)); kept 6 years';
comment on column reports.reporter_email is 'purpose: contacting a signed-out reporter about their report (DSA Art. 16(4)-(5)); kept 6 years';
comment on column reports.copyright is 'purpose: the details a copyright notice must contain (17 U.S.C. §512(c)(3)); passed to the user it''s about';
