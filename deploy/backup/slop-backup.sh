#!/bin/sh
# Backs up SLAPorSLOP's production database and the server's settings and keys (deploy/README.md,
# "Backups"). Run daily by slop-backup.timer.
#
# - The database: pg_dump's compressed custom format, checked by listing it before it counts.
# - Roles (pg_dumpall --globals-only) and ~/.config/slop (env files with the signing keys).
# - Kept: 14 daily and 8 weekly (Sunday) copies, in a folder only this user can read.
# - Off-site: if SLOP_BACKUP_OFFSITE is set, it's run with the new files as arguments (e.g. an
#   rclone command to an encrypted remote). Without it, backups only protect against mistakes,
#   not against losing this machine.
# - Encryption: if SLOP_BACKUP_GPG_RECIPIENT names a GPG public key (whose private key is kept
#   off this machine), the settings-and-keys archive is only ever stored encrypted to it, and
#   the off-site copies of everything are encrypted to it. The local database dumps stay
#   readable for the weekly restore check (security review, 2026-10-22).
# - Writes last-success (a UTC timestamp) for the API's checks (ops.rs).
set -eu

DIR=${SLOP_BACKUP_DIR:-$HOME/slop-backups}
KEEP_DAILY=${SLOP_BACKUP_KEEP_DAILY:-14}
KEEP_WEEKLY=${SLOP_BACKUP_KEEP_WEEKLY:-8}
DB=${SLOP_BACKUP_DB:-slop_prod}
umask 077
mkdir -p "$DIR/daily" "$DIR/weekly"
chmod 700 "$DIR"

stamp=$(date -u +%Y%m%dT%H%M%SZ)
dump="$DIR/daily/$DB-$stamp.dump"
partial="$DIR/.$DB-$stamp.partial"
trap 'rm -f "$partial"' EXIT

podman exec slop-pg pg_dump -U slop --format=custom --compress=6 "$DB" > "$partial"
# A dump pg_restore can't read is no backup.
podman exec -i slop-pg pg_restore --list < "$partial" > /dev/null
mv "$partial" "$dump"
podman exec slop-pg pg_dumpall -U slop --globals-only > "$DIR/daily/globals-$stamp.sql"
RECIPIENT=${SLOP_BACKUP_GPG_RECIPIENT:-}
config="$DIR/daily/config-$stamp.tar.gz"
if [ -n "$RECIPIENT" ]; then
  # The keys never land on disk unencrypted.
  tar -C "$HOME/.config" -czf - slop | gpg --batch --yes --trust-model always \
    --encrypt --recipient "$RECIPIENT" --output "$config.gpg"
  config="$config.gpg"
else
  tar -C "$HOME/.config" -czf "$config" slop
fi

if [ "$(date -u +%u)" = 7 ]; then
  for f in "$dump" "$DIR/daily/globals-$stamp.sql" "$config"; do
    ln -f "$f" "$DIR/weekly/$(basename "$f")"
  done
fi

# Keep the newest N of each kind in each folder.
prune() {
  folder=$1 keep=$2
  for prefix in "$DB-" globals- config-; do
    ls -1t "$folder" | grep "^$prefix" | tail -n +"$((keep + 1))" | while IFS= read -r old; do
      rm -f "$folder/$old"
    done
  done
}
prune "$DIR/daily" "$KEEP_DAILY"
prune "$DIR/weekly" "$KEEP_WEEKLY"

if [ -n "${SLOP_BACKUP_OFFSITE:-}" ]; then
  if [ -n "$RECIPIENT" ]; then
    # Off-site copies are encrypted, every one of them.
    out="$DIR/.offsite-$stamp"
    mkdir -p "$out"
    for f in "$dump" "$DIR/daily/globals-$stamp.sql"; do
      gpg --batch --yes --trust-model always --encrypt --recipient "$RECIPIENT" \
        --output "$out/$(basename "$f").gpg" "$f"
    done
    cp "$config" "$out/"
    # shellcheck disable=SC2086 # the command is meant to be split into words
    $SLOP_BACKUP_OFFSITE "$out"/*
    rm -rf "$out"
  else
    # shellcheck disable=SC2086 # the command is meant to be split into words
    $SLOP_BACKUP_OFFSITE "$dump" "$DIR/daily/globals-$stamp.sql" "$config"
  fi
fi

date -u +%Y-%m-%dT%H:%M:%SZ > "$DIR/last-success"
echo "slop-backup: $dump ($(du -h "$dump" | cut -f1))"
