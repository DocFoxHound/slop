#!/usr/bin/env bash
# Deploys the newest commit on main, if it passes the tests (deploy/README.md).
#
# Run every two minutes by slop-deploy.timer. Builds in its own clone, never in a working copy.
# On success it switches the `current` symlink and restarts slop-api.service; if the new build
# doesn't come up healthy, it switches back.
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
REPO="$ROOT/repo"
BRANCH="${SLOP_DEPLOY_BRANCH:-main}"
REMOTE="${SLOP_DEPLOY_REMOTE:-git@github.com:DocFoxHound/slop.git}"
# The test suite creates throwaway databases on this server (the dev superuser).
TEST_DATABASE_URL="${SLOP_TEST_DATABASE_URL:-postgres://slop:slop@127.0.0.1:5433/slop}"
export PATH="$HOME/.cargo/bin:$PATH"

log() { echo "slop-deploy: $*"; }
mkdir -p "$ROOT/builds"
exec 9>"$ROOT/.lock"
flock -n 9 || { log "another deploy is running"; exit 0; }

[ -d "$REPO/.git" ] || git clone -q "$REMOTE" "$REPO"
git -C "$REPO" fetch -q origin "$BRANCH"
target=$(git -C "$REPO" rev-parse "origin/$BRANCH")
current=$(basename "$(readlink "$ROOT/current" 2>/dev/null || echo none)")
[ "$target" = "$current" ] && exit 0
if [ -f "$ROOT/builds/failed-$target" ]; then exit 0; fi

log "deploying ${target:0:12} (running ${current:0:12})"
git -C "$REPO" checkout -q --force --detach "$target"
git -C "$REPO" clean -qfdx -e target -e web/node_modules
cd "$REPO"

fail() { log "FAILED at $1; staying on ${current:0:12}"; touch "$ROOT/builds/failed-$target"; exit 1; }

# The full output is kept in test.log; the journal gets which tests failed and why.
if ! DATABASE_URL="$TEST_DATABASE_URL" cargo test -q >"$ROOT/test.log" 2>&1; then
  grep -E "panicked at|FAILED|^failures:|^    [a-z_]+$" "$ROOT/test.log" | tail -n 30
  fail "tests"
fi
cargo build -q --release -p slop-api || fail "build"
(cd web && npm ci --silent --no-audit --no-fund && npm test --silent && npm run build --silent) || fail "web build"

build="$ROOT/builds/$target"
rm -rf "$build" && mkdir -p "$build"
cp target/release/slop-api "$build/"
cp -r web/dist "$build/web"
cp deploy/update.sh "$build/update.sh"
cp -r deploy/desktop-linux "$build/desktop-linux"

previous=$(readlink "$ROOT/current" 2>/dev/null || true)
ln -sfn "$build" "$ROOT/current.new" && mv -T "$ROOT/current.new" "$ROOT/current"
systemctl --user restart slop-api.service

for _ in $(seq 30); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" = 204 ]; then
    log "live: ${target:0:12}"
    # The simulation sandbox runs the same build, when it's set up (docs/SIMULATION.md).
    systemctl --user try-restart slop-sim.service slop-sim-pulse.service || log "couldn't restart the sandbox"
    # The deploy script updates itself from the deployed commit.
    install -m 755 "$build/update.sh" "$ROOT/bin/update.sh.new" && mv "$ROOT/bin/update.sh.new" "$ROOT/bin/update.sh"
    # Linux desktop packages: rebuild (in the background) when the app changed.
    last_desktop=$(cat "$ROOT/desktop-linux-built" 2>/dev/null || true)
    if [ -z "$last_desktop" ] || ! git -C "$REPO" diff --quiet "$last_desktop" "$target" -- crates web Cargo.toml Cargo.lock deploy/desktop-linux 2>/dev/null; then
      systemctl --user start --no-block "slop-desktop-linux@$target.service" || log "couldn't start the Linux desktop build"
    fi
    # Keep the last three builds.
    ls -1dt "$ROOT"/builds/*/ | tail -n +4 | xargs -r rm -rf
    exit 0
  fi
  sleep 1
done

log "new build isn't healthy; rolling back"
touch "$ROOT/builds/failed-$target"
if [ -n "$previous" ]; then
  ln -sfn "$previous" "$ROOT/current.new" && mv -T "$ROOT/current.new" "$ROOT/current"
  systemctl --user restart slop-api.service
fi
exit 1
