# Slop: instructions for AI agents

Read `README.md` first. The design docs in `docs/` are the source of truth for behaviour;
`docs/LEGAL-GUIDANCE.md` §1–2 lists rules that code must not break, and things that need the
lawyer before anyone builds them.

## Stack

- **API:** Rust (stable, edition 2024), axum 0.8, sqlx 0.9 (Postgres), tokio. One modular
  application (`docs/ARCHITECTURE.md` §4): each module is a Rust module with its own `routes()`.
- **Database:** PostgreSQL only. Schema changes are new files in `crates/slop-api/migrations/`;
  never edit a migration that has been committed.
- **UI:** `web/`, React 19 + TypeScript + Vite + React Router. One UI for the website and the
  desktop app. Check `isDesktop` (`web/src/platform.ts`) for the few differences; links that
  leave Slop go through `openExternal` / `ExternalLink`, so the desktop app opens the system
  browser instead of navigating its own window.
- **Signed documents:** `crates/slop-manifest` defines release manifests and revocation lists and
  how they're signed. The API and the client must both use it; never hand-roll a format.
- **Client core:** `crates/slop-client` does everything security-sensitive on the player's
  device (tokens, sign-in, download verification, unpacking, launch checks) and is tested without
  Tauri. Put logic there, not in `slop-desktop`.
- **Desktop:** `crates/slop-desktop`, Tauri 2. Keep its capabilities (`capabilities/`) minimal;
  every new command the UI may call is attack surface (`docs/SECURITY.md` §7). New commands must
  be added to `build.rs` and granted in `capabilities/default.json`.
- **Staff actions** are `slop-api` subcommands (`src/admin.rs`), each written to `audit_log`.
  There is no web admin.

## Rules

- **Authorisation on the server for every request.** A thing the caller may not see returns 404,
  not 403, so private things can't be discovered (`docs/SECURITY.md` §8.1).
- **Personal data:** every new column holding personal data gets a `purpose:` comment in the
  migration and a row in `docs/RECORD-OF-PROCESSING.md` in the same change (PRIV-1). Collect as
  little as possible.
- **Age and country rules** live in the `country_policies` table (AGE-1). No country checks in
  feature code; use `policy::ViewerPolicy`.
- **Tokens are stored hashed** (`tokens::hash_token`). Never log tokens, cookies or secrets.
- **SQL:** bind every value. Build dynamic queries with `QueryBuilder` and `push_bind`, never by
  formatting user input into SQL.
- **Never link accounts because an email matches** (`docs/SECURITY.md` §4.1).
- Wording: say "scanned", never "safe" (`docs/LEGAL.md` §12). Don't show trust claims (scans,
  checks passed, build verification) until the feature behind them exists.
- **User-written Markdown** is rendered only through `web/src/markdown.ts` (raw HTML off, no
  images). Never put user content in `dangerouslySetInnerHTML` any other way.
- **Accessibility:** WCAG 2.2 AA (`docs/LEGAL-GUIDANCE.md` A11Y-1): labelled controls, keyboard
  use, visible focus, no colour-only meaning.
- Don't copy the mockup's fake data or shortcuts into real code.

## Before you finish

```sh
cargo fmt
cargo clippy --all-targets          # no warnings
DATABASE_URL=postgres://slop:slop@127.0.0.1:5433/slop cargo test
cd web && npm test && npm run typecheck
```

Add tests for new behaviour, especially who may see or change what. Commit only when the user
asks, and don't push or deploy without their say-so.
