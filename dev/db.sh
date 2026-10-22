#!/usr/bin/env bash
# Starts (or creates) the local Postgres used for development and tests.
# Data lives in the podman volume `slop-pg-dev-data`. Remove with:
#   podman rm -f slop-pg-dev && podman volume rm slop-pg-dev-data
# On the deployment server this is its own container (slop-pg-dev on 5434), separate from the
# production cluster (slop-pg on 5433), so tests and dev tools never hold a login to production.
set -euo pipefail
engine=$(command -v podman || command -v docker)
if "$engine" container exists slop-pg-dev 2>/dev/null || "$engine" inspect slop-pg-dev >/dev/null 2>&1; then
  "$engine" start slop-pg-dev >/dev/null
else
  "$engine" run -d --name slop-pg-dev \
    -e POSTGRES_USER=slop -e POSTGRES_PASSWORD=slop -e POSTGRES_DB=slop \
    -p 127.0.0.1:5434:5432 -v slop-pg-dev-data:/var/lib/postgresql/data \
    docker.io/library/postgres:17 >/dev/null
fi
until "$engine" exec slop-pg-dev pg_isready -U slop -q; do sleep 0.5; done
echo "Postgres ready: postgres://slop:slop@127.0.0.1:5434/slop"
