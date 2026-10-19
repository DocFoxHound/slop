# The test deployment

Slop runs on the development server at **https://slop.188-68-41-241.sslip.io** and updates
itself from GitHub.

## What runs

| Unit (systemd, user `brian`) | What it does |
|---|---|
| `slop-pg.service` | Postgres in podman (`~/.config/containers/systemd/slop-pg.container`); data in the `slop-pg-data` volume. Dev and the deployment share the server; the deployment uses its own `slop_prod` database and role |
| `slop-api.service` | The API, the website (`SLOP_WEB_DIR`) and the release fetcher, on `127.0.0.1:8090` |
| `slop-deploy.timer` → `slop-deploy.service` | Every two minutes, `update.sh`: if `main` has a new commit, run the tests, build, switch over, restart, and roll back if it isn't healthy. A failing commit is skipped (marked in `~/slop-deploy/builds/failed-<sha>`) until the next push |

Caddy (`/etc/caddy/Caddyfile.d/slop.caddyfile`) serves it with a Let's Encrypt certificate. The
server's own `*.happysrv.de` name can't get certificates (the host's customers share, and have
used up, Let's Encrypt's weekly limit), so the deployment uses an `sslip.io` name, which resolves
to the IP in it. Use a real domain when there is one.

Settings and keys are in `~/.config/slop/slop.env` (mode 600; never committed). The release
signing key's public half, which desktop builds trust, is in `~/.config/slop/desktop-build.env`.

## The desktop app

`.github/workflows/desktop.yml` builds the Windows installer on every push to `main` that touches
the app, and uploads it to the deployment with the run's GitHub OIDC token. The server signs it
with the updater key (`SLOP_UPDATER_KEY`, only on the server). The app checks for updates shortly
after it starts and every 3 hours, and shows a notice ("Slop 0.1.N is available: Update and restart
/ Later"); updates are never installed without the player choosing to. The website's store page
links the newest installers.

Linux packages are built here after each deploy that changes the app
(`slop-desktop-linux@<commit>.service`, `deploy/desktop-linux/build.sh`), each in a container of the
oldest system it supports: a `.deb` on Ubuntu 22.04 (for Ubuntu 22.04+ and Debian 12+) and an
`.rpm` on Rocky Linux 10 (for RHEL/Rocky/Alma 10 with EPEL and CRB enabled; EL9 lacks WebKitGTK 4.1, which
Tauri 2 needs). They're published with `slop-api desktop publish`. All platforms share one version
number: `0.1.<number of commits>`.

## Day to day

```sh
journalctl --user -u slop-deploy -f          # watch deploys
journalctl --user -u slop-api -f             # the API's log
systemctl --user start slop-deploy           # deploy now instead of waiting
rm ~/slop-deploy/builds/failed-*             # retry a commit that failed
```

Reviewing and moderating is done in the website's **Admin** panel (accounts with admin rights).
Give an account admin rights from the server, after it has signed in once:

```sh
set -a; . ~/.config/slop/slop.env; set +a
~/slop-deploy/current/slop-api admin grant "<display name, email or account ID>" --reviewer <you>
```

The same staff commands also work from the server:

```sh
set -a; . ~/.config/slop/slop.env; set +a
~/slop-deploy/current/slop-api review pending
```

## Setting it up again

```sh
cp deploy/systemd/* ~/.config/systemd/user/ && systemctl --user daemon-reload  # incl. slop-desktop-linux@.service
install -m 755 deploy/update.sh ~/slop-deploy/bin/update.sh
cp deploy/slop.caddyfile /etc/caddy/Caddyfile.d/ && caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl --user enable --now slop-api.service slop-deploy.timer
```

plus `~/.config/slop/slop.env` (see `.env.example`; make keys with `slop-api keys generate` and
`slop-api keys generate-updater`, and put the updater's public key in
`crates/slop-desktop/tauri.conf.json`).
