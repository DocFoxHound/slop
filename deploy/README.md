# The test deployment

Slop runs on the development server at **https://slop.188-68-41-241.sslip.io** and updates
itself from GitHub.

## What runs

| Unit (systemd, user `brian`) | What it does |
|---|---|
| `slop-pg.service` | Production Postgres in podman (`~/.config/containers/systemd/slop-pg.container`, `127.0.0.1:5433`); data in the `slop-pg-data` volume. The `slop_prod` database and role, and `slop_sim`. Its superuser `slop` has a random password (`~/.config/slop/pg-superuser`); maintenance uses it through `podman exec` |
| `slop-pg-dev.service` | Development and test Postgres (`slop-pg-dev.container`, `127.0.0.1:5434`, user `slop:slop`); the deploy's tests create their throwaway databases here, never in the production cluster. Install: copy `deploy/systemd/slop-pg-dev.container` to `~/.config/containers/systemd/` and `systemctl --user daemon-reload && systemctl --user start slop-pg-dev` |
| `slop-api.service` | The API, the website (`SLOP_WEB_DIR`) and the release fetcher, on `127.0.0.1:8090` |
| `slop-sim.service`, `slop-sim-pulse.service` | The simulation sandbox (`docs/SIMULATION.md`): the same build on the `slop_sim` database at `sim.188-68-41-241.sslip.io` (`127.0.0.1:8092`), and its simulated live players. Settings in `~/.config/slop/slop-sim.env`. Restarted with each deploy |
| `slop-backup.timer` → `slop-backup.service` | Every night at 03:30 UTC, `slop-backup`: the `slop_prod` database, roles and `~/.config/slop` into `~/slop-backups` (below) |
| `slop-restore-check.timer` → `slop-restore-check.service` | Every Sunday, restores the newest backup into a scratch database and compares it with the live one |
| `slop-scan-update.timer` → `slop-scan-update.service` | Every six hours, fresh ClamAV signatures and YARA-Forge rules for the release scanner (below), in the `slop-scan-db` volume |
| `slop-deploy.timer` → `slop-deploy.service` | Every two minutes, `update.sh`: if `main` has a new commit, run the tests (Rust, web, and the browser tests in `web/e2e` against a throwaway Slop on the test database), build, switch over, restart, and roll back if it isn't healthy. A failing commit is skipped (marked in `~/slop-deploy/builds/failed-<sha>`) until the next push |

Caddy (`/etc/caddy/Caddyfile.d/slop.caddyfile`) serves it with a Let's Encrypt certificate. The
server's own `*.happysrv.de` name can't get certificates (the host's customers share, and have
used up, Let's Encrypt's weekly limit), so the deployment uses an `sslip.io` name, which resolves
to the IP in it. Use a real domain when there is one.

Settings and keys are in `~/.config/slop/slop.env` (mode 600; never committed). Behind Caddy it
must include `SLOP_CLIENT_IP_HEADER=x-forwarded-for` (Caddy replaces any value the visitor sends),
or every visitor shares one rate limit; the admin panel raises an alert when it's missing. The release
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
~/slop-deploy/current/slop-api admin grant <account ID> --reviewer <you>   # the ID from their profile link
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

Do this once in a Stripe **sandbox** (test keys, no real money) and again on the **live**
account at launch. Stripe calls webhooks "event destinations"; the steps are the same for both.

1. In Stripe: **Connect → Get started**. Slop is a platform whose sellers (makers, creatives) are
   paid directly and sign up through Stripe's own onboarding; give them the **full Stripe
   dashboard**, and **Stripe** covers losses (refunds, disputes). Slop creates the accounts the
   v1 way (`v1/accounts` with `controller`), so the Accounts v2 notes don't apply.
2. **Developers → Webhooks → Add destination**, scope **Connected accounts**, payload style
   **Snapshot**, the default API version (Slop reads both the older and newer shapes), type
   **Webhook endpoint**, URL `https://<site>/v1/stripe/webhook`. Events:
   `checkout.session.completed`, `checkout.session.async_payment_succeeded`,
   `checkout.session.expired`, `charge.refunded`, `charge.dispute.created`, `account.updated`,
   `invoice.paid`, `customer.subscription.deleted`.
3. A **second destination** at the same URL, scope **Your account** (fix bounties' saved cards),
   Snapshot, the same API version. Events: `checkout.session.completed`, `charge.refunded`,
   `charge.dispute.created`, `checkout.session.expired`.
4. Add to `~/.config/slop/slop.env` and restart. The secret key is under **Developers → API
   keys** (the publishable `pk_` key isn't used). Each destination has its own **signing secret**
   (on its page, **Reveal**); list both, comma-separated. Type them into the file yourself: never
   paste them into a chat or a ticket.
   ```
   STRIPE_SECRET_KEY=sk_test_…   # sk_live_… only after the lawyer's review (LEGAL §4)
   STRIPE_WEBHOOK_SECRET=whsec_connected…,whsec_platform…
   ```
   Check: an app's `GET /v1/apps/{slug}/donate` no longer says donations aren't switched on.
5. Before live keys: DAC7 and HMRC platform-reporting registration (LEGAL-GUIDANCE §16 checklist).
6. Once Slop has its EU OSS / UK VAT registrations, put the numbers in `SLOP_VAT_ID` so fee
   statements show them (e.g. `SLOP_VAT_ID=EU372000000 · GB000000000`).

## Contact address

The website's Contact page (`/contact`, the Terms' "contact page on Slop") publishes one address
for legal notices, authorities, copyright claims, security problems and everything else:
```
SLOP_CONTACT_EMAIL=slop@example.com
```
`/.well-known/security.txt` uses it too, unless `SLOP_SECURITY_CONTACT` names a different contact.

## Email (notifications)

Until SMTP is set, notifications only show under the 🔔 in Slop. To email them too (each user
chooses which kinds on their account page), add to `~/.config/slop/slop.env` and restart:
```
SLOP_SMTP_URL=smtps://user:password@smtp.example.com:465
SLOP_EMAIL_FROM=Slop <notifications@your-domain>
```
Use a provider with a data processing agreement (PRIV-5), and set SPF/DKIM for the From domain.

## More sign-ins (Google, GitLab, Bitbucket, Microsoft, Discord)

Each turns on once its app keys are in `~/.config/slop/slop.env`:

| Provider | Create the app | Callback URL | Scopes |
|---|---|---|---|
| Google | Google Cloud Console → APIs & Services → Credentials → OAuth client (Web) | `https://<site>/v1/auth/google/callback` | openid, email, profile |
| GitLab | gitlab.com → Preferences → Applications | `https://<site>/v1/auth/gitlab/callback` | read_user |
| Bitbucket | Workspace settings → OAuth consumers | `https://<site>/v1/auth/bitbucket/callback` | Account: Read, Email |
| Microsoft | portal.azure.com → App registrations → New registration ("Accounts in any organizational directory and personal Microsoft accounts"; platform Web), then Certificates & secrets → New client secret | `https://<site>/v1/auth/microsoft/callback` | openid, profile, email (asked at sign-in) |
| Discord | discord.com/developers/applications → New Application → OAuth2 (Client ID, Reset Secret, add the redirect) | `https://<site>/v1/auth/discord/callback` | identify, email (asked at sign-in) |

Then `GOOGLE_CLIENT_ID`/`GOOGLE_CLIENT_SECRET` (and likewise `GITLAB_…`, `BITBUCKET_…`, `MICROSOFT_…`,
`DISCORD_…`) and restart
the API. The API also needs `git` installed: it reads repositories on other hosts for ownership
codes (repo_owner.rs).

## Phone notifications (web push)

Phones and browsers get notifications through web push once the server has a VAPID key:
```
slop-api keys generate-vapid          # prints SLOP_VAPID_PRIVATE_KEY=…
SLOP_VAPID_PRIVATE_KEY=…              # in ~/.config/slop/slop.env (and the sandbox's env)
SLOP_VAPID_SUBJECT=https://your-domain   # or mailto:; push services contact it about abuse
```
Changing the key means everyone switches phone notifications on again.

## Release scanner

`SLOP_SCANNER=local` scans every release file before review (DESIGN §10, SECURITY §5.2). The API
runs `SLOP_SCAN_COMMAND` (here `~/slop-deploy/bin/slop-scan`), which starts the `slop-scanner`
container (`deploy/scanner`: ClamAV, YARA with YARA-Forge's core rules, capa) for one file:
no network, read-only, no capabilities, 4 GB of memory, 25 minutes, and the file on stdin to an
unprivileged user inside (an unmapped ID outside, not this account). Nothing in the file is ever
run. A file the scanner couldn't scan completely (an archive that won't unpack or is past the
limits, an engine error or timeout) is held for a person, like one it flags.

```sh
podman build -t slop-scanner deploy/scanner
podman volume create slop-scan-db
podman run --rm --user 0 -v slop-scan-db:/db --entrypoint sh slop-scanner -c 'chown -R 900 /db'
podman run --rm -v slop-scan-db:/db --entrypoint slop-scan-update slop-scanner   # first signatures
install -m 755 deploy/scanner/slop-scan ~/slop-deploy/bin/slop-scan
systemctl --user enable --now slop-scan-update.timer
# in slop.env: SLOP_SCANNER=local and SLOP_SCAN_COMMAND=/home/brian/slop-deploy/bin/slop-scan
```

Rebuild the image when `deploy/scanner` changes. Windows builds also need Microsoft Defender's
check, which arrives from Slop Build or Slop Check on GitHub's Windows runners
(`/v1/slop-build/checks`); with the scanner on, a Windows file can't be approved without a clean
one unless a reviewer clears it.

## Backups

`deploy/backup/slop-backup.sh` (installed as `~/slop-deploy/bin/slop-backup`) dumps `slop_prod`
with `pg_dump --format=custom`, checks the dump lists, and copies the roles and `~/.config/slop`
(the env files, so the signing keys) beside it. 14 daily and 8 weekly copies stay in
`~/slop-backups` (mode 700). `slop-restore-check` restores the newest into `slop_restore_check`
every Sunday, compares row counts with the live database, and drops it.

To restore for real:

```sh
podman exec slop-pg createdb -U slop slop_restored
podman exec -i slop-pg pg_restore -U slop --no-owner --role=slop_prod -d slop_restored < ~/slop-backups/daily/slop_prod-<time>.dump
# check it, stop slop-api, rename the databases, start slop-api
```

**These copies are on the same disk.** They cover mistakes, not losing the machine. Set
`SLOP_BACKUP_OFFSITE` in the backup service's environment to a command that copies its
arguments somewhere else (for example `rclone copy --` to Backblaze B2 or Cloudflare R2, whose
free tiers fit).

**Encrypt them to a key that isn't on this machine.** On your own computer:
`gpg --quick-generate-key "Slop backups" ed25519 encrypt never`, then
`gpg --armor --export "Slop backups" > slop-backups.pub` and keep the private key (and a copy of
it) somewhere safe off this server. Here: `gpg --import slop-backups.pub` and set
`SLOP_BACKUP_GPG_RECIPIENT=<its fingerprint>` in the backup service's environment
(`systemctl --user edit slop-backup.service`, `[Service]` `Environment=…`). From then on the
settings-and-keys archive is only stored encrypted (`config-*.tar.gz.gpg`), and everything sent
off-site is encrypted. The local database dumps stay readable for the weekly restore check.
Restore the keys with `gpg --decrypt config-<time>.tar.gz.gpg | tar -xz` on a machine that has
the private key.

## Staff and approvals

`slop-api admin grant <who>` makes someone staff with the owner role; the Admin → Staff tab gives
and takes the other roles. `SLOP_APPROVALS_REQUIRED` in `slop.env` is how many different staff
approve a sensitive action (withdrawing a release, delisting, suspending, changing roles). It's 1
while there's one person; set it to 2 and restart once there's a second.

## Alerts

The API checks every five minutes (`ops.rs`) and tells every admin under the 🔔, by phone push,
and by email once SMTP is set: a missing or stale backup or restore check
(`SLOP_OPS_BACKUP_DIR=/home/brian/slop-backups`), a commit whose tests failed
(`SLOP_OPS_DEPLOY_DIR=/home/brian/slop-deploy`), a nearly full disk, the scanner failing,
urgent and serious reports as they arrive and any report past its deadline, releases waiting
over a day, revocations, admin changes, and payments not matching Stripe. The admin queue lists
what's still open.

If the whole server is down, nothing on it can say so: point a free outside uptime check
(UptimeRobot, Healthchecks.io, Better Stack) at `https://<site>/healthz`.

## Encrypted fields

`SLOP_FIELD_KEY` (32 random bytes, base64) encrypts personal fields at rest, VAT numbers today.
Keep it with the other keys: without it they can't be read. It's set in `slop.env` and the
sandbox has its own.

## Private repositories outside GitHub

Makers' read-only tokens for private GitLab, Bitbucket and Gitea/Forgejo repositories
(`repo_tokens.rs`) are encrypted with `SLOP_REPO_TOKEN_KEY` in `slop.env` (32 random bytes,
base64: `openssl rand -base64 32`). Keep it with the other keys: losing or changing it makes
every saved token unreadable, and makers have to save theirs again. Unset, makers can't save
tokens. The sandbox has its own key.

The source check reads repositories on GitHub and on the hosts in `SLOP_SOURCE_HOSTS`
(default `gitlab.com=gitlab,codeberg.org=gitea`). Add a self-hosted GitLab or Gitea/Forgejo
server as `host=gitlab` or `host=gitea` to read its public repositories too.

## Database roles

The API runs as a database role that doesn't own the tables (migration 0050, SECURITY §9). It
reads and writes as usual, but only adds rows to payments, vote history and the staff audit log,
and can't switch off their append-only triggers; only the owner can. Migrations run as the owner
in their own step (`slop-api migrate`, `slop-migrate.service`), whose env file is the only place
the owner's password lives, so code running in the API can't reach it. Until this is set up, the
API runs migrations itself and logs "the API owns its tables" at start.

1. `deploy/db-roles.sh postgres://<owner>:<password>@<host>:<port>/<database>` (the current
   `DATABASE_URL`). It creates the role `slop_app` with a new password and prints two lines.
2. Put `SLOP_MIGRATE_DATABASE_URL=…` (the owner) in a new `~/.config/slop/slop-migrate.env`
   (mode 600). In `~/.config/slop/slop.env`, set the new `DATABASE_URL`, add
   `SLOP_MIGRATIONS=external`, and remove any `SLOP_MIGRATE_DATABASE_URL`.
3. Install the units: copy `deploy/systemd/slop-migrate.service` and `slop-api.service` to
   `~/.config/systemd/user/`, then `systemctl --user daemon-reload && systemctl --user restart slop-api`.
   Check the warning is gone: `journalctl --user -u slop-api -n 20`.

The simulation sandbox (`slop_sim`) wipes its own ledgers, so it keeps the owner's URL.

## The build container

`update.sh` never runs a commit's code on the server itself. Tests, lints, the release build, the
website build and the browser tests run in `deploy/build/Containerfile` (rootless podman, as the
deploying user but with no capabilities), which sees only the deploy clone, a cache folder
(`~/slop-deploy/cache`) and an output folder (`~/slop-deploy/out`, with each step's log). It has
the host's network so it can fetch crates and reach the test database on `127.0.0.1:5434`, but no
home folder, secrets or keys. The image is rebuilt when the Containerfile changes. A commit's
code still runs with the API's rights once it's deployed: that's what branch protection and
signed commits are for.

## Signed commits

Once `~/slop-deploy/allowed_signers` exists, `update.sh` deploys only commits signed by a key in
it (SSH signing; the file is in `ssh-keygen`'s allowed-signers format, one
`<email> <public key>` per line). Unsigned or wrongly signed commits are skipped and logged.

1. Sign your commits: `git config --global gpg.format ssh`,
   `git config --global user.signingkey ~/.ssh/id_ed25519.pub`, `git config --global commit.gpgsign true`.
   Add the same key on GitHub as a *signing* key.
2. `echo "dochound1@gmail.com $(cat ~/.ssh/id_ed25519.pub)" > ~/slop-deploy/allowed_signers`.
3. Push a signed commit and watch it deploy (`journalctl --user -u slop-deploy -f`).

## A read-only deploy key

The deploy fetches from GitHub over SSH. Give it its own key that can only read:

1. `ssh-keygen -t ed25519 -N "" -f ~/.ssh/slop_deploy -C slop-deploy`
2. On GitHub: the repository's Settings → Deploy keys → Add, paste `~/.ssh/slop_deploy.pub`,
   leave "Allow write access" off.
3. Add to `~/.ssh/config`:
   ```
   Host github-slop-deploy
     HostName github.com
     User git
     IdentityFile ~/.ssh/slop_deploy
     IdentitiesOnly yes
   ```
4. `git -C ~/slop-deploy/repo remote set-url origin git@github-slop-deploy:DocFoxHound/slop.git`
   and set `Environment=SLOP_DEPLOY_REMOTE=git@github-slop-deploy:DocFoxHound/slop.git` in
   `slop-deploy.service`.

## Rolling back

Before switching to a new build, `update.sh` dumps the production database to
`~/slop-deploy/predeploy/` (the last three are kept). If a build's migration went wrong, after
switching `current` back:

```
podman exec slop-pg createdb -U slop slop_prod_restored
podman exec -i slop-pg pg_restore -U slop --no-owner --role=slop_prod -d slop_prod_restored < ~/slop-deploy/predeploy/<file>.dump
```

then swap the databases' names while the API is stopped. Migrations should also stay compatible
with the release before them (add columns and tables; remove in a later release), so going back
a build rarely needs this.

## Log retention

Slop's database keeps no IP addresses, Caddy has no access log, and the API doesn't log them.
The system journal holds the services' own logs; keep it to 90 days at most (LEGAL-GUIDANCE §15)
in `/etc/systemd/journald.conf`:

```ini
[Journal]
MaxRetentionSec=90day
```

then `sudo systemctl restart systemd-journald`. Records with legal keeping periods (reports,
decisions, law-enforcement requests, agreement acceptances) are purged by the API itself
(`retention.rs`).

## Browser demos

New games with a web build can be played in the browser for their first 14 days (demos.rs). Off
until set up, and capped so it can't grow into a bill:

1. Copy the updated `deploy/slop.caddyfile` (it adds `play.188-68-41-241.sslip.io`) to
   `/etc/caddy/Caddyfile.d/` and reload Caddy.
2. In `~/.config/slop/slop.env`: `SLOP_PLAY_URL=https://play.188-68-41-241.sslip.io` and
   `SLOP_DEMO_DIR=/home/brian/slop-demos`, then `systemctl --user restart slop-api`.

Limits (optional settings): `SLOP_DEMO_DAYS` (14), `SLOP_DEMO_MAX` demos at once (10) and
`SLOP_DEMO_DAILY_GB` served per day across all demos (20); past the daily cap, players are told
to install the game instead. Demos are deleted when they end.

## Source code link

The server, website and desktop app are under the AGPL (README "License"), which asks a network
service to offer its users the source of the version they use. The website's **Source code** page
(`/source`, from `GET /v1/source`) says the code isn't public yet until `SLOP_SOURCE_URL` is set in
`~/.config/slop/slop.env` to the public repository (`https://…`, no trailing slash). The deploy
builds with `SLOP_COMMIT`, so the page then links the exact commit running.

## Setting it up again

```sh
cp deploy/systemd/* ~/.config/systemd/user/ && systemctl --user daemon-reload  # incl. slop-desktop-linux@.service
install -m 755 deploy/update.sh ~/slop-deploy/bin/update.sh
cp deploy/slop.caddyfile /etc/caddy/Caddyfile.d/ && caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl --user enable --now slop-api.service slop-deploy.timer
# The release scanner: see "Release scanner" above
# Optional: the simulation sandbox (docs/SIMULATION.md)
podman exec slop-pg psql -U slop -c 'create database slop_sim'
systemctl --user enable --now slop-sim.service slop-sim-pulse.service
slop-api simulate generate   # with DATABASE_URL pointing at slop_sim
```

plus `~/.config/slop/slop.env` (see `.env.example`; make keys with `slop-api keys generate` and
`slop-api keys generate-updater`, and put the updater's public key in
`crates/slop-desktop/tauri.conf.json`).
