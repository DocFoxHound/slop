-- Sample store data for local development. Not used in production or tests.
-- Re-running replaces the sample maker and their apps.

begin;

delete from apps where maker_id = '00000000-0000-7000-8000-000000000001';
delete from users where id = '00000000-0000-7000-8000-000000000001';

insert into users (id, display_name, email, country_code, age_16_confirmed_at)
values ('00000000-0000-7000-8000-000000000001', 'Sample Maker', null, 'US', now());

insert into apps (id, slug, title, summary, description_md, kind, stage, stage_entered_at, status,
                  maker_id, repo_url, license_spdx, min_age, last_commit_at)
values
  ('00000000-0000-7000-8000-000000000101', 'sample-dungeon', 'Sample Dungeon',
   'A small roguelike for trying out the store.', 'Go down. Find loot. **Try again.**',
   'game', 'released', now() - interval '60 days', 'listed',
   '00000000-0000-7000-8000-000000000001', 'https://github.com/example/sample-dungeon', 'MIT', 7, now() - interval '3 days'),
  ('00000000-0000-7000-8000-000000000102', 'sample-notes', 'Sample Notes',
   'Plain-text notes that stay on your computer.', 'Notes, offline, in Markdown files.',
   'app', 'beta', now() - interval '20 days', 'listed',
   '00000000-0000-7000-8000-000000000001', 'https://github.com/example/sample-notes', 'GPL-3.0-only', 0, now() - interval '100 days'),
  ('00000000-0000-7000-8000-000000000103', 'sample-garden', 'Sample Garden',
   'A cozy garden game that is coming soon.', 'Plant things. Wait. Relax.',
   'game', 'coming_soon', now() - interval '5 days', 'listed',
   '00000000-0000-7000-8000-000000000001', 'https://github.com/example/sample-garden', 'Apache-2.0', 0, now() - interval '1 day'),
  ('00000000-0000-7000-8000-000000000104', 'sample-night-shift', 'Sample Night Shift',
   'A grim visual novel, rated 18+.', 'Hidden in the under-18 view.',
   'game', 'released', now() - interval '200 days', 'listed',
   '00000000-0000-7000-8000-000000000001', 'https://github.com/example/sample-night-shift', 'MIT', 18, now() - interval '30 days');

insert into app_tags (app_id, tag_id)
select a.id, t.id from (values
  ('sample-dungeon', 'roguelike'), ('sample-dungeon', 'retro'), ('sample-dungeon', 'single-player'),
  ('sample-notes', 'note-taking'), ('sample-notes', 'offline'), ('sample-notes', 'minimalist'),
  ('sample-garden', 'simulation'), ('sample-garden', 'cozy'), ('sample-garden', 'relaxing'),
  ('sample-night-shift', 'visual-novel'), ('sample-night-shift', 'dark')
) as v(app, tag)
join apps a on a.slug = v.app join tags t on t.slug = v.tag;

commit;
