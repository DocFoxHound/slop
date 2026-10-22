#!/usr/bin/env bash
# Tests and builds one commit inside the deploy's build container (deploy/build/Containerfile),
# started by deploy/update.sh. Reads /src (the commit), writes /out:
#   /out/ok              everything passed
#   /out/failed          the step that failed (only on failure)
#   /out/*.log           each step's output
#   /out/sbom.json       the software bill of materials (on success)
#   /out/shipped.json    the shipped dependencies, for the dependency audit (on success)
# The built files stay in /src (target/release/slop-api, web/dist) for update.sh to copy.
set -uo pipefail
cd /src
out=/out
mkdir -p "$HOME"
fail() { echo "$1" >"$out/failed"; exit 1; }

# Tests, once more if the database was busy (someone else's run): real failures fail twice.
run_tests() { DATABASE_URL="$SLOP_TEST_DATABASE_URL" cargo test -q --locked >"$out/test.log" 2>&1; }
if ! run_tests; then
  grep -qE "PoolTimedOut|too many clients|connection refused" "$out/test.log" || fail tests
  sleep 60
  run_tests || fail tests
fi
cargo clippy --locked --all-targets -q -- -D warnings >"$out/lint.log" 2>&1 || fail lint
SLOP_COMMIT="$SLOP_COMMIT" cargo build --locked -q --release -p slop-api >"$out/build.log" 2>&1 || fail build
(cd web && npm ci --silent --no-audit --no-fund && npm test --silent && npm run build --silent) >"$out/web.log" 2>&1 || fail "web build"
(cd web && npx playwright test) >"$out/e2e.log" 2>&1 || fail "browser tests"
python3 deploy/sbom.py . "$SLOP_COMMIT" >"$out/sbom.json" 2>"$out/sbom.log" || rm -f "$out/sbom.json"
python3 - >"$out/shipped.json" <<'PY' || rm -f "$out/shipped.json"
import json, sys
sys.path.insert(0, "deploy")
from sbom import built_crates, web_packages
print(json.dumps(sorted([list(p) for p in built_crates(".") | web_packages(".")])))
PY
touch "$out/ok"
exit 0
