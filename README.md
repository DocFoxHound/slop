# Slop

A store, like Steam, for software made with AI: open source, scanned before it's listed, free to
download, and voted **Slop** or **Not Slop** by people who've used it. What it is and why:
[`docs/OVERVIEW.md`](docs/OVERVIEW.md). The full plan: [`docs/DESIGN.md`](docs/DESIGN.md).

Based on the mockup in `Mockup/mockups/slop`. A copy is kept in [`mockup/`](mockup/) as the
reference for screens and behaviour (open `mockup/index.html` in a browser). Its data is fake;
don't copy it into real code.

## Status

| Part | State |
|---|---|
| API server (`crates/slop-api`, Rust + axum + Postgres) | Working |
| Identity: GitHub sign-in (PKCE), sign-up (16+, country), sessions, devices page | Done |
| Desktop sign-in through the system browser (RFC 8252), rotating refresh tokens | Done |
| Catalog: categories, tags, listing, search, store pages, makers' drafts | Done |
| Per-country age policy (UK under-18 view), stage deadlines, dead projects | Done |
| Releases: submit, fetch-and-hash, staff review, Ed25519-signed manifests | Done (phase 1: review is by a person) |
| Revocation (kill switch), audit log, library | Done |
| Admin panel (review queue, listing, moderation, audit log) and makers' publishing progress | Done |
| Slop Pulse (optional): launch tokens, heartbeats, playing now, monthly peaks, playtime; "Share my playtime" | Done |
| Public publishing guide (`/publish/guide`, `/publishing.md`, `/llms.txt`) | Done |
| Rate limits per client IP (strictest on sign-in); hourly cleanup of expired sign-ins, sessions and tokens | Done |
| Desktop client core (`crates/slop-client`): verified install, launch, revocation checks | Done, tested |
| Web UI (`web/`): store, library, install controls, publishing, releases, device approval | Done |
| Desktop app (`crates/slop-desktop`, Tauri 2) | Written; needs the Linux libraries below to build |
| Maker signatures: GitHub artifact attestations (Sigstore), verified by the API and again by the client | Done (GitHub repos) |
| Scan step in the pipeline; scanning itself off for now (files marked "not scanned") | Placeholder (`docs/ARCHITECTURE.md` §6.7) |
| Slop Build: shared GitHub Actions recipe (`slop-build/`), OIDC-authenticated submissions, "Add Slop Build" setup | Written and linted; first real run needs a sample repo |
| Self-hosted scan service, Slop Build for Godot/web/Electron/Python, maker keys for hand-built files, ratings, Pulse, payments… | Later (`docs/ARCHITECTURE.md` §8) |

## Test deployment

Running at **https://slop.188-68-41-241.sslip.io**, updated automatically from `main`
(`deploy/README.md`). The Windows app is built on every push and updates itself. Testing with
private repositories: `docs/TESTING.md`.

## Layout

```
crates/slop-api/        API server, release fetcher and staff commands
crates/slop-manifest/   Signed release manifests and revocation lists (shared)
crates/slop-provenance/ Maker signatures: GitHub artifact attestation checks (shared)
crates/slop-client/     Desktop client core: sign-in, verified installs, launch
crates/slop-desktop/    Desktop app (Tauri 2): web/ plus slop-client
  migrations/           Postgres schema (run automatically at start-up)
  src/auth/             Identity: GitHub OAuth, sign-up, sessions
  src/catalog.rs        Catalog: categories, tags, apps
  src/policy.rs         What a viewer may see, from the country policy table
  tests/api.rs          End-to-end tests (real Postgres, fake GitHub)
web/                    The UI, shared by the website and the desktop app
slop-build/             Slop Build: the shared build recipe (moves to its own repo)
dev/                    Local Postgres and sample data
deploy/                 The self-updating test deployment (systemd, Caddy, update script)
docs/                   Design, architecture, security, legal and payment docs
mockup/                 The original clickable mockup (reference only)
```

## Develop

Needs Rust (stable, via [rustup](https://rustup.rs)), Node 22+, and podman or Docker.

```sh
./dev/db.sh                      # start Postgres on 127.0.0.1:5433
cp .env.example .env             # then edit; at least DATABASE_URL
set -a; . ./.env; set +a
cargo run -p slop-api            # runs migrations, listens on SLOP_BIND (:8080)
./dev/seed.sh                    # optional: sample apps for local browsing

cd web && npm install
npm run dev                      # website on http://localhost:5173 (proxies /v1 to the API)
```

For sign-in to work in development, set `SLOP_PUBLIC_URL=http://localhost:5173` so GitHub
returns to the Vite server.

### Desktop app

The desktop app is the same UI in a Tauri window. On Linux it needs WebKitGTK and a few other
libraries first ([Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)); on Fedora:

```sh
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel dbus-devel
```

Then, with the API running:

```sh
cd web
npm run desktop                  # dev window, hot reload
npm run desktop:build            # installers in target/release/bundle/
```

The desktop app is built for one Slop site and one set of signing keys:

- `SLOP_API_URL`: the Slop site (default `http://localhost:5173/`, the dev website, which proxies
  `/v1` to the API).
- `SLOP_TRUSTED_KEYS`: Slop's public signing key(s), comma-separated. Without one, the app can
  browse but won't install anything.

The page never talks to the API directly in the desktop app: requests go through Rust, which holds
the user's tokens in the OS secure store.

Like Steam, the app opens with a small sign-in window (`web/login.html`) before the store. If a
remembered sign-in still works, the store opens straight away; offline, it opens anyway so installed
apps can be played. Signing in happens in the system browser, where the user approves the device;
the sign-in window also shows the link, in case no browser opens. Signing out closes the store and
brings the sign-in window back. "Browse without signing in" opens the store signed out.

Remembering sign-ins needs the OS secure store: Keychain on macOS, Credential Manager on Windows,
and a Secret Service (GNOME Keyring or KWallet) on Linux. Without one the app still works but asks
to sign in each time it starts; it never falls back to a plain file.

## Releases and review

1. Make a signing key once: `cargo run -p slop-api -- keys generate`. Put `SLOP_SIGNING_KEY` in
   the API's `.env`, and `SLOP_TRUSTED_KEYS` in the environment when building the desktop app.
2. A maker builds their files in GitHub Actions and signs them with
   `actions/attest-build-provenance` (the Releases page shows the snippet), then submits a release
   on **Publish → their app → Releases**: a version, the commit, and a file URL per platform on
   their own release page. Slop downloads each file once to record its size and SHA-256, stores
   nothing else, and looks up and verifies each file's attestation: it must be signed through
   Sigstore by GitHub Actions, name the file's hash, and come from the app's repository at the
   release's commit. Approval is refused for any file without one.
3. Staff review it:

   ```sh
   cargo run -p slop-api -- review pending
   cargo run -p slop-api -- review approve <release-id> --reviewer <you>   # signs the manifest
   cargo run -p slop-api -- review reject <release-id> --reviewer <you> --reason "…"
   cargo run -p slop-api -- review list <slug> --reviewer <you>            # put the app in the store
   cargo run -p slop-api -- review stage <slug> beta --reviewer <you>
   cargo run -p slop-api -- revoke <release-id> --reviewer <you> --reason "…"   # the kill switch
   ```

   Every decision goes into the append-only `audit_log` table.
4. The desktop app downloads from the maker's release page, refuses anything that doesn't match
   the signed manifest, installs into the user's library folder, and re-checks the signature, the
   revocation list and the program's hash at every launch.

The desktop app checks the attestation again itself before downloading, and refuses an update
whose source repository differs from the installed app's, so even a compromised Slop server can't
push a build the maker didn't sign.

For local testing with release files served over plain http and without attestations, set
`SLOP_ALLOW_HTTP_DOWNLOADS=true` and `SLOP_REQUIRE_MAKER_SIGNATURE=false` on the API (never in
production). Debug builds of the desktop app pointed at `localhost` relax the same two checks.

## Test

```sh
DATABASE_URL=postgres://slop:slop@127.0.0.1:5433/slop cargo test
cargo clippy --all-targets
cargo fmt --check
cd web && npm test && npm run typecheck
cd web && npm run build && npm run e2e
```

Integration tests create a fresh database per test on the server in `DATABASE_URL`. The browser
tests (`web/e2e`, Playwright) start a throwaway Slop (`crates/slop-api/examples/e2e_server.rs`)
on a new `slop_e2e_…_sim` database there, with GitHub faked, and drop it afterwards; they also
check the main pages for accessibility problems (axe). Set `SLOP_TEST_GODOT` to a Godot 4 binary
to run Slop Pong against the real Slop Link too.

## API (v1)

| Method & path | Who | What |
|---|---|---|
| `GET /healthz` | anyone | 204 when the database is reachable |
| `GET /v1/auth/github/start?return_to=/path` | anyone | Redirects to GitHub |
| `GET /v1/auth/github/callback` | GitHub | Signs in, or sends a new user to `/signup` |
| `GET /v1/auth/signup` | new user | Suggested name, email, allowed countries |
| `POST /v1/auth/signup` | new user | `{display_name, country_code, age_confirmed}` creates the account |
| `POST /v1/auth/signout` | signed in | Ends this session |
| `GET /v1/auth/providers` | anyone | Which sign-in providers are set up |
| `GET /v1/countries` | anyone | Countries where sign-up is open |
| `GET /v1/me` | signed in | The account |
| `GET /v1/me/sessions`, `DELETE /v1/me/sessions/{id}` | signed in | Devices page |
| `GET /v1/categories` | anyone | Categories with their tags |
| `GET /v1/apps?q=&tag=&category=&stage=&kind=&sort=&limit=&offset=` | anyone | Store listing and search |
| `GET /v1/apps/{slug}` | anyone | Store page (makers also see their drafts) |
| `POST /v1/apps`, `PATCH /v1/apps/{slug}` | maker | Create and edit drafts |
| `GET /v1/me/apps` | signed in | Your apps, including drafts |
| `GET /v1/apps/{slug}/releases`, `POST …` | anyone / maker | Approved releases (makers see all, with status); submit a release |
| `GET /v1/apps/{slug}/manifest` | anyone | Newest approved release's signed manifest |
| `GET /v1/revocations` | anyone | Signed revocation list (valid 6 hours) |
| `GET /v1/keys` | anyone | Slop's public signing key |
| `GET /v1/server` | anyone | What this server checks (scanning, maker signatures) |
| `POST /v1/apps/{slug}/launch` | signed in (desktop app) | Records a launch; issues a Pulse token |
| `POST /v1/pulse/start`, `/heartbeat`, `/end` | the game (Pulse token) | Slop Pulse pings |
| `GET /v1/apps/{slug}/players` | anyone | Playing now, peak this month, daily peaks |
| `PATCH /v1/me` | signed in | Settings ("Share my playtime") |
| `POST /v1/slop-build/releases` | Slop Build (GitHub OIDC token) | A release built by the recipe |
| `GET /v1/apps/{slug}/slop-build` | maker | Detected project type and the workflow to add |
| `GET /v1/me/library`, `PUT`/`DELETE /v1/me/library/{slug}` | signed in | The library |
| `POST /v1/auth/desktop/requests` | desktop app | Start desktop sign-in |
| `GET /v1/auth/desktop/requests/{id}`, `POST …/approve` | signed in (website) | Approve a desktop device |
| `POST /v1/auth/desktop/token` | desktop app | Code or refresh token → access + refresh tokens |

Errors are `{"error": {"code", "message", "field"?}}`. The desktop app authenticates with
`Authorization: Bearer <access token>`; the website with its session cookie.

## License

Copyright © 2026 IronPoint Development LLC.

Slop is open source under two licences, so improvements to the service stay open while code that
makers and other stores build on stays free to use in anything:

| Part | Licence |
|---|---|
| The server (`crates/slop-api`), the website (`web/`), the desktop app (`crates/slop-desktop`, `crates/slop-client`), and the deployment and development scripts (`deploy/`, `dev/`) | [GNU Affero General Public License v3.0 or later](LICENSE-AGPL) |
| Everything else, including the manifest and provenance crates (`crates/slop-manifest`, `crates/slop-provenance`), the engine snippets and examples (`examples/`), the build and check recipes (`slop-build/`), the docs and the mockup | [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option |

The AGPL means anyone who runs a changed version of Slop's server or website for others must offer
those users its source code. Code from the second group never brings the AGPL into your own game
or store: the snippets, the recipes and the manifest checker are yours to copy.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
this project is licensed as the part it changes is licensed above, without any additional terms or
conditions. See [CONTRIBUTING.md](CONTRIBUTING.md).
