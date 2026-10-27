#!/usr/bin/env bash
# How much of this month's Windows code-signing allowance is used (deploy/signing.conf,
# docs/CODE-SIGNING.md).
#
#   deploy/signing-allowance.sh [--this-run]
#
# Counts the pipeline's runs on main since the allowance last started again: each run builds, and
# so signs, the Windows app once per attempt (a re-run signs again). Without --this-run (promote.sh)
# it says what one more release would bring it to; with it (the pipeline, whose own run is already
# counted) whether this run fits. GH_TOKEN, when set, is used to ask GitHub.
#
# Prints a summary, then `key=value` lines (used, after, limit, signing_on, fits). Exits 0 when the
# release fits the allowance, 3 when it would go over it, 4 when GitHub couldn't be asked.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=deploy/signing.conf
. deploy/signing.conf
this_run=0
[ "${1:-}" = "--this-run" ] && this_run=1

# python3, or python on GitHub's Windows runner.
py=$(command -v python3 || command -v python)
LIMIT="$SIGNING_MONTHLY_LIMIT" PER="$SIGNINGS_PER_RELEASE" DAY="$SIGNING_CYCLE_DAY" ON="$SIGNING_ON" THIS_RUN="$this_run" \
  "$py" -I - <<'EOF'
import datetime, json, os, sys, urllib.request

limit, per, day, on = (int(os.environ[k]) for k in ("LIMIT", "PER", "DAY", "ON"))
this_run = os.environ["THIS_RUN"] == "1"
today = datetime.datetime.now(datetime.timezone.utc).date()
start = today.replace(day=day)
if today.day < day:
    start = (start.replace(day=1) - datetime.timedelta(days=1)).replace(day=day)

url = ("https://api.github.com/repos/DocFoxHound/slop/actions/workflows/pipeline.yml/runs"
       f"?branch=main&event=push&created=%3E%3D{start.isoformat()}&per_page=100")
req = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json"})
if os.environ.get("GH_TOKEN"):
    req.add_header("Authorization", "Bearer " + os.environ["GH_TOKEN"])
try:
    with urllib.request.urlopen(req, timeout=20) as res:
        runs = json.load(res).get("workflow_runs") or []
except Exception as e:  # noqa: BLE001
    print(f"Couldn't count this month's releases: GitHub said {e}.")
    print(f"signing_on={on}")
    sys.exit(4)

# Every attempt of every run on main built the Windows app (and, once signing is on, signed it).
attempts = sum(int(r.get("run_attempt") or 1) for r in runs)
releases = attempts if this_run else attempts + 1
used = (releases - 1) * per
after = releases * per
fits = after <= limit
state = "signing is on" if on else "not signing yet: only counting"
print(f"Windows signing allowance since {start.isoformat()} ({state}):")
print(f"  {used} of {limit} signings used ({releases - 1} release build{'s' if releases - 1 != 1 else ''} x {per})"
      f"; this release makes it {after} of {limit}.")
if not fits:
    print(f"  That's over the allowance by {after - limit}.")
for k, v in (("used", used), ("after", after), ("limit", limit), ("signing_on", on), ("fits", int(fits))):
    print(f"{k}={v}")
sys.exit(0 if fits else 3)
EOF
