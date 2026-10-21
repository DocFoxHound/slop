#!/usr/bin/env bash
# Gives the API its own database role (deploy/README.md, "Database roles"): it reads and writes
# as usual, but can only add to payments, vote history and the audit log, and can't switch off
# their append-only triggers. Run once per database, as its owner:
#   deploy/db-roles.sh postgres://owner:password@host:port/dbname
# It prints the two lines to put in slop.env. The password is new and shown once.
set -euo pipefail
owner_url="${1:?usage: db-roles.sh <owner database URL>}"
role="${SLOP_APP_ROLE:-slop_app}"
password=$(openssl rand -hex 24)
psql "$owner_url" -v ON_ERROR_STOP=1 -q <<SQL
do \$\$ begin
  if exists (select 1 from pg_roles where rolname = '$role') then
    execute format('alter role %I login password %L', '$role', '$password');
  else
    execute format('create role %I login password %L', '$role', '$password');
  end if;
end \$\$;
grant connect on database $(basename "${owner_url%%\?*}") to $role;
select slop_grant_app_role('$role');
SQL
app_url=$(python3 - "$owner_url" "$role" "$password" <<'PY'
import sys, urllib.parse
u = urllib.parse.urlsplit(sys.argv[1])
host = u.hostname + (f":{u.port}" if u.port else "")
print(urllib.parse.urlunsplit((u.scheme, f"{sys.argv[2]}:{sys.argv[3]}@{host}", u.path, u.query, "")))
PY
)
echo "Put these in slop.env, then restart the API:"
echo "SLOP_MIGRATE_DATABASE_URL=$owner_url"
echo "DATABASE_URL=$app_url"
