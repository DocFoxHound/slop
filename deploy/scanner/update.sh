#!/bin/sh
# Refreshes the scanner's signatures and rules in /db (run with network, every six hours).
set -eu
mkdir -p /db/clamav /db/yara
cat > /tmp/freshclam.conf <<CONF
DatabaseDirectory /db/clamav
DatabaseMirror database.clamav.net
Foreground yes
CONF
freshclam --config-file=/tmp/freshclam.conf --quiet || freshclam --config-file=/tmp/freshclam.conf
# YARA-Forge's core and extended packages, split into the rules that hold a file and the ones that
# only make a note, keeping only rules whose licence lets SLAPorSLOP use them (slop-scan-rules).
# The newest release's tag, from where github.com's "latest" page leads (GitHub's API allows few
# lookups without signing in, shared by everything on the server's address). With no answer,
# the rules already here are kept.
tag=$(curl -fsSI https://github.com/YARAHQ/yara-forge/releases/latest | tr -d '\r' \
  | sed -n 's|^[Ll]ocation: .*/releases/tag/\([0-9A-Za-z._-]*\)$|\1|p' || true)
if [ -z "$tag" ]; then
  [ -f /db/yara/hold.yarc ] || { echo "couldn't find YARA-Forge's newest release" >&2; exit 1; }
  echo "couldn't find YARA-Forge's newest release; keeping $(cat /db/yara/version)" >&2
  tag=$(cat /db/yara/version)
fi
if [ "$(cat /db/yara/version 2>/dev/null)" != "$tag" ] || [ ! -f /db/yara/hold.yarc ]; then
  rm -rf /tmp/yf && mkdir -p /tmp/yf/out
  for p in core extended; do
    curl -fsSL -o "/tmp/yf/$p.zip" "https://github.com/YARAHQ/yara-forge/releases/download/$tag/yara-forge-rules-$p.zip"
    mkdir "/tmp/yf/$p" && unzip -q "/tmp/yf/$p.zip" -d "/tmp/yf/$p"
  done
  slop-scan-rules "$(find /tmp/yf/core -name '*.yar' | head -1)" "$(find /tmp/yf/extended -name '*.yar' | head -1)" /tmp/yf/out
  yarac -w /tmp/yf/out/hold.yar /db/yara/hold.yarc.new
  yarac -w /tmp/yf/out/note.yar /db/yara/note.yarc.new
  mv /db/yara/hold.yarc.new /db/yara/hold.yarc
  mv /db/yara/note.yarc.new /db/yara/note.yarc
  rm -f /db/yara/core.yarc
  echo "$tag" > /db/yara/version
  rm -rf /tmp/yf
fi
echo "clamav $(ls /db/clamav | tr '\n' ' '); yara-forge $(cat /db/yara/version)"
