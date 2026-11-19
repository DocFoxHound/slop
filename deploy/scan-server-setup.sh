#!/usr/bin/env bash
# Sets up SLAPorSLOP's scanning server (deploy/README.md, "Scanning server") in the account it runs
# as (slopscan), once: the repository, the scanner image and its signatures, the token, and the
# services that keep it running what production runs. Run as that account, with its systemd:
#   sudo -iu slopscan env XDG_RUNTIME_DIR=/run/user/$(id -u slopscan) bash scan-server-setup.sh
# Safe to run again: it keeps the token and anything already done.
set -euo pipefail
ROOT="$HOME/slop-deploy"
REPO="$ROOT/repo"
REMOTE=https://github.com/DocFoxHound/slop.git
say() { echo "== $*"; }

[ "$(id -u)" != 0 ] || { echo "Run this as the scanning account, not root."; exit 1; }
grep -q "^$(id -un):" /etc/subuid || { echo "$(id -un) has no subordinate IDs for podman (/etc/subuid)."; exit 1; }
systemctl --user show-environment >/dev/null 2>&1 \
  || { echo "No systemd for this account: enable lingering and pass XDG_RUNTIME_DIR (see the top of this file)."; exit 1; }

say "The repository, at the production tag"
mkdir -p "$ROOT/bin" "$ROOT/builds" "$HOME/.config/slop" "$HOME/.config/systemd/user/slop-deploy.service.d"
[ -d "$REPO/.git" ] || git clone -q "$REMOTE" "$REPO"
git -C "$REPO" fetch -q origin main "+refs/tags/production:refs/tags/production"
git -C "$REPO" checkout -q --force --detach production
grep -q "ScanServer" "$REPO/crates/slop-api/src/main.rs" \
  || { echo "The production tag doesn't have the scanning server yet: promote first, then run this again."; exit 1; }

say "The scanner image (a few minutes)"
podman build -q --ignorefile "$REPO/deploy/scanner/Containerfile.dockerignore" -t slop-scanner \
  -f "$REPO/deploy/scanner/Containerfile" "$REPO" >/dev/null
install -m 755 "$REPO/deploy/scanner/slop-scan" "$ROOT/bin/slop-scan"
git -C "$REPO" rev-parse HEAD >"$ROOT/scanner-built"

say "The token production sends"
ENV="$HOME/.config/slop/scan-server.env"
if ! grep -q '^SLOP_SCAN_TOKEN=' "$ENV" 2>/dev/null; then
  (umask 077; echo "SLOP_SCAN_TOKEN=$(openssl rand -hex 32)" >>"$ENV")
fi
chmod 600 "$ENV"

say "The services"
for unit in slop-scan-server.service slop-deploy.service slop-deploy.timer slop-scan-update.service slop-scan-update.timer; do
  install -m 644 "$REPO/deploy/systemd/$unit" "$HOME/.config/systemd/user/$unit"
done
install -m 644 "$REPO/deploy/systemd/slop-deploy-scan-server.conf" \
  "$HOME/.config/systemd/user/slop-deploy.service.d/scan-server.conf"
install -m 755 "$REPO/deploy/update.sh" "$ROOT/bin/update.sh"
systemctl --user daemon-reload
systemctl --user enable -q slop-scan-server.service slop-deploy.timer slop-scan-update.timer

say "Today's signatures and rules (a few minutes)"
if ! podman volume exists slop-scan-db; then
  podman volume create slop-scan-db >/dev/null
  podman run --rm --user 0 -v slop-scan-db:/db --entrypoint sh slop-scanner -c 'chown -R 900 /db'
fi
systemctl --user start slop-scan-update.service

say "The first deploy: builds the program and starts it (10-20 minutes)"
systemctl --user start slop-deploy.service || true
journalctl --user -u slop-deploy.service -n 5 --no-pager | tail -n 3
if [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8095/healthz)" = 204 ]; then
  say "The scanning server is running."
else
  say "It isn't answering yet: journalctl --user -u slop-deploy -u slop-scan-server"
fi
# Shown until a run gets this far, so a run that stopped halfway doesn't lose it.
if [ ! -e "$ROOT/token-shown" ]; then
  echo
  echo "Put these in production's slop.env (/home/slop/.config/slop/slop.env), then restart slop-api there:"
  echo "  SLOP_SCANNER=remote"
  echo "  SLOP_SCAN_URL=https://scan.188-68-41-241.sslip.io/"
  grep '^SLOP_SCAN_TOKEN=' "$ENV" | sed 's/^/  /'
  echo "The token is shown this once; it's kept in $ENV."
  touch "$ROOT/token-shown"
fi
