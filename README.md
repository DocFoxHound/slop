# Slop

A store, like Steam, for software made with AI: open source, scanned before it's listed, free to
download, and voted **Slop** or **Not Slop** by people who've used it. What it is and why:
[`docs/OVERVIEW.md`](docs/OVERVIEW.md). The full plan: [`docs/DESIGN.md`](docs/DESIGN.md).

Based on the mockup in `Mockup/mockups/slop`. A copy is kept in [`mockup/`](mockup/) as the
reference for screens and behaviour (open `mockup/index.html` in a browser). Its data is fake;
don't copy it into real code.

## Status

Milestone 1 (Foundation + Catalog) is in progress.

| Part | State |
|---|---|
| API server (`crates/slop-api`, Rust + axum + Postgres) | Started |
| Identity: GitHub sign-in (PKCE), sign-up with 16+ confirmation and country, sessions, devices page | Done |
| Catalog: categories, tags, store listing, search, store pages, makers' drafts | Done |
| Per-country age policy (UK under-18 view), stage deadlines, dead projects | Done |
| Web UI | Not started |
| Publishing, Slop Build, scanning, Distribution, Ratings, Pulse, payments… | Later (`docs/ARCHITECTURE.md` §8) |

## Layout

```
crates/slop-api/        API server
  migrations/           Postgres schema (run automatically at start-up)
  src/auth/             Identity: GitHub OAuth, sign-up, sessions
  src/catalog.rs        Catalog: categories, tags, apps
  src/policy.rs         What a viewer may see, from the country policy table
  tests/api.rs          End-to-end tests (real Postgres, fake GitHub)
dev/                    Local Postgres and sample data
docs/                   Design, architecture, security, legal and payment docs
mockup/                 The original clickable mockup (reference only)
```

## Develop

Needs Rust (stable, via [rustup](https://rustup.rs)) and podman or Docker.

```sh
./dev/db.sh                      # start Postgres on 127.0.0.1:5433
cp .env.example .env             # then edit; at least DATABASE_URL
set -a; . ./.env; set +a
cargo run -p slop-api            # runs migrations, listens on SLOP_BIND
./dev/seed.sh                    # optional: sample apps for local browsing
```

Sign-in needs a GitHub OAuth app (GitHub → Settings → Developer settings → OAuth Apps) with the
callback URL `<SLOP_PUBLIC_URL>/v1/auth/github/callback`. Without one the API runs with
sign-in off.

## Test

```sh
DATABASE_URL=postgres://slop:slop@127.0.0.1:5433/slop cargo test
cargo clippy --all-targets
cargo fmt --check
```

Integration tests create a fresh database per test on the server in `DATABASE_URL`.

## API (v1)

| Method & path | Who | What |
|---|---|---|
| `GET /healthz` | anyone | 204 when the database is reachable |
| `GET /v1/auth/github/start?return_to=/path` | anyone | Redirects to GitHub |
| `GET /v1/auth/github/callback` | GitHub | Signs in, or sends a new user to `/signup` |
| `GET /v1/auth/signup` | new user | Suggested name, email, allowed countries |
| `POST /v1/auth/signup` | new user | `{display_name, country_code, age_confirmed}` creates the account |
| `POST /v1/auth/signout` | signed in | Ends this session |
| `GET /v1/countries` | anyone | Countries where sign-up is open |
| `GET /v1/me` | signed in | The account |
| `GET /v1/me/sessions`, `DELETE /v1/me/sessions/{id}` | signed in | Devices page |
| `GET /v1/categories` | anyone | Categories with their tags |
| `GET /v1/apps?q=&tag=&category=&stage=&kind=&sort=&limit=&offset=` | anyone | Store listing and search |
| `GET /v1/apps/{slug}` | anyone | Store page (makers also see their drafts) |
| `POST /v1/apps`, `PATCH /v1/apps/{slug}` | maker | Create and edit drafts |
| `GET /v1/me/apps` | signed in | Your apps, including drafts |

Errors are `{"error": {"code", "message", "field"?}}`.
