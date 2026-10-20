# The test deployment

Slop runs on the development server at **https://slop.188-68-41-241.sslip.io** and updates
itself from GitHub.

## What runs

| Unit (systemd, user `brian`) | What it does |
|---|---|
| `slop-pg.service` | Postgres in podman (`~/.config/containers/systemd/slop-pg.container`); data in the `slop-pg-data` volume. Dev and the deployment share the server; the deployment uses its own `slop_prod` database and role |
| `slop-api.service` | The API, the website (`SLOP_WEB_DIR`) and the release fetcher, on `127.0.0.1:8090` |
| `slop-sim.service`, `slop-sim-pulse.service` | The simulation sandbox (`docs/SIMULATION.md`): the same build on the `slop_sim` database at `sim.188-68-41-241.sslip.io` (`127.0.0.1:8092`), and its simulated live players. Settings in `~/.config/slop/slop-sim.env`. Restarted with each deploy |
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

## GitHub App

Slop's GitHub App lets GitHub tell Slop when a maker publishes a release (a webhook), and gives
requests about each repository that has it installed their own API allowance. Without it Slop
polls on the shared `GITHUB_TOKEN`, which runs out at around 800 watched apps.

1. On GitHub: **Settings → Developer settings → GitHub Apps → New GitHub App**.
   - **Homepage URL:** the site, e.g. `https://slop.example`.
   - **Webhook URL:** `https://slop.example/v1/github/webhook`, **Active**. **Webhook secret:** a
     long random string (`openssl rand -hex 32`).
   - **Repository permissions:** Contents **Read-only**, Metadata **Read-only**.
   - **Organization permissions** (for official teams, `teams.rs`): Members **Read-only**, and
     Administration **Read-only** (to see whether the organization requires 2FA). Nothing else.
   - **Subscribe to events:** **Release**. (Installation events are always sent.)
   - **Where can this GitHub App be installed?** **Any account**, so makers can install it.
2. After creating it, note the **App ID** and the App's public name (in its public link,
   `github.com/apps/<name>`). Under **Private keys**, generate one; a `.pem` file downloads.
3. On the server: put the `.pem` at `~/.config/slop/github-app.pem` (`chmod 600`), and add to
   `slop.env`:

   ```sh
   GITHUB_APP_ID=123456
   GITHUB_APP_SLUG=<name>
   GITHUB_APP_PRIVATE_KEY_FILE=/home/brian/.config/slop/github-app.pem
   GITHUB_APP_WEBHOOK_SECRET=<the secret>
   ```

   then `systemctl --user restart slop-api`. GitHub's **Advanced** tab on the App shows each
   webhook delivery and Slop's answer (204 means handled).

Makers see an **Install on GitHub** link in their app's **Updates** box once it's configured.

## People payments (Stripe Connect)

Until Stripe keys are set, People works (profiles, portfolios, requests and offers) but nobody can
be paid, so nobody can be hired; pages say "Payments aren't switched on yet". The sandbox
simulates payments instead.

1. In Stripe: turn on **Connect** for the platform account (the creatives' accounts are their
   own, with the full Stripe dashboard; Stripe carries losses).
2. **Developers → Webhooks → Add endpoint** on **connected accounts**:
   `https://<site>/v1/stripe/webhook`, events `checkout.session.completed`,
   `checkout.session.async_payment_succeeded`, `charge.refunded`, `charge.dispute.created`,
   `account.updated`, and for server donations `invoice.paid` and `customer.subscription.deleted`.
3. Add to `~/.config/slop/slop-api.env` and restart:
   ```
   STRIPE_SECRET_KEY=sk_test_…   # sk_live_… only after the lawyer's review (LEGAL §4)
   STRIPE_WEBHOOK_SECRET=whsec_…
   ```
4. Before live keys: DAC7 and HMRC platform-reporting registration (LEGAL-GUIDANCE §16 checklist).
5. Once Slop has its EU OSS / UK VAT registrations, put the numbers in `SLOP_VAT_ID` so fee
   statements show them (e.g. `SLOP_VAT_ID=EU372000000 · GB000000000`).

## Email (notifications)

Until SMTP is set, notifications only show under the 🔔 in Slop. To email them too (each user
chooses which kinds on their account page), add to `~/.config/slop/slop-api.env` and restart:
```
SLOP_SMTP_URL=smtps://user:password@smtp.example.com:465
SLOP_EMAIL_FROM=Slop <notifications@your-domain>
```
Use a provider with a data processing agreement (PRIV-5), and set SPF/DKIM for the From domain.

## Setting it up again

```sh
cp deploy/systemd/* ~/.config/systemd/user/ && systemctl --user daemon-reload  # incl. slop-desktop-linux@.service
install -m 755 deploy/update.sh ~/slop-deploy/bin/update.sh
cp deploy/slop.caddyfile /etc/caddy/Caddyfile.d/ && caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl --user enable --now slop-api.service slop-deploy.timer
# Optional: the simulation sandbox (docs/SIMULATION.md)
podman exec slop-pg psql -U slop -c 'create database slop_sim'
systemctl --user enable --now slop-sim.service slop-sim-pulse.service
slop-api simulate generate   # with DATABASE_URL pointing at slop_sim
```

plus `~/.config/slop/slop.env` (see `.env.example`; make keys with `slop-api keys generate` and
`slop-api keys generate-updater`, and put the updater's public key in
`crates/slop-desktop/tauri.conf.json`).
