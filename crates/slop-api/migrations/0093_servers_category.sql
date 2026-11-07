-- A Servers category (owner request 2026-11-07): dedicated servers people run on their own
-- computer or host so others can play a game in the store. A server is an app like any other (a
-- Windows and/or Linux download, built and checked the same way); being in this category is what
-- makes it a dedicated server, which the Content Policy's rule on code from outside SLAPorSLOP
-- doesn't cover (legal/content.md, "Browser games and dedicated servers"; catalog.rs
-- `IS_DEDICATED_SERVER`). Inserts skip rows that are already there, like 0089.

insert into categories (id, slug, name, description, position) values
    (9, 'servers', 'Servers', 'Dedicated servers you run on your own computer or host, for games in the store', 6)
on conflict do nothing;

-- Right after Utilities; the rest move down one.
update categories set position = case slug
    when 'games' then 1 when 'family' then 2 when 'tools' then 3 when 'creative' then 4
    when 'utilities' then 5 when 'servers' then 6 when 'learning' then 7 when 'play-style' then 8
    when 'vibe' then 9 else position end;

insert into tags (slug, name, category_id)
select v.slug, v.name, c.id
from (values
    ('dedicated-server', 'Dedicated Server', 'servers')
) as v (slug, name, category)
join categories c on c.slug = v.category
on conflict (slug) do nothing;
