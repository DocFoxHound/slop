#!/usr/bin/env bash
# Builds Slop's Linux packages for a commit and publishes them for the desktop app's auto-updater
# (deploy/README.md). Run by slop-desktop-linux.service after a deploy that changed the app.
#
#   build.sh <commit> [deb|rpm ...]
#
# Each package is built in its own container, on the oldest system it supports, from a clean
# export of the commit. Caches live in podman volumes (slop-build-cache-<kind>).
set -euo pipefail

ROOT="${SLOP_DEPLOY_ROOT:-$HOME/slop-deploy}"
REPO="$ROOT/repo"
HERE="$(cd "$(dirname "$0")" && pwd)"
COMMIT="$1"; shift
KINDS=("${@:-deb rpm}")
read -r -a KINDS <<< "${KINDS[*]}"
log() { echo "slop-desktop-linux: $*"; }

exec 9>"$ROOT/.desktop-linux.lock"
flock 9

set -a; . "$HOME/.config/slop/slop.env"; . "$HOME/.config/slop/desktop-build.env"; set +a
VERSION="0.1.$(git -C "$REPO" rev-list --count "$COMMIT")"
SITE="${SLOP_PUBLIC_URL%/}"
log "building $VERSION from ${COMMIT:0:12}: ${KINDS[*]}"

for kind in "${KINDS[@]}"; do
  podman build -q -t "slop-build-$kind" -f "$HERE/Containerfile.$kind" "$HERE" >/dev/null
  src=$(mktemp -d "$ROOT/.desktop-src-XXXXXX")
  git -C "$REPO" archive "$COMMIT" | tar -x -C "$src"
  out="$ROOT/desktop-out/$kind"
  rm -rf "$out" && mkdir -p "$out"
  podman run --rm \
    -v "$src:/src:Z" -v "$out:/out:Z" -v "slop-build-cache-$kind:/cache" \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target -e npm_config_cache=/cache/npm \
    -e VERSION="$VERSION" -e KIND="$kind" \
    -e SLOP_API_URL="$SITE/" -e SLOP_TRUSTED_KEYS="$SLOP_TRUSTED_KEYS" -e SLOP_ALLOW_UNSIGNED=true \
    "slop-build-$kind" bash -euo pipefail -c '
      export PATH=/root/.cargo/bin:$PATH
      cd /src
      node -e "
        const fs = require(\"fs\"); const f = \"crates/slop-desktop/tauri.conf.json\";
        const c = JSON.parse(fs.readFileSync(f, \"utf8\")); c.version = process.env.VERSION;
        fs.writeFileSync(f, JSON.stringify(c, null, 2));"
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
