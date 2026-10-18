#!/usr/bin/env bash
# Starts (or creates) the local Postgres used for development and tests.
# Data lives in the podman volume `slop-pg-data`. Remove with: podman rm -f slop-pg && podman volume rm slop-pg-data
set -euo pipefail
engine=$(command -v podman || command -v docker)
if "$engine" container exists slop-pg 2>/dev/null || "$engine" inspect slop-pg >/dev/null 2>&1; then
  "$engine" start slop-pg >/dev/null
else
  "$engine" run -d --name slop-pg \
    -e POSTGRES_USER=slop -e POSTGRES_PASSWORD=slop -e POSTGRES_DB=slop \
    -p 127.0.0.1:5433:5432 -v slop-pg-data:/var/lib/postgresql/data \
    docker.io/library/postgres:17 >/dev/null
fi
until "$engine" exec slop-pg pg_isready -U slop -q; do sleep 0.5; done
echo "Postgres ready: postgres://slop:slop@127.0.0.1:5433/slop"
