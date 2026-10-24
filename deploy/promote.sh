#!/usr/bin/env bash
# Sends what's on dev to production (deploy/README.md, "How changes ship").
#
#   deploy/promote.sh [--yes]
#
# Fast-forwards main on GitHub to dev's newest commit and refuses anything that isn't a
# fast-forward, so main only ever gets commits that are already on dev: signed by you and live on
# the test server. GitHub's pipeline (.github/workflows/pipeline.yml) then runs every check and
# waits for your approval before production deploys. Run it in your own checkout, with your
# GitHub access (not the deploy's clone, whose key can only read).
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
ACTIONS=https://github.com/DocFoxHound/slop/actions
yes=
for arg in "$@"; do
  case "$arg" in
    --yes | -y) yes=1 ;;
    -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
die() { echo "promote: $*" >&2; exit 1; }

cd "$(dirname "$0")/.."
# The lists below print straight out: in a terminal, git would open a pager and wait there.
export GIT_PAGER=cat PAGER=cat
git rev-parse --git-dir >/dev/null 2>&1 || die "run it from a Slop checkout"

git fetch -q origin main dev || die "couldn't fetch main and dev from origin"
main=$(git rev-parse origin/main)
dev=$(git rev-parse origin/dev)
[ "$main" != "$dev" ] || { echo "main already has everything on dev (${dev:0:12})."; exit 0; }
if ! git merge-base --is-ancestor "$main" "$dev"; then
  echo "main has commits that aren't on dev:"
  git log --oneline "$dev..$main"
  die "not a fast-forward; bring them into dev first (and let the test server run them)"
fi

echo "dev is at ${dev:0:12}: $(git log -1 --format=%s "$dev")"
# What the test server runs (when this is the test server): current -> builds/<commit>.
if running=$(readlink "$ROOT/current" 2>/dev/null); then
  running=$(basename "$running")
  if [ -e "$ROOT/builds/failed-$dev" ]; then
    echo "WARNING: the test server couldn't deploy ${dev:0:12} (journalctl --user -u slop-deploy)."
  elif [ "$running" != "$dev" ]; then
    echo "Note: the test server runs ${running:0:12}, not ${dev:0:12} yet (it may still be building)."
  fi
fi
echo "New on main (and so on production, once you approve):"
git log --oneline "$main..$dev"
echo

if [ -z "$yes" ]; then
  read -r -p "Push ${dev:0:12} to main? [y/N] " answer
  case "$answer" in
    y | Y | yes) ;;
    *) echo "Not pushed."; exit 1 ;;
  esac
fi

# A plain push: GitHub refuses it if main moved meanwhile and it's no longer a fast-forward.
git push -q origin "$dev:refs/heads/main" || die "not pushed (main changed on GitHub?); run it again"
echo "main -> ${dev:0:12}."
echo "GitHub now runs the checks and builds the Windows app (about 20-30 minutes), then waits for"
echo "you: open $ACTIONS, pick the newest \"Pipeline\" run, Review deployments -> production ->"
echo "Approve and deploy. slopai.games deploys it within minutes after that, and its Windows app"
echo "follows."
