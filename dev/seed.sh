#!/usr/bin/env bash
# Loads sample store data into the local database. Development only, never production.
# Run the API once first so the migrations have created the tables.
set -euo pipefail
engine=$(command -v podman || command -v docker)
"$engine" exec -i slop-pg psql -q -v ON_ERROR_STOP=1 -U slop slop < "$(dirname "$0")/seed.sql"
echo "Sample data loaded."
