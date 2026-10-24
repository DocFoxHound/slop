#!/usr/bin/env bash
# Deploys the newest commit on a branch, or the commit a tag names (deploy/README.md).
#
# SLOP_DEPLOY_MODE picks how, in a drop-in for slop-deploy.service:
#   build        the test server (deploy/README.md, "How changes ship"): SLOP_DEPLOY_BRANCH=dev,
#                every push deployed within minutes, built without the tests (GitHub runs them
#                when main moves forward to dev), then its desktop apps built here
#   production   production (deploy/README.md, "Production"): SLOP_DEPLOY_REF=production, the
#                commit the `production` tag points to, which must be on main (the branch);
#                built without the tests, which GitHub ran before the owner approved it
#   test         (the default) the tests, lints and browser tests first, then the build: how the
#                test server worked before the GitHub pipeline, kept for a server without one
#
# Other settings, for running it somewhere else (all have the defaults this server uses):
#   SLOP_DEPLOY_ROOT, SLOP_DEPLOY_REMOTE, SLOP_DEPLOY_BRANCH (main), SLOP_DEPLOY_REF
#   SLOP_DEPLOY_SERVICE (slop-api.service) and SLOP_DEPLOY_HEALTH_URL, the API and its check
#   SLOP_DEPLOY_ALSO_RESTART, units restarted with it (the simulation sandbox)
#   SLOP_DEPLOY_PG_CONTAINER (slop-pg) and SLOP_PROD_DATABASE (slop_prod), the snapshot's source
#   SLOP_DEPLOY_DESKTOP, the desktop apps built here after a deploy that changed the app: "linux"
#                (the default, both servers), "linux windows" (the test server: its "Slop Dev"
#                app for Windows too, cross-compiled), or empty
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
DEPLOY_TAG="${SLOP_DEPLOY_REF:-}"
MODE="${SLOP_DEPLOY_MODE:-test}"
SERVICE="${SLOP_DEPLOY_SERVICE:-slop-api.service}"
HEALTH_URL="${SLOP_DEPLOY_HEALTH_URL:-http://127.0.0.1:8090/healthz}"
ALSO_RESTART="${SLOP_DEPLOY_ALSO_RESTART-slop-sim.service slop-sim-pulse.service}"
PG_CONTAINER="${SLOP_DEPLOY_PG_CONTAINER:-slop-pg}"
DESKTOP="${SLOP_DEPLOY_DESKTOP-linux}"
export PATH="$HOME/.cargo/bin:$PATH"

log() { echo "slop-deploy: $*"; }
case "$MODE" in
  test | build | production) ;;
  *) log "SLOP_DEPLOY_MODE must be test, build or production, not '$MODE'"; exit 1 ;;
esac
for kind in $DESKTOP; do
  case "$kind" in
    linux | windows) ;;
    *) log "SLOP_DEPLOY_DESKTOP lists linux and windows, not '$kind'"; exit 1 ;;
  esac
done
# Production builds without the tests only what passed them on GitHub and was approved: the tag,
# never a branch's newest.
if [ "$MODE" = production ] && [ -z "$DEPLOY_TAG" ]; then
  log "SLOP_DEPLOY_MODE=production needs SLOP_DEPLOY_REF (the tag to deploy)"
  exit 1
fi
mkdir -p "$ROOT/builds"
exec 9>"$ROOT/.lock"
flock -n 9 || { log "another deploy is running"; exit 0; }

[ -d "$REPO/.git" ] || git clone -q "$REMOTE" "$REPO"
if [ -n "$DEPLOY_TAG" ]; then
  # The tag moves (.github/workflows/pipeline.yml), so it's fetched with force; an annotated tag
  # is peeled to its commit.
  git -C "$REPO" fetch -q origin "$BRANCH" "+refs/tags/$DEPLOY_TAG:refs/tags/$DEPLOY_TAG"
  target=$(git -C "$REPO" rev-parse --verify -q "refs/tags/$DEPLOY_TAG^{commit}") \
    || { log "tag $DEPLOY_TAG doesn't name a commit"; exit 1; }
else
  git -C "$REPO" fetch -q origin "$BRANCH"
  target=$(git -C "$REPO" rev-parse "origin/$BRANCH")
fi
current=$(basename "$(readlink "$ROOT/current" 2>/dev/null || echo none)")
[ "$target" = "$current" ] && exit 0
if [ -f "$ROOT/builds/failed-$target" ]; then exit 0; fi

log "deploying ${target:0:12}${DEPLOY_TAG:+ (tag $DEPLOY_TAG)} (running ${current:0:12})"
# A tag can be put on any commit, but production takes only what went through main (and so
# passed the pipeline's checks first).
if [ -n "$DEPLOY_TAG" ] && ! git -C "$REPO" merge-base --is-ancestor "$target" "origin/$BRANCH"; then
  log "REFUSED ${target:0:12}: tag $DEPLOY_TAG isn't on $BRANCH"
  touch "$ROOT/builds/failed-$target"
  exit 1
fi
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

# Tests and build, in the build container (only the build in the build and production modes).
# The image is rebuilt when its Containerfile changes.
if [ "$MODE" = test ]; then
  build_env=(-e SLOP_TEST_DATABASE_URL="$TEST_DATABASE_URL")
else
  build_env=(-e SLOP_BUILD_ONLY=1)
fi
image="localhost/slop-deploy-build:$(sha256sum deploy/build/Containerfile | cut -c1-16)"
if ! podman image exists "$image"; then
  podman build -q -t "$image" -f deploy/build/Containerfile deploy/build >"$ROOT/image.log" 2>&1 \
    || { tail -n 20 "$ROOT/image.log"; fail "build image"; }
fi
mkdir -p "$ROOT/cache/cargo" "$ROOT/cache/npm" "$ROOT/out"
rm -rf "$ROOT/out"/* target/release/slop-api web/dist
podman run --rm --network=host --userns=keep-id --cap-drop=all --security-opt=no-new-privileges \
  -e HOME=/tmp/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm \
  "${build_env[@]}" -e SLOP_COMMIT="$target" \
  -v "$REPO:/src:Z" -v "$ROOT/cache:/cache:Z" -v "$ROOT/out:/out:Z" -w /src \
  "$image" bash deploy/build/build.sh >"$ROOT/out/container.log" 2>&1 || true
if [ ! -e "$ROOT/out/ok" ] || [ -e "$ROOT/out/failed" ] || [ ! -x target/release/slop-api ] || [ ! -d web/dist ]; then
  # The step name comes from the commit's own script: shown, never run.
  step=$(head -c 40 "$ROOT/out/failed" 2>/dev/null | tr -cd 'a-z ' || true)
  step=${step:-container}
  # Only a summary for the log: whatever it hits (no match, head closing the pipe), the commit is
  # still marked failed below, so it isn't tested again every two minutes.
  case "$step" in
    tests) grep -E "panicked at|FAILED|^failures:|^    [a-z_]+$" "$ROOT/out/test.log" | tail -n 30 ;;
    lint) tail -n 30 "$ROOT/out/lint.log" ;;
    "browser tests") grep -E "✘|Error:|failed" "$ROOT/out/e2e.log" | head -n 30 ;;
    *) tail -n 30 "$ROOT/out/container.log" ;;
  esac || true
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
[ ! -d deploy/desktop-windows ] || cp -r deploy/desktop-windows "$build/desktop-windows"

# A copy of the database from just before the new build's migrations run, so a bad migration can
# be undone by hand (deploy/README.md, "Rolling back"). The last three are kept. If it can't be
# taken, nothing is switched; the next run tries again.
PROD_DB="${SLOP_PROD_DATABASE:-slop_prod}"
# shellcheck disable=SC2174 # $ROOT exists; only predeploy is private
mkdir -p -m 700 "$ROOT/predeploy"
snapshot="$ROOT/predeploy/$PROD_DB-$(date -u +%Y%m%dT%H%M%SZ)-${target:0:12}.dump"
if ! (umask 077; podman exec "$PG_CONTAINER" pg_dump -U slop --format=custom --compress=6 "$PROD_DB" >"$snapshot.partial"); then
  rm -f "$snapshot.partial"
  log "couldn't snapshot $PROD_DB before switching; will try again"
  exit 1
fi
mv "$snapshot.partial" "$snapshot"
# shellcheck disable=SC2012 # names this script made: no spaces
ls -1t "$ROOT"/predeploy/*.dump | tail -n +4 | xargs -r rm -f

previous=$(readlink "$ROOT/current" 2>/dev/null || true)
ln -sfn "$build" "$ROOT/current.new" && mv -T "$ROOT/current.new" "$ROOT/current"
systemctl --user restart "$SERVICE"

for _ in $(seq 30); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' "$HEALTH_URL")" = 204 ]; then
    log "live: ${target:0:12}"
    # The simulation sandbox runs the same build, when it's set up (docs/SIMULATION.md).
    if [ -n "$ALSO_RESTART" ]; then
      # shellcheck disable=SC2086 # a list of units
      systemctl --user try-restart $ALSO_RESTART || log "couldn't restart $ALSO_RESTART"
    fi
    # The deploy script updates itself from the deployed commit.
    mkdir -p "$ROOT/bin"
    install -m 755 "$build/update.sh" "$ROOT/bin/update.sh.new" && mv "$ROOT/bin/update.sh.new" "$ROOT/bin/update.sh"
    # Desktop apps (SLOP_DEPLOY_DESKTOP): rebuilt in the background when the app changed since
    # the last build for that platform. Each build publishes to this server's own API.
    for kind in $DESKTOP; do
      last_desktop=$(cat "$ROOT/desktop-$kind-built" 2>/dev/null || true)
      if [ -z "$last_desktop" ] || ! git -C "$REPO" diff --quiet "$last_desktop" "$target" -- crates web Cargo.toml Cargo.lock "deploy/desktop-$kind" deploy/desktop-config.js 2>/dev/null; then
        systemctl --user start --no-block "slop-desktop-$kind@$target.service" || log "couldn't start the $kind desktop build"
      fi
    done
    # Known-vulnerable dependencies (OSV), for the admins' alerts (ops.rs). Never blocks a
    # deploy: a new advisory against an old dependency shouldn't stop a fix from going out.
    audit_dependencies || log "dependency audit didn't run"
    # Keep the last three builds.
    # shellcheck disable=SC2012 # commit names: no spaces
    ls -1dt "$ROOT"/builds/*/ | tail -n +4 | xargs -r rm -rf
    exit 0
  fi
  sleep 1
done

log "new build isn't healthy; rolling back"
touch "$ROOT/builds/failed-$target"
if [ -n "$previous" ]; then
  ln -sfn "$previous" "$ROOT/current.new" && mv -T "$ROOT/current.new" "$ROOT/current"
  systemctl --user restart "$SERVICE"
fi
exit 1
