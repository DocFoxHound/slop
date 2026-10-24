#!/usr/bin/env bash
# Cross-compiles Slop's Windows installer for a commit on this (Linux) server and publishes it for
# the desktop app's auto-updater (deploy/README.md, "The desktop app"). Run by
# slop-desktop-windows@.service after a dev deploy that changed the app; production's Windows app
# is built on GitHub's Windows runners instead (.github/workflows/pipeline.yml).
#
#   build.sh <commit>
#
# Built in its own container (Containerfile.windows: cargo-xwin, LLVM, NSIS) from a clean export
# of the commit, like the Linux packages (deploy/desktop-linux/build.sh). The caches (crates, npm,
# Microsoft's SDK files, the build folder) live in a podman volume. The container gets the
# version, the site and the release-signing public key; never the updater key, which only
# `slop-api desktop publish` uses, on this side, to sign the finished installer.
#
# SLOP_CONFIG_DIR (default ~/.config/slop) holds slop.env and desktop-build.env;
# SLOP_DESKTOP_WINDOWS_CACHE names the cache volume (default slop-build-cache-windows).
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
REPO="$ROOT/repo"
CONFIG="${SLOP_CONFIG_DIR:-$HOME/.config/slop}"
CACHE="${SLOP_DESKTOP_WINDOWS_CACHE:-slop-build-cache-windows}"
HERE="$(cd "$(dirname "$0")" && pwd)"
COMMIT="$1"
TARGET=x86_64-pc-windows-msvc
log() { echo "slop-desktop-windows: $*"; }

exec 9>"$ROOT/.desktop-windows.lock"
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
# A test site's app is "Slop Dev" (deploy/desktop-config.js) and accepts releases without GitHub
# attestations (private repositories can't make them, docs/TESTING.md). Only a test site's: the
# same rule as deploy/desktop-linux/build.sh and crates/slop-desktop/build.rs.
host=${SITE#*://}; host=${host%%[/:]*}
case "$host" in
  localhost | 127.0.0.1 | *.sslip.io) channel=dev; unsigned=(-e SLOP_ALLOW_UNSIGNED=true) ;;
  *) channel=production; unsigned=() ;;
esac
log "building $VERSION ($channel) from ${COMMIT:0:12} for $SITE"

image="localhost/slop-build-windows:$(sha256sum "$HERE/Containerfile.windows" | cut -c1-16)"
if ! podman image exists "$image"; then
  podman build -q -t "$image" -f "$HERE/Containerfile.windows" "$HERE" >/dev/null
fi
src=$(mktemp -d "$ROOT/.desktop-src-XXXXXX")
trap 'rm -rf "$src"' EXIT
git -C "$REPO" archive "$COMMIT" | tar -x -C "$src"
out="$ROOT/desktop-out/windows"
rm -rf "$out" && mkdir -p "$out"
podman run --rm --cap-drop=all --security-opt=no-new-privileges \
  -v "$src:/src:Z" -v "$out:/out:Z" -v "$CACHE:/cache" \
  -e HOME=/tmp/home -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target \
  -e npm_config_cache=/cache/npm -e XWIN_CACHE_DIR=/cache/xwin -e XWIN_ARCH=x86_64 \
  -e XDG_CACHE_HOME=/cache/xdg \
  -e VERSION="$VERSION" -e SITE="$SITE" -e CHANNEL="$channel" -e TARGET="$TARGET" \
  -e SLOP_API_URL="$SITE/" -e SLOP_TRUSTED_KEYS="$SLOP_TRUSTED_KEYS" "${unsigned[@]}" \
  "$image" bash -euo pipefail -c '
    mkdir -p "$HOME"
    cd /src
    # The version, the site the app talks to and updates from (this server), and for a test
    # site the "Slop Dev" name and identifier.
    node deploy/desktop-config.js
    cd web && npm ci --no-audit --no-fund --silent
    # The cache keeps earlier builds; clear their installers so only this version is published.
    bundle="/cache/target/$TARGET/release/bundle/nsis"
    rm -rf "$bundle"
    npm run --silent desktop:build:windows -- --runner cargo-xwin --target "$TARGET"
    cp "$bundle/"*"_${VERSION}_x64-setup.exe" /out/
  ' || { log "build failed"; exit 1; }
exe=$(ls "$out"/*"_${VERSION}_x64-setup.exe")
[ "$(echo "$exe" | wc -l)" = 1 ] || { log "expected one installer for $VERSION, found: $exe"; exit 1; }
log "built $(basename "$exe") ($(du -h "$exe" | cut -f1))"
# Signed for the updater and published by this server's own API (desktop_updates.rs).
"$ROOT/current/slop-api" desktop publish --platform windows --version "$VERSION" --commit "$COMMIT" "$exe"
echo "$COMMIT" > "$ROOT/desktop-windows-built"
log "done: $VERSION"
