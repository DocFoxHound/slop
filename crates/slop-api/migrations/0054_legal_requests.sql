-- Requests from police, courts and authorities (LEGAL-GUIDANCE CHAT-3, CON-11, CON-12): a log of
-- every one, what was asked, the deadline, what Slop gave and whether the person was told. Kept
-- 6 years (§15). The yearly transparency report counts them (CHAT-4).
create table legal_requests (
    id              uuid primary key,
    reference       text not null unique,
    received_at     timestamptz not null,
    -- ISO 3166-1 alpha-2 of the authority.
    country         text not null check (country ~ '^[A-Z]{2}$'),
    authority       text not null check (char_length(authority) between 1 and 200),
    kind            text not null check (kind in ('subpoena', 'court_order', 'warrant', 'eu_production_order',
                                                  'preservation', 'emergency', 'uk_daa', 'removal_order', 'other')),
    legal_basis     text not null default '' check (char_length(legal_basis) <= 500),
    -- What they asked for, in Slop's words.
    asked           text not null check (char_length(asked) between 1 and 4000),
    -- The account it's about, when there is one.
    subject_user_id uuid references users (id) on delete set null,
    deadline        timestamptz,
    status          text not null default 'open'
                    check (status in ('open', 'answered', 'partly_answered', 'refused', 'withdrawn')),
    -- What Slop gave, or why not.
    response        text check (char_length(response) <= 4000),
    -- notified, barred (a law or order forbids telling them), not_needed (no account involved), pending
    user_notice     text not null default 'pending' check (user_notice in ('pending', 'notified', 'barred', 'not_needed')),
    created_by      text not null,
    answered_by     text,
    answered_at     timestamptz,
    created_at      timestamptz not null default now()
);
create index legal_requests_open on legal_requests (deadline) where status = 'open';
comment on table legal_requests is 'purpose: the law-enforcement request log (CHAT-3): what authorities asked, by when, what Slop gave; kept 6 years; counted in the transparency report';
comment on column legal_requests.subject_user_id is 'purpose: the account a request concerns, to answer it and to tell the person unless a law or the order forbids it';
