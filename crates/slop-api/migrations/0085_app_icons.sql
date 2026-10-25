-- An icon for every app (media.rs, built 2026-10-25): a square picture the maker names as "icon"
-- in .slop/media.json, beside the thumbnail. Players see it in their library, on store tiles and
-- on the app's page; without an approved one, the website crops the thumbnail, then shows the
-- app's first letter. Like the other pictures it's read at one commit, checked from its header,
-- linked at that commit, and reviewed as part of its set. Not personal data.
--
-- The API's own role gets the new column like every other: slop_grant_app_role (0050) runs after
-- each migration and grants on whole tables.

-- {path, url, sha256, width, height, format, dhash}, or null for no icon.
alter table media_sets add column icon jsonb;
comment on column media_sets.icon is 'purpose: the app''s square icon from the maker''s repository, shown in libraries, on store tiles and on the app''s page once the set is approved';

-- Look-alikes (0052) now say which picture looks like another maker's: thumbnail or icon. Rows
-- from before have no "kind" and are about the thumbnail.
comment on column media_sets.lookalikes is 'purpose: other makers'' apps whose live thumbnail or icon looks like this set''s ({slug, title, distance, kind}), shown to reviewers to catch impersonation';
