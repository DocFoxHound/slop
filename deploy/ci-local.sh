#!/usr/bin/env bash
# Runs GitHub's checks here, the way the pipeline does (.github/workflows/pipeline.yml):
# deploy/build/build.sh in the deploy/build/Containerfile image, CI=true, UTC, on a clean copy of
# a commit. To find out in minutes what a failed run on GitHub saw, instead of pushing and
# waiting for another run.
#
#   deploy/ci-local.sh [--e2e-only] [--order reverse|shuffle:<seed>] [commit]
#
# --e2e-only  only the website and its browser tests (skips the Rust tests, lint, release build)
# --order     the browser test files in another order (SLOP_E2E_ORDER, web/playwright.config.ts)
# commit      default HEAD; a clean copy is checked out in $SLOP_CI_LOCAL (kept, with its build
#             folder, so later runs are quicker)
#
# The tests use the test Postgres, SLOP_TEST_DATABASE_URL (default the test server's,
# 127.0.0.1:5434). Logs land in $SLOP_CI_LOCAL/out; the summary is printed at the end.
set -euo pipefail

e2e_only=
order=
commit=HEAD
while [ $# -gt 0 ]; do
  case "$1" in
    --e2e-only) e2e_only=1 ;;
    --order) order="$2"; shift ;;
    -h | --help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    *) commit="$1" ;;
  esac
  shift
done

cd "$(dirname "$0")/.."
repo=$(pwd)
sha=$(git rev-parse --verify "$commit^{commit}")
work="${SLOP_CI_LOCAL:-$HOME/.cache/slop-ci-local}"
db="${SLOP_TEST_DATABASE_URL:-postgres://slop:slop@127.0.0.1:5434/slop}"

# A clean copy of the commit: everything but the build folder and node_modules is reset.
mkdir -p "$work"
if [ ! -d "$work/src/.git" ]; then
  git clone -q --no-checkout "$repo" "$work/src"
fi
git -C "$work/src" fetch -q "$repo" "$sha"
git -C "$work/src" checkout -q --detach -f "$sha"
git -C "$work/src" clean -q -fdx -e target -e web/node_modules

image="localhost/slop-deploy-build:$(sha256sum deploy/build/Containerfile | cut -c1-16)"
if ! podman image exists "$image"; then
  echo "Building the build image (once per Containerfile change)…"
  podman build -q -t "$image" -f deploy/build/Containerfile deploy/build >/dev/null
fi

mkdir -p "$work/cache/cargo" "$work/cache/npm" "$work/out"
rm -rf "$work/out"/*
env_args=(-e CI=true -e TZ=UTC -e CARGO_INCREMENTAL=0 -e SLOP_TEST_DATABASE_URL="$db" -e SLOP_COMMIT="$sha")
[ -n "$e2e_only" ] && env_args+=(-e SLOP_E2E_ONLY=1)
[ -n "$order" ] && env_args+=(-e SLOP_E2E_ORDER="$order")

echo "Checking ${sha:0:12}$([ -n "$e2e_only" ] && echo ", browser tests only")$([ -n "$order" ] && echo ", order $order") in $work…"
started=$(date +%s)
podman run --rm --network=host --userns=keep-id --cap-drop=all --security-opt=no-new-privileges \
  -e HOME=/tmp/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm "${env_args[@]}" \
  -v "$work/src:/src:Z" -v "$work/cache:/cache:Z" -v "$work/out:/out:Z" -w /src \
  "$image" bash deploy/build/build.sh >"$work/out/container.log" 2>&1 || true
took=$(( ($(date +%s) - started) / 60 ))

out="$work/out"
if [ -e "$out/ok" ] && [ ! -e "$out/failed" ]; then
  echo "All checks passed (${took} min)."
  exit 0
fi
step=$(head -c 40 "$out/failed" 2>/dev/null | tr -cd 'a-z ' || true)
step=${step:-container}
echo "Failed at: $step (${took} min). Logs: $out"
case "$step" in
  tests) grep -E "panicked at|FAILED|^failures:|^    [a-z_]+$" "$out/test.log" | tail -n 30 ;;
  lint) tail -n 30 "$out/lint.log" ;;
  "browser tests")
    grep -E "✘|Error:|^ *> *[0-9]+ \||failed" "$out/e2e.log" | head -n 40
    echo "Screenshots, traces and page snapshots: $work/src/web/e2e/.results" ;;
  *) tail -n 30 "$out/container.log" ;;
esac || true
exit 1
