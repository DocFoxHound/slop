#!/usr/bin/env bash
# Deploys the newest commit on main, if it passes the tests (deploy/README.md).
#
# Run every two minutes by slop-deploy.timer. Builds in its own clone, never in a working copy,
# and runs the commit's code (tests, build scripts, npm scripts) only inside the build container
# (deploy/build/), which sees the source and a cache folder: no home folder, secrets or keys.
# On success it switches the `current` symlink and restarts slop-api.service; if the new build
# doesn't come up healthy, it switches back.
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
REPO="$ROOT/repo"
BRANCH="${SLOP_DEPLOY_BRANCH:-main}"
REMOTE="${SLOP_DEPLOY_REMOTE:-git@github.com:DocFoxHound/slop.git}"
# The test suite creates throwaway databases on this server (the dev superuser).
TEST_DATABASE_URL="${SLOP_TEST_DATABASE_URL:-postgres://slop:slop@127.0.0.1:5434/slop}"
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
# Signed commits only, once there's a list of who may sign (deploy/README.md, "Signed commits").
SIGNERS="$ROOT/allowed_signers"
if [ -s "$SIGNERS" ]; then
  if ! git -C "$REPO" -c gpg.format=ssh -c gpg.ssh.allowedSignersFile="$SIGNERS" verify-commit "$target" >/dev/null 2>&1; then
    log "REFUSED ${target:0:12}: not signed by a key in $SIGNERS"
    touch "$ROOT/builds/failed-$target"
    exit 1
  fi
fi
git -C "$REPO" checkout -q --force --detach "$target"
git -C "$REPO" clean -qfdx -e target -e web/node_modules
cd "$REPO"

fail() { log "FAILED at $1; staying on ${current:0:12}"; touch "$ROOT/builds/failed-$target"; exit 1; }

audit_dependencies() {
  podman run --rm --network=host -v "$REPO:/src:ro,z" ghcr.io/google/osv-scanner:v2.7.0@sha256:d2c46aa2483c4e02759bc25f2224d28a5ceeb3dca5a1ecd1f569fbed83a626f0 \
    scan source --lockfile /src/Cargo.lock --lockfile /src/web/package-lock.json --format json >"$ROOT/osv.json" 2>/dev/null || true
  python3 - "$ROOT/osv.json" "$target" "$REPO/deploy/audit-accepted.json" "$build/shipped.json" >"$ROOT/audit.json.new" <<'PY' && mv "$ROOT/audit.json.new" "$ROOT/audit.json"
import json, sys, datetime
# Only what's really shipped: Cargo.lock also lists optional dependencies nobody enables, and
# package-lock.json development tools. Worked out in the build container (deploy/build/build.sh);
# read here as data only.
try:
    shipped = {tuple(p) for p in json.load(open(sys.argv[4]))}
except Exception:
    sys.exit(1)
try:
    data = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
try:
    accepted = json.load(open(sys.argv[3]))["accepted"]
except Exception:
    accepted = {}
found = []
for r in data.get("results", []):
    for p in r.get("packages", []):
        pkg = p.get("package", {})
        if (pkg.get("name"), pkg.get("version")) not in shipped:
            continue
        for v in p.get("vulnerabilities", []):
            if v.get("id") in accepted or any(a in accepted for a in v.get("aliases", [])):
                continue
            found.append({"package": pkg.get("name"), "version": pkg.get("version"), "ecosystem": pkg.get("ecosystem"),
                          "id": v.get("id"), "summary": (v.get("summary") or "")[:200]})
print(json.dumps({"checked_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                  "commit": sys.argv[2], "vulnerabilities": found, "accepted": len(accepted)}, indent=1))
PY
}

# Tests and build, in the build container. The image is rebuilt when its Containerfile changes.
image="localhost/slop-deploy-build:$(sha256sum deploy/build/Containerfile | cut -c1-16)"
if ! podman image exists "$image"; then
  podman build -q -t "$image" -f deploy/build/Containerfile deploy/build >"$ROOT/image.log" 2>&1 \
    || { tail -n 20 "$ROOT/image.log"; fail "build image"; }
fi
mkdir -p "$ROOT/cache/cargo" "$ROOT/cache/npm" "$ROOT/out"
rm -rf "$ROOT/out"/* target/release/slop-api web/dist
podman run --rm --network=host --userns=keep-id --cap-drop=all --security-opt=no-new-privileges \
  -e HOME=/tmp/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm \
  -e SLOP_TEST_DATABASE_URL="$TEST_DATABASE_URL" -e SLOP_COMMIT="$target" \
  -v "$REPO:/src:Z" -v "$ROOT/cache:/cache:Z" -v "$ROOT/out:/out:Z" -w /src \
  "$image" bash deploy/build/build.sh >"$ROOT/out/container.log" 2>&1 || true
if [ ! -e "$ROOT/out/ok" ] || [ -e "$ROOT/out/failed" ] || [ ! -x target/release/slop-api ] || [ ! -d web/dist ]; then
  # The step name comes from the commit's own script: shown, never run.
  step=$(head -c 40 "$ROOT/out/failed" 2>/dev/null | tr -cd 'a-z ' || true)
  step=${step:-container}
  case "$step" in
    tests) grep -E "panicked at|FAILED|^failures:|^    [a-z_]+$" "$ROOT/out/test.log" | tail -n 30 ;;
    lint) tail -n 30 "$ROOT/out/lint.log" ;;
    "browser tests") grep -E "✘|Error:|failed" "$ROOT/out/e2e.log" | head -n 30 ;;
    *) tail -n 30 "$ROOT/out/container.log" ;;
  esac
  fail "$step"
fi

build="$ROOT/builds/$target"
rm -rf "$build" && mkdir -p "$build"
cp target/release/slop-api "$build/"
cp -r web/dist "$build/web"
# The software bill of materials for this build, served at /sbom.json (SECURITY §13).
cp "$ROOT/out/sbom.json" "$build/web/sbom.json" 2>/dev/null || log "no SBOM from the build"
cp "$ROOT/out/shipped.json" "$build/shipped.json" 2>/dev/null || true
cp deploy/update.sh "$build/update.sh"
cp -r deploy/desktop-linux "$build/desktop-linux"

# A copy of the database from just before the new build's migrations run, so a bad migration can
# be undone by hand (deploy/README.md, "Rolling back"). The last three are kept. If it can't be
# taken, nothing is switched; the next run tries again.
PROD_DB="${SLOP_PROD_DATABASE:-slop_prod}"
mkdir -p -m 700 "$ROOT/predeploy"
snapshot="$ROOT/predeploy/$PROD_DB-$(date -u +%Y%m%dT%H%M%SZ)-${target:0:12}.dump"
if ! (umask 077; podman exec slop-pg pg_dump -U slop --format=custom --compress=6 "$PROD_DB" >"$snapshot.partial"); then
  rm -f "$snapshot.partial"
  log "couldn't snapshot $PROD_DB before switching; will try again"
  exit 1
fi
mv "$snapshot.partial" "$snapshot"
ls -1t "$ROOT"/predeploy/*.dump | tail -n +4 | xargs -r rm -f

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
    # Known-vulnerable dependencies (OSV), for the admins' alerts (ops.rs). Never blocks a
    # deploy: a new advisory against an old dependency shouldn't stop a fix from going out.
    audit_dependencies || log "dependency audit didn't run"
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
