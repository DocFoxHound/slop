-- Ratings open in Beta as well as at release (owner decision 2026-10-25; DESIGN §7). Each vote
-- remembers the stage the app was in when it was cast, so pages can say "Rated in Beta". Every
-- vote before this was cast on a released app.
alter table vote_events add column stage app_stage not null default 'released';
alter table votes add column stage app_stage not null default 'released';
