#!/usr/bin/env bash
# Tests and builds one commit inside the deploy's build container (deploy/build/Containerfile),
# started by deploy/update.sh. Reads /src (the commit), writes /out:
#   /out/ok              everything passed
#   /out/failed          the step that failed (only on failure)
#   /out/*.log           each step's output
#   /out/sbom.json       the software bill of materials (on success)
#   /out/shipped.json    the shipped dependencies, for the dependency audit (on success)
# The built files stay in /src (target/release/slop-api, web/dist) for update.sh to copy.
#
# GitHub's pipeline runs it the same way for its checks (.github/workflows/pipeline.yml).
#
# SLOP_BUILD_ONLY=1 (update.sh's build and production modes, deploy/README.md) skips the tests,
# lints and browser tests: the test server deploys dev quickly and leaves them to GitHub's
# pipeline, production deploys only commits that passed them there, and neither has a test
# database. The build, the SBOM and shipped.json are the same either way.
set -uo pipefail
cd /src || exit 1
out=/out
mkdir -p "$HOME"
fail() { echo "$1" >"$out/failed"; exit 1; }

# A build folder from before the container (built on the host, another system) is cleared once,
# so nothing compiled outside the container is mixed in.
if [ -d target ] && [ ! -e target/.built-in-container ]; then
  rm -rf target
fi
mkdir -p target && touch target/.built-in-container

# Two files whose names differ only in capitals are one file on Windows, and its build fails.
if command -v git >/dev/null && git -c safe.directory='*' rev-parse --git-dir >/dev/null 2>&1; then
  same=$(git -c safe.directory='*' ls-files | sort -f | uniq -di)
  if [ -n "$same" ]; then
    echo "names that differ only in capitals (one file on Windows): $same" >"$out/lint.log"
    fail lint
  fi
fi

# Only the release build is stamped with the commit (the tests check a plain build has none).
commit="${SLOP_COMMIT:-}"
unset SLOP_COMMIT
build_only="${SLOP_BUILD_ONLY:-}"
if [ "$build_only" != 1 ]; then
  # Tests, once more if the database was busy (someone else's run): real failures fail twice.
  run_tests() { DATABASE_URL="$SLOP_TEST_DATABASE_URL" cargo test -q --locked >"$out/test.log" 2>&1; }
  if ! run_tests; then
    grep -qE "PoolTimedOut|too many clients|connection refused" "$out/test.log" || fail tests
    sleep 60
    run_tests || fail tests
  fi
  cargo clippy --locked --all-targets -q -- -D warnings >"$out/lint.log" 2>&1 || fail lint
fi
SLOP_COMMIT="$commit" cargo build --locked -q --release -p slop-api >"$out/build.log" 2>&1 || fail build
if [ "$build_only" = 1 ]; then
  (cd web && npm ci --silent --no-audit --no-fund && npm run build --silent) >"$out/web.log" 2>&1 || fail "web build"
else
  (cd web && npm ci --silent --no-audit --no-fund && npm test --silent && npm run build --silent) >"$out/web.log" 2>&1 || fail "web build"
  (cd web && npx playwright test) >"$out/e2e.log" 2>&1 || fail "browser tests"
fi
python3 deploy/sbom.py . "$commit" >"$out/sbom.json" 2>"$out/sbom.log" || rm -f "$out/sbom.json"
python3 - >"$out/shipped.json" <<'PY' || rm -f "$out/shipped.json"
import json, sys
sys.path.insert(0, "deploy")
from sbom import built_crates, web_packages
print(json.dumps(sorted([list(p) for p in built_crates(".") | web_packages(".")])))
PY
touch "$out/ok"
exit 0
