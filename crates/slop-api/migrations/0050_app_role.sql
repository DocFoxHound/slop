-- The API can run as a role that doesn't own the tables (SECURITY §9, deploy/README.md "Database
-- roles"): migrations run as the owner (SLOP_MIGRATE_DATABASE_URL), the API as its own role
-- (DATABASE_URL). That role reads and writes as usual, but only adds rows to the ledgers, and
-- can't switch off their append-only triggers, which only an owner can.
create or replace function slop_grant_app_role(app_role name) returns void language plpgsql as $$
begin
    execute format('grant usage on schema public to %I', app_role);
    execute format('grant select, insert, update, delete on all tables in schema public to %I', app_role);
    execute format('grant usage, select, update on all sequences in schema public to %I', app_role);
    execute format('grant execute on all functions in schema public to %I', app_role);
    -- Ledgers: payments, vote history and the staff audit log only grow.
    execute format('revoke update, delete, truncate on payments, vote_events, audit_log from %I', app_role);
    -- Closing an account replaces its ID in the vote history (account.rs), and nothing else.
    execute format('grant update (user_id) on vote_events to %I', app_role);
    -- Tables and sequences later migrations make.
    execute format('alter default privileges in schema public grant select, insert, update, delete on tables to %I', app_role);
    execute format('alter default privileges in schema public grant usage, select, update on sequences to %I', app_role);
end
$$;
comment on function slop_grant_app_role(name) is 'purpose: gives the API''s own database role what it needs, with the ledgers insert-only; run after every migration by the API when SLOP_MIGRATE_DATABASE_URL is set';
