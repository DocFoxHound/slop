-- Admin → Apps shows, for every app, how many accounts have played it and when one last did, so
-- staff can spot a dead game among thousands (admin_web.rs, apps). user_app_usage is keyed by
-- account first, so finding one app's players meant reading the whole table; this index makes it
-- one lookup per app. No new data.
create index user_app_usage_app on user_app_usage (app_id, last_played_at);
