# SLAPorSLOP — Dedicated servers linked to their games

**Status:** Built 2026-11-08 (migration 0094; DESIGN.md §5). Owner decisions are marked. The owner answered the open questions on 2026-11-08 (§9). Where the build differs from the proposal below, §11 says how.
**Written:** 2026-11-08
**Related:** [`DESIGN.md`](DESIGN.md) §5 (the Servers category, built 2026-11-07) · [`SECURITY.md`](SECURITY.md) (code from outside SLAPorSLOP) · [`ARCHITECTURE.md`](ARCHITECTURE.md)

---

## 1. What changes and why

Today a dedicated server is an ordinary app with the **Dedicated Server** tag in the Servers category, listed in the store on its own, and SLAPorSLOP doesn't link it to the game it's for (`DESIGN.md` §5; `catalog::IS_DEDICATED_SERVER`, `catalog.rs:941`). Reviewers only check that its description names the game (`review_guide.md:120`).

The owner wants instead (owner decisions, 2026-11-08):

1. **Separate repositories.** A game's client and its dedicated server live in different repos. SLAPorSLOP enforces it.
2. **One store entry.** A game and its servers are one listing in the store, not several.
3. **Separate library entries.** In the library the game and each of its servers are separate, and a player can get any of them. **The default is the client only.**
4. **Makers choose a server repo when publishing a game.**
5. **"Dedicated" means players run it.** It must be clear everywhere that a dedicated server is a program **players run on their own machines** (or a host they rent) so others can play. That is different from a game whose online play needs servers the maker runs and that aren't available to download.
6. **SLAPorSLOP only hands out the program.** Players launch and host servers themselves. SLAPorSLOP doesn't run servers, list the ones running, or match players to them. Whether a game has servers up is between its maker and its players.
7. **A game can have several servers.** Different server programs, each for players to run and manage themselves (a world server and a lobby server, say).

## 2. Model: the server is its own app, linked like a mod

Mods already work this way: a mod is its own app with its own repo, releases, review and signature, linked to its game by `parent_app_id` (`0024_branches.sql`, `0046_mods.sql`). A dedicated server follows the same pattern, without a store entry of its own.

- **Its own app.** It keeps its own `apps` row, `repo_url`, GitHub identity (`0069_repo_identity.sql`), releases, files per platform (`release_files`, one per platform), build, source check, scan, review and signed manifest. That pipeline is per app today and doesn't change.
- **The link.** A new nullable `apps.server_for_app_id` (references `apps.id`). Not unique: a game can have several servers (owner), each with its own name.
- **Always for a game.** A server is added from its game, and can be built and reviewed while the game is still a draft; it shows only where its game does (§8). A server-like program with no game on SLAPorSLOP is a tool, published as an ordinary app (owner).
- **Rules on the link**, checked by the API when it's set and again by reviewers:
  - **Same maker** as the game (`maker_id`), so nobody attaches a server to someone else's game.
  - **A different repository** from the game (`github_repo_id` differs). A repo that holds both client and server has to be split first. That's the owner's rule.
  - The server must be a server: today's checks still apply (Dedicated Server rules, `repo_scan.dedicated_server`, `source_scan.rs:1697,1736,1846`), and the reviewer also confirms it's a server for **this** game.
  - Not a browser game, a mod or a branch, and the game itself mustn't be a browser game (nothing to host).
- **Kind.** A linked server counts as a dedicated server everywhere `is_dedicated_server` is used today (fixes, labels, moderation, the "loads code from outside" exception), whether or not it carries the tag.

**Why not one app with two components?** Everything is keyed by app: releases are `unique(app_id, version)`, `release_files` is keyed `(release_id, platform)`, the manifest describes one app's files (`slop-manifest` `lib.rs:191-222`), `library_entries`, votes, Pulse, revocation, freezes, repo tokens and private sharing are all per app. A second app reuses all of that; components would touch nearly all of it.

## 3. The store: one entry

- A linked server is **not listed** on its own: not in search, categories, Browse, For you, featured shelves or "new". Its address redirects to its game's store page.
- The game's store page gets a **Dedicated server** section (**Dedicated servers** when there are several):
  > **Dedicated server** — Run your own server on your computer or a host, so others can play.

  with one row per server: its name, platforms, newest version, download size, its "What this app does" notes (including **Loads code from outside SLAPorSLOP** when the check finds it) and **Get the server**.
- **Votes** stay on the game. The server has no page to vote on.
- **Reports**: the game's Report menu gains "a dedicated server" (which one, when there are several) as what's reported, filed against the server's app so staff see the right repo and releases.
- **The Servers category** becomes a filter of games: **Games with a dedicated server**, instead of a list of server apps. Servers already in the category get nothing special (owner): no notice and no deadline. They stay listed as they are until their makers link them to their games.

### Two kinds of "server", never confused

| | Dedicated server | Online play on the maker's servers |
|---|---|---|
| Who runs it | Players, on their own machine or a host | The maker |
| Downloadable | Yes, from the game's page | No |
| Store page | **Dedicated server** section and **Get the server** | A label: **Online play uses the maker's servers** |
| Maker sets it | **Add a dedicated server** (a repo) | A checkbox on the game |

The publishing screen asks about both separately, with the wording above, so a maker can't pick one meaning the other.

## 4. The library: two entries, client by default

- `library_entries` is per app, so the game and each of its servers get one each. Nothing about that changes.
- **Get** on a game with a server adds the **client only**. Next to it, unticked: **Also get the dedicated server** (one box per server when there are several). A server can be added later from the game's page or the game's library entry.
- In the library each server shows as **<Game> — <server's name>**, grouped under its game, with its own install, update, launch and uninstall (`slop-client` `library.rs`: `apps/<slug>/`, one artifact per app, as now).
- **Uninstalling the game** asks whether to remove its servers too. Don't remove them automatically: unlike a mod, a server is useful on its own.
- **Playtime:** a server's running time doesn't count toward the game's Pulse playtime, "playing now" or peaks (`0010_pulse.sql`, `0072`). Either Pulse is off for servers, or it's counted as server time and never shown as playtime.

## 5. Servers without a desktop

Many players host on a Linux VPS where the desktop app can't run. The game's page and the server's library entry offer a **direct download** of the server's file for each platform, with its SHA-256 and how to verify the manifest's signature. A small command-line installer that verifies and updates a server can come later.

## 6. Publishing

On the game's **Publish** and **Manage** pages (`web/src/pages/Publish.tsx`, `Manage.tsx`), add an optional section:

> **Dedicated server (optional)**
> A program players run on their own computer or a host so others can play your game. It lives in its own repository and is built, checked and reviewed like any release. Not for servers only you run: for that, tick **Online play uses the maker's servers** instead.
>
> **Add a dedicated server** → its repository

That creates the server's app as a draft, linked to the game, named after the game plus "Dedicated server" (the maker can rename it, for example "Lobby server"), and opens its publishing progress (`PublishProgress.tsx`). Its releases then go through the usual flow. **Add another dedicated server** adds more; each one's repository must differ from the game's. Changing a server's repo once players have it goes through the existing reviewed repo change (`app_changes` `kind='repo'`).

## 7. Signing and older desktop apps

- **Keep the link out of the signed manifest at first.** The manifest uses `deny_unknown_fields` (`slop-manifest` `lib.rs:190`), so a new `server_for` field would make installed desktop apps that predate it refuse to install servers. The server is signed and verified on its own anyway; the link only decides what the store and library show.
- If the link should be signed later (so a client can prove a server belongs to a game), ship a desktop update that accepts an optional `server_for` field first (skipped when absent, like `mod_for`), wait until most installs have it, then start writing it.

## 8. Versions, takedowns and age

- **Versions:** each server has its own releases and version numbers, like any app (owner). SLAPorSLOP doesn't track which server versions work with which client versions: keeping them compatible is the maker's job. The game page shows each one's newest version.
- **Takedowns:** delisting, revoking or freezing the **game** hides its servers and stops new server downloads. Pulling only a **server** leaves the game alone, and its row says that server isn't available.
- **Age and content labels:** each keeps its own (`0057`); the game's page shows the strictest minimum age among the game and its servers, and each server's labels in its row.
- **Private sharing and drafts:** a server is visible to exactly those who can see its game.

## 9. Owner answers (2026-11-08)

1. **More than one server per game?** Yes. A game can have several server programs, each for players to run and manage themselves. Running servers aren't SLAPorSLOP's business: players host them, and SLAPorSLOP doesn't list them.
2. **A server before its game is listed?** A server-like program with no game is a tool or another kind of app, published as an ordinary app. A dedicated server belongs to a game, and it shows wherever that game does.
3. **Required server versions?** No. A server's versions work like any app's, and compatibility is up to the maker.
4. **A deadline for servers already in the Servers category?** No. Nothing special: publishing a server is like publishing any other kind of app.

## 10. Suggested build order

1. Migration: `server_for_app_id`, its rules, and treat linked apps as dedicated servers.
2. API: hide linked servers from listings and add them to the game's store data; link and unlink for makers; redirect the server's address.
3. Web: the store page section, the publishing section, the library grouping and checkbox.
4. Desktop: **Also get the dedicated server**, grouping, the uninstall question.
5. Pulse rules for servers, direct downloads with checksums, the Servers category as a filter.
6. Docs: `DESIGN.md` §5, `SECURITY.md`, the maker and review guides, and the Content Policy wording if "dedicated server" is defined there.

## 11. As built (2026-11-08)

- **The link** is the one mods use: `parent_app_id` is the game, with `branch_kind = 'server'`,
  not a new `server_for_app_id`. Every store listing already leaves out apps with a parent, so a
  server is out of search, categories, Browse and shelves on its own; the places that treat
  children as community builds or mods (the Community list, the library's builds, Play's picker,
  the platform filter, Pulse for branches, the signed manifest's `build_of`) leave servers out.
  `catalog::IS_DEDICATED_SERVER` covers linked servers and tagged ones.
- **Adding one:** `POST /v1/apps/{game}/servers` (and the `add_dedicated_server` tool), on the
  game's **Edit store page** (not Manage). Only the game's maker; the repository must differ from
  the game's, and since one repository is one app everywhere, each server has its own. A server
  takes the Dedicated Server tag, not the game's tags, and is an app (kind `app`), not a game.
- **Age:** a server's rating is at least its game's, as for a mod, so the game's page shows the
  game's rating and each server's own where it's higher.
- **Visibility:** a server is in the store only while its game is (`push_store_visible_as`);
  a private game's server is shared through the game's shares, and goes public, and in the store
  once ready, with the game (`admin::make_public`). It still needs a reviewer to list it, like any
  app.
- **Reports:** a **Report** button on each server's row, filed against the server's app.
- **Library:** each server under its game; a game's entry has a **Dedicated server** tab;
  uninstalling the game has an unticked **Uninstall its dedicated server too**.
- **Pulse:** launching a server gets no token and records nothing (no playtime, no launch days).
- **Not built:** a command-line installer for servers (§5); a game's Report menu choosing a server
  (each row has its own instead).
