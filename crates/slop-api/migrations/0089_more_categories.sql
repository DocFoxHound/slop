-- More categories and tags (owner request 2026-10-26): a Kids & Family category, and the genres,
-- tools and moods a store needs (think Steam and itch.io genres, not hundreds of tags). Inserts
-- skip rows that are already there, so it can't clash with a tag staff added by hand.

-- A category can hold apps to an age: under it and its tags, the store lists only apps rated at
-- most `max_age` and without any of `excluded_labels` (catalog.rs). Null: no limit.
alter table categories
    add column max_age smallint check (max_age in (0, 3, 7, 12, 16, 18)),
    add column excluded_labels text[] not null default '{}'
        check (excluded_labels <@ array['romance', 'suggestive', 'violence', 'horror', 'strong_language', 'gambling_themes']::text[]);

-- Kids & Family: up to 7+ (PEGI 7), and none of the themes that aren't for children at any
-- rating. Mild cartoon violence and a love story are allowed: the 7+ cap already limits them.
insert into categories (id, slug, name, description, position, max_age, excluded_labels) values
    (8, 'family', 'Kids & Family', 'Simple, gentle fun for younger players and the whole family', 2, 7,
     array['suggestive', 'horror', 'strong_language', 'gambling_themes'])
on conflict do nothing;

-- Right after Games; the rest move down one.
update categories set position = case slug
    when 'games' then 1 when 'family' then 2 when 'tools' then 3 when 'creative' then 4
    when 'utilities' then 5 when 'learning' then 6 when 'play-style' then 7 when 'vibe' then 8
    else position end;

-- Kids moves from Learning to Kids & Family, keeping its ID and slug so apps keep it.
update tags set category_id = (select id from categories where slug = 'family') where slug = 'kids';

insert into tags (slug, name, category_id)
select v.slug, v.name, c.id
from (values
    ('family', 'Family', 'family'), ('all-ages', 'All Ages', 'family'),
    ('educational', 'Educational', 'family'),

    ('action', 'Action', 'games'), ('adventure', 'Adventure', 'games'), ('arcade', 'Arcade', 'games'),
    ('shooter', 'Shooter', 'games'), ('fighting', 'Fighting', 'games'), ('rpg', 'RPG', 'games'),
    ('sports', 'Sports', 'games'), ('horror', 'Horror', 'games'), ('survival', 'Survival', 'games'),
    ('sandbox', 'Sandbox', 'games'), ('open-world', 'Open World', 'games'),
    ('metroidvania', 'Metroidvania', 'games'), ('rhythm', 'Rhythm', 'games'),
    ('tower-defense', 'Tower Defense', 'games'), ('city-builder', 'City Builder', 'games'),
    ('deckbuilder', 'Deckbuilder', 'games'), ('board-game', 'Board Game', 'games'),
    ('word-game', 'Word Game', 'games'), ('trivia', 'Trivia', 'games'),
    ('point-and-click', 'Point & Click', 'games'), ('stealth', 'Stealth', 'games'),
    ('bullet-hell', 'Bullet Hell', 'games'), ('endless-runner', 'Endless Runner', 'games'),
    ('farming', 'Farming', 'games'), ('exploration', 'Exploration', 'games'),
    ('physics', 'Physics', 'games'), ('party', 'Party', 'games'),

    ('communication', 'Communication', 'tools'), ('office', 'Office', 'tools'),
    ('calendar', 'Calendar', 'tools'), ('health-fitness', 'Health & Fitness', 'tools'),
    ('ai-assistant', 'AI Assistant', 'tools'),

    ('drawing', 'Drawing', 'creative'), ('photo-editing', 'Photo Editing', 'creative'),
    ('animation', 'Animation', 'creative'), ('audio-editing', 'Audio Editing', 'creative'),
    ('design', 'Design', 'creative'), ('game-making', 'Game Making', 'creative'),

    ('security', 'Security', 'utilities'), ('networking', 'Networking', 'utilities'),
    ('accessibility', 'Accessibility Tools', 'utilities'), ('customization', 'Customization', 'utilities'),
    ('hardware', 'Hardware', 'utilities'), ('media-player', 'Media Player', 'utilities'),
    ('screenshots', 'Screenshots', 'utilities'),

    ('science', 'Science', 'learning'), ('coding', 'Coding', 'learning'),
    ('history', 'History', 'learning'), ('geography', 'Geography', 'learning'),
    ('reading', 'Reading', 'learning'),

    ('split-screen', 'Split-screen', 'play-style'), ('pvp', 'PvP', 'play-style'),
    ('turn-based', 'Turn-based', 'play-style'), ('real-time', 'Real-time', 'play-style'),
    ('keyboard-only', 'Keyboard Only', 'play-style'),

    ('wholesome', 'Wholesome', 'vibe'), ('spooky', 'Spooky', 'vibe'), ('cute', 'Cute', 'vibe'),
    ('weird', 'Weird', 'vibe'), ('atmospheric', 'Atmospheric', 'vibe'), ('intense', 'Intense', 'vibe')
) as v (slug, name, category)
join categories c on c.slug = v.category
on conflict (slug) do nothing;
