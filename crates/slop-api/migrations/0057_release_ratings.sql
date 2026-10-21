-- Ratings per release (LEGAL-GUIDANCE AGE-7): content can change between versions, so each
-- release carries its own age rating and content labels; the store page shows the rating of
-- the release players get. Raising one is immediate; lowering one waits for a person.
alter table releases
    add column min_age smallint check (min_age in (0, 3, 7, 12, 16, 18)),
    add column content_labels text[];
comment on column releases.min_age is 'purpose: this release''s age rating (AGE-7); the store page takes it when the release is approved';
comment on column releases.content_labels is 'purpose: this release''s content labels (AGE-7)';
-- Releases from before carry the store page's rating as it is now.
update releases r set min_age = a.min_age, content_labels = a.content_labels from apps a where a.id = r.app_id;
