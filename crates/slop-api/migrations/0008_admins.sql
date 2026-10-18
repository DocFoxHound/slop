-- Slop staff who can review releases and moderate apps from the website's admin panel.
-- Granted and removed only from the server (`slop-api admin grant|revoke`).
alter table users add column is_admin boolean not null default false;
