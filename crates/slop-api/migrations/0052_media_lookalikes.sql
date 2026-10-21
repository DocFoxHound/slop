-- Store-page icons that look like another maker's (media.rs `lookalikes`): a new thumbnail is
-- compared with every other maker's live one by perceptual hash, and close ones are shown to
-- the reviewer. A note for a person, never an automatic rejection.
alter table media_sets add column lookalikes jsonb not null default '[]';
comment on column media_sets.lookalikes is 'purpose: other makers'' apps whose live icon looks like this set''s thumbnail ({slug, title, distance}), shown to reviewers to catch impersonation';
