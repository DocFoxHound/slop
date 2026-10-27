#!/usr/bin/env bash
# Builds SLAPorSLOP's Linux packages for a commit and publishes them for the desktop app's auto-updater
# (deploy/README.md). Run by slop-desktop-linux.service after a deploy that changed the app.
#
#   build.sh <commit> [deb|rpm ...]
#
# Each package is built in its own container, on the oldest system it supports, from a clean
# export of the commit. Caches live in podman volumes (slop-build-cache-<kind>; another prefix in
# SLOP_DESKTOP_CACHE_PREFIX). SLOP_CONFIG_DIR (default ~/.config/slop) holds slop.env and
# desktop-build.env.
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
REPO="$ROOT/repo"
CONFIG="${SLOP_CONFIG_DIR:-$HOME/.config/slop}"
HERE="$(cd "$(dirname "$0")" && pwd)"
COMMIT="$1"; shift
KINDS=("${@:-deb rpm}")
read -r -a KINDS <<< "${KINDS[*]}"
log() { echo "slop-desktop-linux: $*"; }

exec 9>"$ROOT/.desktop-linux.lock"
flock 9

# The server's own settings: the site, and the release-signing public key to trust.
set -a
# shellcheck source=/dev/null
. "$CONFIG/slop.env"
# shellcheck source=/dev/null
. "$CONFIG/desktop-build.env"
set +a
VERSION="0.1.$(git -C "$REPO" rev-list --count "$COMMIT")"
SITE="${SLOP_PUBLIC_URL%/}"
# A test site's app is "Slop Dev" (deploy/desktop-config.js), which installs beside "SLAPorSLOP", and
# accepts releases without GitHub attestations (private repositories can't make them,
# docs/TESTING.md). Only a test site's: the same rule as crates/slop-desktop/build.rs, which
# refuses to build an unsigned app for any other.
host=${SITE#*://}; host=${host%%[/:]*}
case "$host" in
  localhost | 127.0.0.1 | *.sslip.io) channel=dev; unsigned=(-e SLOP_ALLOW_UNSIGNED=true) ;;
  *) channel=production; unsigned=() ;;
esac
log "building $VERSION ($channel) from ${COMMIT:0:12} for $SITE: ${KINDS[*]}"

for kind in "${KINDS[@]}"; do
  podman build -q -t "slop-build-$kind" -f "$HERE/Containerfile.$kind" "$HERE" >/dev/null
  src=$(mktemp -d "$ROOT/.desktop-src-XXXXXX")
  git -C "$REPO" archive "$COMMIT" | tar -x -C "$src"
  out="$ROOT/desktop-out/$kind"
  rm -rf "$out" && mkdir -p "$out"
  podman run --rm \
    -v "$src:/src:Z" -v "$out:/out:Z" -v "${SLOP_DESKTOP_CACHE_PREFIX:-slop-build-cache-}$kind:/cache" \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target -e npm_config_cache=/cache/npm \
    -e VERSION="$VERSION" -e KIND="$kind" -e SITE="$SITE" -e CHANNEL="$channel" \
    -e SLOP_API_URL="$SITE/" -e SLOP_TRUSTED_KEYS="$SLOP_TRUSTED_KEYS" "${unsigned[@]}" \
    "slop-build-$kind" bash -euo pipefail -c '
      export PATH=/root/.cargo/bin:$PATH
      cd /src
      # The version, the site the app talks to and updates from (this server), and for a test
      # site the "Slop Dev" name and identifier.
      node deploy/desktop-config.js
      cd web && npm ci --no-audit --no-fund --silent
      # The cache keeps earlier builds; clear their packages so only this version is published.
      rm -rf "/cache/target/release/bundle/$KIND"
      npm run --silent "desktop:build:$KIND"
      cp /cache/target/release/bundle/$KIND/*"$VERSION"*.$KIND /out/
    ' || { log "$kind build failed"; rm -rf "$src"; exit 1; }
  rm -rf "$src"
  pkg=$(ls "$out"/*"$VERSION"*."$kind")
  [ "$(echo "$pkg" | wc -l)" = 1 ] || { log "expected one $kind package for $VERSION, found: $pkg"; exit 1; }
  "$ROOT/current/slop-api" desktop publish --platform "linux-$kind" --version "$VERSION" --commit "$COMMIT" "$pkg"
done
echo "$COMMIT" > "$ROOT/desktop-linux-built"
log "done: $VERSION"
