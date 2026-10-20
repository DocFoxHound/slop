-- Abuse checks (abuse.rs, SECURITY §11, METRICS §5), built 2026-10-20: patterns a person should
-- look at. Nothing changes automatically.
create table abuse_signals (
    id                  uuid primary key,
    -- vote_burst, review_bomb, vote_swing, lookalike_name, rate_limited
    kind                text not null,
    -- Stable for the same finding (e.g. kind + app + day), so it's raised once.
    key                 text not null unique,
    subject_kind        text not null check (subject_kind in ('app', 'account')),
    subject_id          uuid not null,
    subject_label       text not null,
    detail              jsonb not null,
    status              text not null default 'open' check (status in ('open', 'dismissed', 'actioned')),
    decided_by          text,
    decision_note       text check (char_length(decision_note) <= 1000),
    detected_at         timestamptz not null default now(),
    decided_at          timestamptz
);

create index abuse_signals_open on abuse_signals (detected_at) where status = 'open';

comment on table abuse_signals is 'purpose: spotting manipulated ratings, impersonation and spam for a person to review (legitimate interest: platform integrity)';
