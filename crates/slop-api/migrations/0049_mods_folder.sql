-- Mods in the folder a game already reads (DESIGN §9): a maker can name it, and the Slop app
-- keeps only mods approved on Slop there. Null: the game reads SLOP_MODS instead.
alter table apps
    add column mods_folder jsonb;
comment on column apps.mods_folder is 'purpose: where the app reads mods from ({base: game|saves, path, layout: folder|flat}), signed into its release manifests so the Slop app fills it with approved mods';
