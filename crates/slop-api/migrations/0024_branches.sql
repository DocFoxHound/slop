-- Branches and community builds (DESIGN §9), built 2026-10-20.
--
-- A branch is an app of its own (its author, fork, releases, review, signing, votes and donate
-- button) linked to the app it branches from. It isn't listed in the store on its own: it shows on
-- the original's page as a Community build, and in the "Community branches" shelf.

alter table apps
    add column parent_app_id uuid references apps (id),
    -- mod, fix, translation, port, other.
    add column branch_kind text check (branch_kind in ('mod', 'fix', 'translation', 'port', 'other')),
    -- What it changes, in the author's words.
    add column branch_note text check (char_length(branch_note) <= 500),
    add constraint apps_branch_complete check ((parent_app_id is null) = (branch_kind is null)),
    add constraint apps_not_own_branch check (parent_app_id <> id);

create index apps_parent on apps (parent_app_id) where parent_app_id is not null;
