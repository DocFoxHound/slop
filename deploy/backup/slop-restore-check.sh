#!/bin/sh
# Proves the newest backup restores (ARCHITECTURE §7: restore tests). Run weekly by
# slop-restore-check.timer: restores it into a scratch database, compares a few tables with the
# live database, then drops the scratch copy. Writes last-restore-check for ops.rs.
set -eu

DIR=${SLOP_BACKUP_DIR:-$HOME/slop-backups}
DB=${SLOP_BACKUP_DB:-slop_prod}
SCRATCH=slop_restore_check

latest=$(ls -1t "$DIR"/daily/"$DB"-*.dump 2>/dev/null | head -n 1)
[ -n "$latest" ] || { echo "slop-restore-check: no backup to restore" >&2; exit 1; }

psql() { podman exec -i slop-pg psql -U slop -v ON_ERROR_STOP=1 -qAt "$@"; }
cleanup() { podman exec slop-pg dropdb -U slop --if-exists "$SCRATCH" >/dev/null 2>&1 || true; }
trap cleanup EXIT

cleanup
podman exec slop-pg createdb -U slop "$SCRATCH"
podman exec -i slop-pg pg_restore -U slop --no-owner --no-acl --exit-on-error -d "$SCRATCH" < "$latest"

for table in users apps releases audit_log; do
  live=$(psql -d "$DB" -c "select count(*) from $table")
  restored=$(psql -d "$SCRATCH" -c "select count(*) from $table")
  # The backup is older than the live database, so it can have fewer rows, never more, and none
  # only if the live table is empty too.
  if [ "$restored" -gt "$live" ] || { [ "$live" -gt 0 ] && [ "$restored" -eq 0 ]; }; then
    echo "slop-restore-check: $table has $restored rows in the backup and $live live" >&2
    exit 1
  fi
done

date -u +%Y-%m-%dT%H:%M:%SZ > "$DIR/last-restore-check"
echo "slop-restore-check: $(basename "$latest") restores"
