-- Payment records and vote history can't be changed (SECURITY §9, METRICS §4), and VAT numbers
-- are encrypted at rest (SECURITY §8.1, §10). Built 2026-10-20.

-- Corrections are new rows (a refund, a new vote), never edits. Two exceptions:
-- - closing an account replaces its ID on vote_events with a random one (account.rs), changing
--   nothing else;
-- - the simulation sandbox wipes its own data, and only in a database named *_sim.
create function slop_append_only() returns trigger language plpgsql as $$
begin
    if current_setting('slop.purge', true) = 'on' and current_database() like '%\_sim' then
        return case when tg_op = 'DELETE' then old else new end;
    end if;
    if tg_op = 'UPDATE' and tg_table_name = 'vote_events'
       and (to_jsonb(new) - 'user_id') = (to_jsonb(old) - 'user_id') then
        return new;
    end if;
    raise exception '% is append-only: record a correction as a new row', tg_table_name;
end
$$;

create trigger payments_append_only before update or delete on payments
    for each row execute function slop_append_only();
create trigger vote_events_append_only before update or delete on vote_events
    for each row execute function slop_append_only();

-- VAT numbers are stored sealed ("enc1:…", fields.rs) once SLOP_FIELD_KEY is set; the format
-- is checked before sealing.
alter table users drop constraint if exists users_vat_number_check;
