-- Playtime is credited at most at the speed of the clock per player and app, however many play
-- sessions run at once (security review, 2026-10-22: parallel sessions each added playtime, so a
-- script could reach full vote weight in minutes).
alter table user_app_usage add column credited_until timestamptz;
comment on column user_app_usage.credited_until is 'purpose: the time up to which playtime has been credited, so overlapping sessions add no more than real time';
