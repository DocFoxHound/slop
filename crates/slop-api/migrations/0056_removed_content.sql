-- Terrorist content removal orders (Regulation (EU) 2021/784, LEGAL-GUIDANCE CON-11): content
-- taken down in one step is kept, sealed, for six months (Art. 6) for a review or an appeal,
-- then deleted (retention.rs).
create table removed_content (
    id                  uuid primary key,
    legal_request_id    uuid not null references legal_requests (id),
    decision_id         uuid not null references moderation_decisions (id),
    subject_kind        text not null,
    subject_id          uuid not null,
    -- The content as it was when removed.
    snapshot            jsonb not null,
    removed_at          timestamptz not null default now(),
    preserve_until      timestamptz not null
);
comment on table removed_content is 'purpose: a sealed copy of content removed on a terrorist content removal order, kept six months for review and appeals (Regulation (EU) 2021/784 Art. 6); staff with sensitive rights only';
