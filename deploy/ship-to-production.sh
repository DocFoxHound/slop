#!/usr/bin/env bash
# Ships the commit the test server is running to production (deploy/README.md, "Production").
#
#   deploy/ship-to-production.sh [--yes]
#
# Run on the test server, in a checkout with push access to GitHub (not the deploy's own clone,
# whose key can only read). It moves the annotated tag `production` to the commit in
# ~/slop-deploy/current and pushes only that tag; production's update.sh deploys it within a few
# minutes, and the desktop workflow builds production's app. Only a commit that's live here, so
# passed every test, and that's on main can be shipped.
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
TAG=production
yes=
for arg in "$@"; do
  case "$arg" in
    --yes | -y) yes=1 ;;
    -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
die() { echo "ship-to-production: $*" >&2; exit 1; }

cd "$(dirname "$0")/.."
git rev-parse --git-dir >/dev/null 2>&1 || die "run it from a Slop checkout"

# What the test server is running: current -> builds/<commit>.
link=$(readlink "$ROOT/current" 2>/dev/null) || die "nothing is deployed here ($ROOT/current is missing)"
commit=$(basename "$link")
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "$ROOT/current points to '$link', not a build"

git fetch -q origin main
# The tag as GitHub has it now: the tag object (for the push's lease) and the commit it names.
remote_tag=$(git ls-remote --refs origin "refs/tags/$TAG" | cut -f1)
old=
if [ -n "$remote_tag" ]; then
  git fetch -q origin "+refs/tags/$TAG:refs/tags/$TAG"
  old=$(git rev-parse "refs/tags/$TAG^{commit}")
fi

git cat-file -e "$commit^{commit}" 2>/dev/null || die "${commit:0:12} isn't in this checkout"
git merge-base --is-ancestor "$commit" origin/main || die "${commit:0:12} isn't on origin/main"
[ "$commit" != "$old" ] || { echo "Production already runs ${commit:0:12}."; exit 0; }

echo "The test server runs ${commit:0:12}: $(git log -1 --format=%s "$commit")"
if [ -z "$old" ]; then
  echo "Production hasn't had a commit yet. Shipping everything up to it; the newest:"
  git log --oneline -n 20 "$commit"
elif git merge-base --is-ancestor "$old" "$commit"; then
  echo "New on production (since ${old:0:12}):"
  git log --oneline "$old..$commit"
else
  echo "WARNING: this takes production back from ${old:0:12}. These would no longer be live:"
  git log --oneline "$commit..$old"
  echo "(Desktop apps never go back a version: the desktop build of an older commit is refused.)"
fi
echo

if [ -z "$yes" ]; then
  read -r -p "Ship ${commit:0:12} to production? [y/N] " answer
  case "$answer" in
    y | Y | yes) ;;
    *) echo "Not shipped."; exit 1 ;;
  esac
fi

git -c user.name=DocFoxHound -c user.email=dochound1@gmail.com \
  tag -f -a -m "Production: ${commit:0:12}, shipped $(date -u +%Y-%m-%dT%H:%MZ)" "$TAG" "$commit" >/dev/null
# Only the tag, and only if nobody moved it since it was read above.
git push -q --force-with-lease="refs/tags/$TAG:$remote_tag" origin "refs/tags/$TAG:refs/tags/$TAG" \
  || die "not shipped: the tag changed on GitHub since it was read; run it again"
echo "Shipped: $TAG -> ${commit:0:12}. Production deploys it within a few minutes."
