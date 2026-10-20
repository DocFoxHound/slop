#!/bin/sh
# Refreshes the scanner's signatures and rules in /db (run with network, daily).
set -eu
mkdir -p /db/clamav /db/yara
cat > /tmp/freshclam.conf <<CONF
DatabaseDirectory /db/clamav
DatabaseMirror database.clamav.net
Foreground yes
CONF
freshclam --config-file=/tmp/freshclam.conf --quiet || freshclam --config-file=/tmp/freshclam.conf
# YARA-Forge's core set: the curated, low false-positive rules.
tag=$(curl -fsSL https://api.github.com/repos/YARAHQ/yara-forge/releases/latest | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')
if [ "$(cat /db/yara/version 2>/dev/null)" != "$tag" ]; then
  curl -fsSL -o /tmp/core.zip "https://github.com/YARAHQ/yara-forge/releases/download/$tag/yara-forge-rules-core.zip"
  rm -rf /tmp/core && mkdir /tmp/core && unzip -q /tmp/core.zip -d /tmp/core
  rules=$(find /tmp/core -name '*.yar' | head -1)
  yarac -w "$rules" /db/yara/core.yarc.new
  mv /db/yara/core.yarc.new /db/yara/core.yarc
  echo "$tag" > /db/yara/version
fi
echo "clamav $(ls /db/clamav | tr '\n' ' '); yara-forge $(cat /db/yara/version)"
