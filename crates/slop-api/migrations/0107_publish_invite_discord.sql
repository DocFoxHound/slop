-- Where makers ask for the beta invite code (owner request 2026-11-19): the SLAPorSLOP Discord's
-- invite link, set by staff next to the code. Empty until staff set it; the closed-beta notice
-- then names the Discord without a link.
alter table publish_invite add column discord_url text not null default '';
