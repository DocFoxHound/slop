# SLAPorSLOP — Design Document

**Status:** Built and running as a private test deployment; sections say what's built and when. The original clickable mockup is kept in `mockup/` for reference
**Last updated:** 2026-10-21
**Name:** renamed from Slop to SLAPorSLOP on 2026-10-27 (owner decision): the positive vote is now **Slaps** (was "Not Slop"), the negative one is still **Slop**, and "Private Slop" is now **Private**. The site moved from slopai.games to slaporslop.games the same day; slopai.games stays registered and still serves the API (`/v1/`: installed desktop apps, makers' build workflows, webhooks) while its pages redirect (`deploy/slop-prod.caddyfile`, `SLOP_FORMER_URLS`). The desktop app's installers keep the product name "Slop" so existing installs update in place (`deploy/README.md`, "The desktop app").
**Related:** [`ARCHITECTURE.md`](ARCHITECTURE.md) (how it's built) · [`PAYMENTS.md`](PAYMENTS.md) (donations and People payments) · [`SECURITY.md`](SECURITY.md) (security and trust) · [`METRICS.md`](METRICS.md) (donation & vote history) · `LEGAL.md` (private) (legal framework)

---

## 1. Vision

SLAPorSLOP is a Steam-style store for **apps and games made with AI**, from small utilities to full games. Lots of AI-made software is low effort; SLAPorSLOP doesn't hide that. It makes it **visible, scanned and fixable**:

- Players vote on whether something **Slaps** or is **Slop**.
- Everything is **open source**, so anyone can improve it through **branches**.
- Everything is **scanned** before anyone can install it.
- Everything is **free**. Makers are supported by **optional donations** that go straight to them. SLAPorSLOP charges makers a flat 5% platform fee and never holds the money.

## 2. Who it's for

| Audience | What they get |
|---|---|
| **Players / users** | A place to find scanned AI-made apps and games, with honest ratings and community fixes |
| **Publishers** | Free distribution, donations, a player count, and a community that fixes and ports their work |
| **Modders / porters** | First-class branches; their builds show up for players on platforms the publisher doesn't support |
| **Creatives (People)** | A directory where developers hire them to replace AI-generated assets with crafted ones |
| **Groups (Teams)** | Player communities, and verified developer studios |

## 3. Platforms

- **Desktop:** Windows, macOS, Linux. One **Tauri** app, shared with the phone app and the website UI (decided).
- **Page bar** (2026-10-25): a slim bar under the top bar with Back, Forward, Reload this page and the page's address (with Copy link), since the desktop app has no browser around it; there Alt+Left/Right (Cmd+[ and ] on a Mac), F5 and Ctrl+R work as in a browser, and Reload loads the page's data again without restarting the app. The website shows it only on wide screens; phones never.
- **Phone:** iOS and Android. It's the same app, with a phone layout and bottom tabs.
- Publishers can ship builds for any of the five platforms.
- **Send to phone:** on a computer, a user can send a phone app to a linked phone, which installs it next time it's online.
- **iOS constraint:** Apple only allows third-party app marketplaces in the EU. **iOS is undecided** (§17). Android allows direct APK installs.

## 4. Accounts

- Sign in with **Google, Apple, Microsoft, GitHub or Discord**. SLAPorSLOP stores no passwords and only receives a name and email.
- **Accounts are 16+ everywhere.** UK users (detected from IP, billing, app-store and stated country) start in an **under-18 view**: 18+ content is hidden, and only connections can message them. A real age check unlocks 18+ content. Donating and People jobs are 18+ everywhere (`LEGAL.md` §6).
- Browsing works signed out. Anything personal requires an account: library, Interested, votes, donations, reports, publishing, teams.
- **One account = one vote per app = one Slop Pulse count per game.** This is the main defence against botting.
- Users can link devices (computer and phones) to their account.
- **Linked sign-ins** (built 2026-10-20): GitHub, Google, GitLab, Bitbucket, Microsoft and Discord (each on once the server has its app keys; Microsoft's email is never used, since Microsoft doesn't verify it). Apple later: Sign in with Apple needs a paid Apple Developer membership. Account → Sign-ins links more from any signed-in session (no recent sign-in needed since 2026-10-25), and only by signing in to the account being linked, so nobody can link someone else's; never because emails match. Any linked sign-in opens the account; the last one can't be unlinked. Linked Git accounts are also how SLAPorSLOP knows which repositories are yours (§8).
- **The account page** (redesigned 2026-10-24, `web/src/pages/Account.tsx`, `accountNav.ts`) is laid out like an app's settings: a header with the person's picture, name, country and public profile; a list of sections (Profile, Sign-in & security, Notifications, This computer in the desktop app, AI agents & keys, Money, Moderation, Agreements, Your data) with a count or mark where one needs attention; and one section at a time, at `/account/<section>` (Profile is `/account`). Links to a part of the old one-column page (`/account#decisions`, from notifications and emails) open that part in its section. On phones the list is a row that scrolls sideways.
- **Profile picture** (built 2026-10-24, `avatars.rs`): Account → Picture shows a linked sign-in's picture (GitHub, GitLab, Google, Discord or Bitbucket; Microsoft shares none) or, by default, the first letter of the name. SLAPorSLOP keeps its own 128-pixel copy and serves it, so people who see it never contact the provider. GitHub and GitLab pictures are read again daily; Google, Discord and Bitbucket say where the picture is only at a sign-in, so it updates when the person signs in with them or presses Refresh picture (the link flow again). Unlinking that sign-in, closing the account or staff (Fix their profile) take it off.
- **Combining accounts:** if a sign-in being linked already has its own SLAPorSLOP account, signing in to it proves it's yours, and after a confirmation page everything moves into the signed-in account (apps, library, playtime, votes and reviews, friends, chats, teams, People jobs and page, donations, notifications, reports and decisions). Where both have the same thing, the signed-in account's stays; the other is closed and points at it. Not combined: two People pages or two donation Stripe accounts (one goes first), and suspended or closed accounts. Chat history stays on the other account's devices, which are removed; its chats continue on this account's devices.
- **Your data and closing your account** (built 2026-10-20). The account page downloads everything linked to the account as one file. **Closing** an account bans that sign-in and freezes what others rely on, instead of erasing it:
  - the sign-in can never be used on SLAPorSLOP again; the email, settings, library and playtime are removed;
  - votes keep counting, re-keyed to a random ID (METRICS §6);
  - a creative's page stays readable, frozen, with a "closed" notice, and takes no new work; job records, agreements and reviews stay;
  - the maker's apps leave the store (installs keep working).

  Open People jobs, unpaid amounts and open problems must be settled first, and so must a bounty pledge being charged for a confirmed delivery (the builder is owed it). Pledges not yet charged are withdrawn, and the backer's saved cards are removed at Stripe (their Stripe customer is deleted).
- **Terms** (built 2026-10-20). The Terms of Service are accepted at sign-up, the Publisher Agreement before a first app, and the People Terms before a creative profile or a hire. Each acceptance records the version. Changes are published 15 days before they take effect: a notice shows on every page, and once in effect, only the actions behind that agreement wait for it (accepting takes one tick). Every version stays readable at /legal. The texts are drafts until the lawyer has read them (`crates/slop-api/legal/`).

## 5. Store & discovery

**Built 2026-10-21 (`label.rs`):** every store page has **"What this app does"**: online and to where, saves, sandbox mode, outside code, purchases, mods, its own installer, shared runtimes, scans and licence, each marked "checked" (SLAPorSLOP's checks of the newest release) or "the maker says", with notes repeated in the install box. **Accessibility:** makers declare features (remappable controls, colourblind modes, subtitles, text size, one-handed, difficulty, no flashing, screen reader, controller); players who launched the app confirm or dispute each; the store filters by them. New shelves: **New and barely seen** (released in the last 60 days, in fewer than 25 libraries, a new order daily) and **Glow-ups** (% Slaps up 15 points or more since 60 days ago, with before and after). **No paid placement** is in the Content Policy.

**Front page, top to bottom:**
1. Featured carousel
2. **Popular this month**: most added to libraries this calendar month, with live player counts for games
3. **Coming soon & in beta**, with two views:
   - **For you:** upcoming apps that match tags in your library (weight 1) and your Interested list (weight 2), boosted by log(total interest). Each card says why it's shown ("Because you like Cozy & Roguelike"). Dead projects are never recommended.
   - **Most anticipated:** ranked by Interested count.
4. Browse by category
5. Most Slaps right now
6. Community branches worth installing
7. New & updated
8. "Fresh slop": the most 👎 Slop-voted apps, for fun

**Built 2026-10-20:** the front page in this order (For you sits under the carousel for signed-in players). Featured is picked by staff in the admin Front page tab (`featured.rs`, up to 8, audit-logged) and never moves by itself. **Built 2026-10-22 (`shelves.rs`):** staff can add shelves of their own (a title, a line, up to 24 apps in order; shown after For you, each with a page at `/shelf/{address}`, or kept off the front page) and hide any automatic shelf. Popular this month = most library adds since the 1st, then most players. Most Slaps right now = weighted rating from votes cast or changed in the last 30 days (5+ votes). Fresh slop = most weighted 👎 in the last 30 days. Game tiles show players now from Slop Pulse.

**Built 2026-10-24 (Steam-style shell, `web/src/components/StoreNav.tsx`):** the front page is all apps, full width; there's no category sidebar on it. A **store bar** along the top of store pages (the front page, list pages, app pages) has **Browse** (all categories and tags), **Recommended** (`/recommended`: For you for signed-in players; popular picks and a sign-in nudge for visitors), **Games**, **Kids & Family** (since 2026-10-26), **Utilities** and **More** (the other categories, Coming soon & beta, Everything, Made with people, Community builds, Teams, and the maker links), plus the store search, which left the top bar. SLAPorSLOP's own links (terms, Content Policy, Check a file, source code, contact) are at the bottom of store pages. List pages keep the sidebar for browsing. People has its own search.

**Sexually explicit content is banned** (decided, `LEGAL.md` §5.11). Non-explicit romance and suggestive themes are allowed, with a content label. **Built 2026-10-20:** makers tick labels on the store-page form (romance, suggestive themes, violence, horror, strong language, gambling themes); they show on the app page.

**Categories and tags** have two levels:
- **Broad categories:** Games, Kids & Family, Tools, Creative, Utilities, Learning, Play style, Vibe.
- **Specific tags** sit under each category (Roguelike, Note-taking, Pixel Art, Touch Controls…). **Expanded 2026-10-26** (migration 0089) to a store-sized set of about a hundred (Action, RPG, Tower Defense, Deckbuilder, Point & Click…; Office, AI Assistant; Drawing, Game Making; Security, Media Player; Coding, Science; Split-screen, Turn-based; Wholesome, Spooky…). The editor's tag picker groups them by category, with a search, and closes the groups that rarely fit the app's type.
- **Kids & Family** (Kids, moved from Learning, Family, All Ages, Educational) sits right after Games, and on the store bar. It lists only apps rated **7+ or under** without suggestive themes, horror, strong language or gambling themes (the category's `max_age` and `excluded_labels`), on its page and its tags' pages, for every viewer. Makers can still tag an older app; the editor warns as they type and `get_fixes` recommends fixing the rating or the tags, but saving is never refused. Party games are a Games tag, not Kids & Family, since many are for adults.
- Every app has several tags. Every category and tag has its own browsable page.

**Search and filters:** filter by tag, % Slaps, runs on my platform (including community builds), and released/upcoming. Sort by % Slaps, popular this month, most reviewed, or recently updated. **Built 2026-10-20:** all of these on every list page ("this computer" marks your platform; a community build counts only while it's in the store).

## 6. Library

- **Add to library** is free and instant. SLAPorSLOP then **asks whether to install now**. An app can sit in the library uninstalled.
- The install prompt lists every build for your platform: the official one first, then community builds with caveats (see §9).
- **Interested** upcoming apps appear in the library before they have a build, and users get notified when they enter beta or release.
- **Private** apps you own or that are shared with you have their own section.
- For each installed app the library shows:
  - Play/Launch.
  - Community builds (built 2026-10-21): the official app and its community builds install side by side, each with its own save folder. A community build sits under its installed original, and the original's **Play** button has a list of the installed builds to start instead (the pick is kept per original; the build's signed manifest names its original, `build_of`). Picking a build whose save folder is empty offers to copy the saves across.
  - Queued updates, with their scan progress.
  - **Save sync** (built 2026-10-21, `save_sync.rs` in the API and the client): an app's save folder follows the player between their computers, **end-to-end encrypted** with a key from their chat backup key (so it needs chat set up on that computer, and their recovery code restores it); SLAPorSLOP stores ciphertext it can't read. Fetched before a launch when another computer saved and this one hasn't changed since; sent after the app closes. When both changed, nothing is overwritten and the player picks which to keep (the other ends up under Saves). 20 MB per app, 200 MB per player, newest two kept; on by default where available, off with one switch.
  - **Shared runtimes** (built 2026-10-21, `crates/slop-manifest/src/runtimes.rs`): a release can name Windows runtimes it needs from SLAPorSLOP's approved list (Visual C++ Redistributable x64/x86, .NET Desktop Runtime 8/9, DirectX 9 extras; `GET /v1/runtimes`). They're signed into its manifest; an update that leaves them out keeps the last approved release's. Before **Play**, the app checks which are missing and, once the player agrees, downloads each from the vendor's own link and runs it only if Windows says it's validly signed by that vendor (Authenticode). Its installer asks for admin rights itself. Not yet tried on a real PC.
  - Uninstall (keeps the app in the library, and its saves unless the player ticks "Delete my saves too") and Remove.
  - **Sandbox mode** (built 2026-10-21, `SECURITY.md` §6.2): a switch per app, suggested the first time a player starts an app that goes online, and "Run every app in sandbox mode". Saves live in SLAPorSLOP's save folder for the app (`SLOP_SAVE_DIR`, `library/saves/<address>`), the same in both modes, so switching is seamless. Every release's source check reports where it saves (`save_folder`: SLOP_SAVE_DIR, Godot's `user://`, elsewhere, or none found); saving elsewhere is a note for the maker, and while the newest release does, the store page doesn't say "works in sandbox mode" even if the maker ticked it; SLAPorSLOP copies them before each sandboxed session, and **Saves** lets the player go back to a copy. If an app closes with an error right after starting sandboxed, the player can run it normally once, turn the sandbox off for it, or tell SLAPorSLOP. Linux (bubblewrap) and Windows (AppContainer, built 2026-10-21; not yet tried on a real PC).

**Built 2026-10-20:** the desktop app checks installed apps for updates at start and every 30 minutes (`GET /v1/library/updates`). "Update apps automatically" (on by default) installs approved updates one at a time, never while the app is running, and never for apps with their own installer. Each row shows an update that's ready, or a newer version still being checked ("downloading and scanning" or "waiting for a person"). The library's Community section lists the official app and its community builds for this platform that aren't installed; installing one keeps the current one, can copy the saves, and can make it what the original's Play button starts.

**Laid out like Steam's (built 2026-10-25, `web/src/pages/Library.tsx`):** a sidebar lists every app once, grouped with counts (Installed, Not installed or In your library, Private, Interested; "(1/23)" while a search or filter hides some), with a search, a filter (All, Installed, Updates, Private, Interested, Community builds) and a short status in the accent colour (Update queued or ready, Downloading 40%, Scanning update, Update in review, Running); arrow keys move through it. The selected app is at `/library/<app>`: a banner (its first store picture, else its tile, else a gradient), one big button (Play with the build picker, Install, Update with Play beside it, Run installer, Running; on the website "Get the SLAPorSLOP app to play" and the library toggle), a settings button (sandbox mode, run normally once, Saves, reinstall, uninstall with "Delete my saves too") and an info button (what it does, versions, download size, licence, source), then tabs: Activity (release notes newest first, the player's own bug reports, an update being checked) with About and **Friends who rated it** (votes friends show on their public profiles; SLAPorSLOP doesn't share friends' libraries or playtime), Community builds & mods, Saves, Bug reports and Reviews. Once a player has played an app and not voted, the library asks "Slap or Slop?" (§7), and an app's page shows its rating beside the title with Rate it, or why the viewer can't yet. The library's settings (gear by its title) hold automatic updates, sandbox mode for every app and save sync; Downloads at the foot of the sidebar ("Downloads – 2 of 3 complete") lists what's downloading, queued and done this session, with Update now. On a phone the list and the app are two screens.

## 7. Ratings: Slap or Slop

- **Two buttons only:** 👍 **Slaps** and 👎 **Slop**, plus an optional written review.
- **Only users who have launched the app through SLAPorSLOP** can vote. That's the same launch that counts them in Slop Pulse. One vote per account, changeable at any time.
- **Weighted by playtime** (decided): votes from people who played longer count for more. Store pages show **both** the weighted and unweighted % (`METRICS.md` §2).
- Votes and reviews are **public on profiles by default**. Users can make them private.
- **Built 2026-10-20** (`crates/slop-api/src/reviews.rs`): an optional written review under your vote (shown only while you have a vote; edit or delete any time), a Reviews list on app pages with 👍/👎 filters and a Report button on each, and profiles at `/u/{id}` with your apps, People page, and votes and reviews when public. In the UK under-18 view they're private by default (PRIV-6); a private profile's reviews show as "A player". Closing an account deletes its written reviews (its votes stay, anonymised).
- **Maker replies (built 2026-10-21):** an app's maker can reply once under each review (changing it replaces it), shown with the review and marked when the review changed after; the reviewer is told. A report on a review covers its reply too (the reviewer can report a reply under their own review), and staff can remove just the reply (`remove_reply`, a decision about the maker), after which it can't be posted again.
- Displayed as **% Slaps**, with a label so the direction is always clear:

| % Slaps | Label |
|---|---|
| 95–100 | Overwhelmingly Slaps |
| 80–94 | Really Slaps |
| 70–79 | Mostly Slaps |
| 40–69 | Mixed |
| 20–39 | Mostly Slop |
| 0–19 | Overwhelmingly Slop |

- **Ratings over time:** a monthly chart of % Slaps shows whether updates and branches are improving the app.
- **Since the new work:** apps that hired creatives through SLAPorSLOP also show % Slaps from votes since the first release with that work (§15.4). Hiring never changes how votes count.
- Voting opens once an app is **playable: in Beta or released** (owner decision 2026-10-25: "Betas are playable and should be rateable"; before that, only at full release). Coming soon pages have nothing to play, so they show Interested counts instead. Every other rule is unchanged (only players who launched it through SLAPorSLOP, one vote per account, weighted by playtime, the abuse checks). **Built 2026-10-25 (migration 0088):** each vote records the app's stage when it was cast; reviews whose vote was cast in Beta say **Rated in Beta**, the rating panel says how many votes were, and the maker's breakdown groups them by stage. A vote changed after release counts as a released one.
- **Rating is obvious at the top of the page (2026-10-25):** beside the rating, a primary **Rate it** whenever the viewer can vote; otherwise a clear panel with why and the step to take right there: signed out, **Sign in to rate**; not played yet, the Play/Install button in the SLAPorSLOP app or **Get the SLAPorSLOP app** on the website; the maker, "Makers can't rate their own app"; Coming soon, "Ratings open when it's playable" with the Interested button.

## 8. Publishing

**Requirements for every listing:**
1. **Open source**, in a public repo (GitHub, GitLab, Bitbucket or self-hosted) under an OSI-approved license. A private app may use a private repo (§13).
   - **Only the repository's owner can put it on SLAPorSLOP** (built 2026-10-20, `repo_owner.rs`). Collaborators can't, even with admin rights. One app per repository; anyone else builds on it as a branch from their own fork, which must be theirs too. Checked whenever a repository is linked or changed (apps linked before this weren't re-checked). How it's proved:
     - **GitHub:** a GitHub account linked to the SLAPorSLOP account owns it, or is an owner of the organization that does (through SLAPorSLOP's GitHub App on the organization).
     - **GitLab.com:** a linked GitLab account owns the project, or is an Owner of its group.
     - **Bitbucket Cloud:** a linked Bitbucket account owns it.
     - **Anything else** (self-hosted GitLab, Gitea, Forgejo…, and GitLab groups or Bitbucket workspaces SLAPorSLOP can't ask about): a code SLAPorSLOP gives the publisher, added to the README (or a `slop-verify.txt` at the top) on the default branch. SLAPorSLOP reads it with Git, so any public https host works. This proves the publisher can push to it, the most such hosts let anyone check.
2. **At least one build** for at least one platform, on the **maker's own release page**. Normally it's made by **Slop Build**: the maker just verifies their account, links the repo, and accepts one ready-made pipeline file (`ARCHITECTURE.md` §6.2). The build is signed in the pipeline. App files are preferred, but an installer or APK is allowed. Coming soon pages are the only exception.
3. **Passes every scan** (§10).
4. **Games:** **Slop Pulse** (§11) is recommended, not required (decided 2026-10-18).
5. **Publisher Agreement** (`LEGAL.md` §11): owns the rights (including AI output), no loot boxes, no pay-to-win, no scams, no hidden telemetry, nothing unlocked by donations.

**Tracked branch:** the publisher picks one branch as the official version. Each push to it runs Slop Build and **queues an update**. The update is scanned, signed by SLAPorSLOP, and then delivered to everyone who installed from that branch, all at once (no staged rollout).

**Release stages:**

| Stage | Needs | Time limit |
|---|---|---|
| **Coming soon** | Source + every scan (games: Pulse SDK found in source). No build. | 12 months, then must move to Beta |
| **Beta** | At least one build + every scan (games: live Pulse handshake) | 12 months, then must fully release |
| **Released** | Same as Beta | — |

- **Time limits (2026-10-25):** Coming soon went from 30 days to a year, the same as Beta, while we decide what to do about the two stages. Both are configurable (`SLOP_COMING_SOON_DAYS`, `SLOP_BETA_DAYS`).
- **Dead project:** a Coming soon or Beta project with **no commits for 90 days** is labelled dead (decided; **only** for Coming soon and Beta, never Released; the value is configurable). It stays visible but is never recommended, and it comes back with the next commit.
- **Missed deadline:** the page is **delisted** (hidden from the store, search and recommendations) until it moves to the next stage (decided). Existing installs and libraries keep working.
- People can mark Coming soon and Beta apps **Interested**, which puts them in their library and gives publishers a demand signal. (Built 2026-10-20: the count shows on tiles and pages, "Most anticipated" sorts by it, Interested tags count double in "For you", and everyone Interested is notified when the app reaches beta or release.)

### Notifications (built 2026-10-20)
- A 🔔 in the top bar with an unread count, and a notifications page. Kinds: **People jobs** (every step by the other side, and new messages, merged while unread), **Interested** apps moving on, and **moderation** (decisions, appeal answers, report outcomes).
- The page is an inbox (2026-10-25): the kinds down the side with their unread counts, rows grouped by day (Today, Yesterday, This week, Earlier), Unread only, Mark all as read for the kind shown, and each one can be marked unread again or dismissed (deleted). Older ones load 30 at a time.
- Each kind can also be **emailed**, on by default and switched off per kind on the account page. Product emails only, never marketing (TERMS-3). Signed-out reporters get their report's outcome by email.
- Emails go out only when the server has SMTP set (`deploy/README.md`).

### Makers' and creatives' AI agents (built 2026-10-21)

Makers and creatives can hand their work to their own AI coding agent (`maker_agents.rs`, `fixes.rs`, `offers.rs`; guide: publishing.md "Working with your AI agent"). A token from Account → AI agents (look or publish; some apps or all; People jobs or not; a day up to 12 months, decided 2026-10-25) works over the API and as an MCP server with tools for store pages, releases with notes, SLAPorSLOP's checks and a fix for every problem found, community builds, mods, offering fixes to an original (a pull request the agent opens itself, which the original's maker accepts or declines), and People requests, quotes, changes and job conversations. **SLAPorSLOP runs no AI for this:** fixes are fixed text built from the check results, and the agent does the writing and fixing on the maker's side. Paying, accepting agreements, offers or changes, payouts and account settings stay with the person. Friends' chat is end-to-end encrypted and isn't available to agents.

**Setting a token up** (2026-10-25): people pasted the old ready-made `claude mcp add … Bearer slopmk_…` line into an AI chat, which then held the secret. The page no longer shows a command with the token in it. After making one it says plainly never to paste it into a chat with any AI, shows the token once with a Copy button, and walks through the setup for Claude Code, Codex, Grok Build and other tools on Windows, macOS and Linux (`AgentSetup.tsx`, `agentSetup.ts`): open a terminal (explained per system), save the token once in the `SLOP_TOKEN` environment variable in that terminal (the only step with the token), then connect the tool with a reference to the variable (`${SLOP_TOKEN}` in Claude Code's and Grok's MCP headers, Codex's `bearer_token_env_var`), so the token never appears in a chat, a command the AI sees, or the tool's settings. **Expiry:** each token shows how long it has left (highlighted within 14 days, and when expired), the account's AI agents section gets a mark within 7 days, and the owner is notified a week and a day before and on expiry (category `agents`, its own email switch; `maker_agents::warn_expiring`, hourly, each warning once, none for a token made for a week or less or already replaced). **Make a replacement** fills in a new token with the same name, scope, apps, People access and lifetime; the old one keeps working until its own expiry or until removed. The agent sees its own token's expiry in `whats_new` (`your_token`) and at the end of `maker_guide`, and is told to pass it on within a week.

### Handoff links from an AI assistant (built 2026-10-27)

Any AI assistant, connected to SLAPorSLOP or not, can prepare a new app's store page or a People page and give the person one link: `/publish/new#fill=<fields>` or `/people/me#fill=<fields>`, the fields as URL-encoded JSON (`encodeURIComponent(JSON.stringify(fields))`) named as the API names them (`NewApp`, `ProfileInput` plus `portfolio_repo`; guides: publishing.md and portfolio.md). The fill rides in the fragment, so it never reaches the server or its logs. The form opens filled in, with a notice of what was filled in, what was left out and why, and what to check most; the person changes what they like and presses Create themselves. Nothing is sent before that (`web/src/handoff.ts`, `components/Handoff.tsx`).
- **Untrusted:** only the form's own fields, of the right type and within the form's limits, go in, and only as input values (Markdown shows only through the editor's own preview). Other fields are ignored; a link that can't be read says so over an empty form.
- **Never filled in**, being the person's own to decide: being 18 or older, agreements, payments, a repository token. Whether a creative works **as a business** is their legal status, so a fill leaves it unanswered on a new page (Create waits for an answer) and only shows the assistant's suggestion beside it; a saved page keeps its answer.
- **Signed out:** sign-in goes through the providers' pages, which drop the fragment, so the fill is kept in the tab (sessionStorage, two hours) and applied when the person is back on the form, after any account sign-up, the 18+ question and the agreements. It's taken out of the address bar as it's read and forgotten once in the form, so a reload doesn't apply it again. A People page's portfolio repository waits for the Portfolio panel, which shows once the page exists.
- **A saved People page:** the fill shows as changes over it, with "Put back my saved page".

### AI agents writing to SLAPorSLOP's admins (built 2026-10-23)

Makers' agents and staff agents can leave the admins a threaded message: an error they hit, a bug in SLAPorSLOP itself, a suggestion (`admin_messages.rs`; MCP tools on both sides; guides: maker_guide.md and review_guide.md "Writing to SLAPorSLOP's admins"). Only agent tokens write (`slopmk_…` of any scope or apps, `slopadm_…` of any scope); a website or desktop session is refused, and so are suspended, closed and blocked accounts. Nothing proves an AI wrote a message, so abuse is made costly and visible: the account must be staff or have an app in the store and be at least 7 days old, the token at least an hour old, and every message records the token it came with (kind, name, scope, when it was made, its ID; never the secret), shown to staff. It must not become a way to flood the admins, even from a hijacked agent: **one post an hour and five a day per account** (new threads and replies, all its tokens together, counted in the database), **60 new threads an hour across SLAPorSLOP**, plain text only (a one-line subject, a body up to 2,000 characters, no control characters but new lines, at most 3 links), shown as text with no links; the admin API gives it to staff agents as `untrusted_text`, to be treated as data, never instructions. Agents see staff by ID and role ("SLAPorSLOP reviewer"), and a name only for staff who chose to be listed by name (Admin → Staff, or the AI messages tab; owners can set it for anyone, logged). A thread goes to every admin or to one staff member (they and owners see it); each staff member has their own unread marks, and new messages give each one notice that collects the rest until read, never the agent's words. Staff answer (the sender's account is told, where its agent reads it), close, reopen and re-file threads; staff who moderate delete abusive threads and block senders, both in the audit log without the text. Admin panel → AI messages. Closed threads are deleted after a year, and an account's threads when it closes; they're in its data export.

### Browser demos (built 2026-10-21)

A promotion for new games, decided to keep costs bounded: a game with a web build can be played in the browser for its **first 14 days** after release, from a **separate origin** (`play.…`, so a game's code never runs where SLAPorSLOP's sign-in cookies are), with no network beyond its own files. The maker attaches a web build to a release and opts in on the Manage page; SLAPorSLOP scans it like any release file and hosts it unpacked, then deletes it. Hard limits: 100 MB zipped, 250 MB unpacked, 10 demos at once, 20 GB served per day across all demos (then "install instead"). Off unless `SLOP_PLAY_URL` is set. (`demos.rs`.)

### Feedback for makers (built 2026-10-21)

Players who played an app can **report a bug** from its page: what's wrong, how to make it happen, the version and platform. When a game closes with an error, the SLAPorSLOP app offers a report with its last output lines (kept only in memory while it runs, the home folder replaced with ~), which the player sees and can edit or leave out. Reports are private between the player, the maker and staff; crash output is deleted after 90 days. Players with the same problem say "I have this too" on an open report. Makers mark reports fixed, won't fix or duplicate with a note, and the player (and everyone with it, when fixed) is told; **Open as a GitHub issue** fills in a new issue for the maker, and their AI agent can file and update reports. Makers can turn reports off. **How each release landed** (Manage page): % Slaps from votes cast on each version, the reviews written while it was newest with their most repeated phrases (counted, no AI), and its bug reports. (`bug_reports.rs`, `slop-client` `output.rs`.)

## 9. Branches & community builds

- Anyone can fork the repo and **register a branch**: mods, fixes, translations, ports.
- Branches with builds are scanned like any other upload and can be installed directly.
- **Filling platform gaps:** if the publisher ships only Windows and a branch adds a Linux build, Linux users see it as an option. It's clearly labelled **Community** and comes with these caveats:
  - Made by @author, not the publisher.
  - Passed the same security scan.
  - May lag behind official updates.
  - Links to the branch's README, and requires an "I understand" checkbox before installing.
- Community builds of games must include Slop Pulse too.
- **Built 2026-10-20** (`crates/slop-api/src/branches.rs`): a branch is an app of its own (the author's fork, releases reviewed and signed like any app, its own votes, reviews and Donate button) linked to the original. "Register a branch" on an app's page (mod, fix, translation, port or other; your own fork, not the original repository). It isn't listed in the store on its own: the original's page lists its Community builds, ones that add a platform first ("adds Linux"), and there's a Community builds shelf and page. A branch's page says whose it is, and installing needs the caveats ticked. **Same rules as a new app (2026-10-21):** registering a branch or mod asks what creating an app asks (description, age rating, content labels, online, works in sandbox mode), on a page built around the original, and counts toward the new-app limit; it starts from the original's answers, and can't be rated below the original or drop its content labels (also enforced when editing). Its releases go through the same scans, source check and review. Built since: community builds of games whose original counts players with Pulse can't be listed before their own first Pulse handshake (the branch's Manage page says so), and "runs on my platform" in search includes community builds.

- **Mods (built 2026-10-21, migration 0046):** add-ons a game loads, published by anyone for any app (decided 2026-10-21: no opt-in by the game's maker, who can report a mod but not hide it). A mod is a branch with the kind `addon`, from the author's own repository, checked, reviewed and signed like any app. Its releases are one `.zip` for every platform (`any`), with no program to start. The original's page has one **Community** area: its mods, then its community builds. Mods aren't in the store, the Community shelf, player counts or the list of versions to run. Apps may only load mods approved on SLAPorSLOP (Content Policy, "Code from outside SLAPorSLOP"). One-off donations to mods and community builds give the original's maker 30% as a separate payment first (PAYMENTS §5.3). **In the SLAPorSLOP app (built 2026-10-21):** a mod's signed manifest names its game (`mod_for`), so the app adds it only to that game, once installed, into `mods/<slug>/`; nothing in a mod is started. At launch the game gets its mods' folders in `SLOP_MODS` (like `PATH`), minus any SLAPorSLOP has pulled. In the library, each installed game has one **Community** section: its other builds to switch to, and its mods as tick boxes (added ones first, pulled ones marked). A mod's page has **Add to <game>** in the app. Uninstalling a game removes its mods. **Built 2026-10-21 (migration 0049):** mods may contain code (decided 2026-10-21): the source check marks a mod with code (`mod_code`, with its languages) for a full read, and one with networking code (`mod_online`) as "goes online", which is still approvable but shown to players and makes sandbox mode suggested for the game it's added to. A maker can name the folder the game already reads mods from (`mods_folder`: in its files or its save folder, each mod in its own folder or all files flat), signed into its manifests; before each start and after mods are added, the SLAPorSLOP app makes that folder hold exactly the approved mods, moves anything else to quarantine and tells the player that mods must be published on SLAPorSLOP; in sandbox mode it's read-only. **Slop Link:** a running game gets `SLOP_LINK` (a folder per run: it writes `requests/`, SLAPorSLOP writes `answers/`, so it works the same in sandbox mode and offline) and can ask for mods by SLAPorSLOP address and version; the player confirms each request (decided 2026-10-21: every time), and the SLAPorSLOP app adds only approved mods for that game, then tells the game how each went and where its mods are.

## 10. Security & compliance pipeline

Runs on every upload (official, community, private and upcoming), on every update, and again on any credible report.

| Check | What it does |
|---|---|
| Malware scan | Multi-engine scan of every binary, installer and archive |
| Behavior sandbox | Runs each build in a VM and watches file, network and process activity |
| Build matches source | Checks the maker's signature, that the commit exists, and the CI build attestation ("Built from source" badge) |
| Dependency audit | Known vulnerabilities and hijacked packages |
| Secrets & telemetry | Leaked keys, hidden trackers, undisclosed network endpoints |
| License compliance | Matches code and assets against known projects |
| Illegal content screening | Text, images and audio |
| Monetization rules | Loot boxes, pay-to-win, fake currencies, scam patterns |
| Slop Pulse *(public games)* | SDK present; heartbeat carries nothing but app id + token |

**Built so far (2026-10-20):**

- **Malware scan:** every release file, unpacked, with ClamAV, YARA (YARA-Forge core rules) and capa (what each program can do), in a locked-down container with no network (`deploy/scanner`). A ClamAV or YARA finding puts the file **on hold**; notable capa capabilities (keylogging, injection, persistence…) are a **note** for the reviewer. The scanner never decides "blocked": that's a person's call.
- **Microsoft Defender:** Windows builds are checked with Defender and the day's definitions on GitHub's Windows runner, in the maker's own repository: Slop Build does it on every build, and **Slop Check** (`slop-build/.github/workflows/check.yml`) does it for makers who build elsewhere. The result reaches SLAPorSLOP with the run's GitHub OIDC token, which proves which repository and workflow sent it. With scanning on, a Windows file needs a clean Defender check before approval.
- **Signing:** Slop Build signs the Windows program with the maker's free SignPath Foundation certificate when they have one, and every check records who signed it.
- **Dependency audit and secrets:** Slop Build runs OSV-Scanner and gitleaks on the source. Only the maker and reviewers see the results, and they never stop a release. Every day after that, the dependency watch checks each live release's packages against OSV again and tells the maker about new advisories (SECURITY §5.6).
- **Source check (licence and monetization):** for every release from a GitHub, GitLab or Codeberg repository (or another GitLab or Gitea server listed in `SLOP_SOURCE_HOSTS`, or one the maker saved a read-only token for), SLAPorSLOP reads the source at the release commit once (`source_scan.rs`). It checks that the licence files match the store page's licence, that source files' SPDX headers don't name another, and that a public app's licence is OSI-approved; it looks for payment libraries, in-app purchase APIs (Steam, Unity, Google Play, Apple), checkout links, loot boxes and premium currencies; and it looks for code from outside SLAPorSLOP (Content Policy): loading assemblies, classes, native libraries or scripts at run time, resource packs and asset bundles, self-updaters, and data formats that can rebuild any object, the usual way a hostile game server takes over a player's computer. These are **notes** for the reviewer and the maker, since patterns in text have false positives; a person rejects a release that really sells something. An update whose check finds a note the last approved release didn't have waits for a person instead of being approved automatically. Not checked yet: Bitbucket and other hosts, code copied from other projects, and pay-to-win design.
- **False positives:** a reviewer can clear a scan finding or a Defender detection with a reason (10+ characters, audit-logged, shown to the maker).
- **VirusTotal (optional):** makers and reviewers get a "Look up on VirusTotal" link for each file's hash, a second opinion only. SLAPorSLOP sends nothing there and never requires it (privacy for private apps; the free tier bars commercial use).
- **Public report:** the app page says what the newest approved release was scanned with, Defender's result and who signed it. Never "safe".

Why not upload to Microsoft, VirusTotal and the like for makers: Microsoft has no submission API (false positives are submitted by the developer, by hand), and the free tiers of VirusTotal, MetaDefender and Hybrid Analysis don't allow commercial use. Players aren't shown SmartScreen's "unrecognized app" prompt in the first place, because the SLAPorSLOP app adds no Mark-of-the-Web to what it downloads.

Results are **passed**, **note**, **on hold** (fixable, e.g. a license mismatch or a leaked key), **rejected** (a policy violation, e.g. a loot box) or **blocked** (malicious; the account is flagged for human review). Every app has a public security report.

## 11. Slop Pulse (player count)

- **Purpose:** show "N playing now" and a monthly peak for games. Nothing else.
- **How it works:**
  1. When the SLAPorSLOP client launches a game, it passes a **one-time signed launch token**.
  2. A tiny open-source library in the game sends `start` and then a heartbeat every few minutes, carrying only the app id and that token.
- **Anti-botting:**
  - Tokens are only issued for real launches by signed-in installs, and are single-use per session.
  - Each account counts once per game.
  - Heartbeats must chain from a valid token and are rate-limited.
  - Spikes from new accounts are flagged.
  - Copies launched outside SLAPorSLOP simply aren't counted.
- **Optional** (decided 2026-10-18; it was planned as required for public games). Recommended for games: it powers "playing now", the monthly peak and playtime-weighted votes. The publishing guide (`web/public/publishing.md`) has the protocol and code for common languages.
- **Privacy:** no device IDs or gameplay data. Extra payload fails the scan.

## 12. Money

Details are in `PAYMENTS.md`, and the legal reasoning is in `LEGAL.md` §3–4.
- **Nothing costs money to download.**
- **SLAPorSLOP never holds money.** Every payment goes straight into the maker's or creative's own Stripe account. SLAPorSLOP receives only its 5% platform fee.
- **Donate button:** presets around the publisher's suggested amount, from $1 to $500. **The donor pays the card fees**, and the total is shown before paying. **Donors are never listed publicly,** and get nothing but a thank-you.
- **Donations unlock nothing,** and apps can't tell who donated. That's what keeps a donation from being a purchase.
- **Server donation:** a monthly recurring donation, only for apps that declare they run servers. It never controls access to those servers. Cancel any time.
- **Built 2026-10-20** (`PAYMENTS.md` §10): Donate on app pages, server donations, the maker's settings, totals and refunds on the Manage page, and donation history, cancelling and fee statements on the account page.
- **Always visible (owner decision 2026-10-25):** every public Beta or released app's page shows Donate to everyone (its details load signed out). Signed out, it has **Sign in to donate**. When donations are off it says why (off on this server, paused, not turned on, Stripe not ready), and the app's maker gets the fix right there: Stripe's sign-up, or **Turn on donations**. The maker's Edit store page has a **Get paid** section at the top with the same settings as Manage, and every step to set it up.
- **Next to Play (owner decision 2026-10-25):** Donate is a normal-size button in the same row as Play / Install / Add to library, on the store page's Get it box and in the library's action bar, and opens the donate form right there. It's the same green as Play while the viewer can donate; quiet, with "Donations off" beside it and the reason on hover or focus, when they can't (its panel has the reason, and the maker's fix). Once the viewer has donated it's grey, **♥ Donated**, and its panel says what and when, with **Donate again**. A running server donation shows as **♥ $5.46/month** in grey with **Cancel** beside it: one "are you sure?" that says it ends now and no more payments are taken, then it's cancelled at once (PAY-15). Only the viewer sees their own; the app looks the same to everyone else (PAY-5).
- **Where the money goes:** **one** Stripe account per app, picked by the publisher. If several people made it, the publisher splits the money with them outside SLAPorSLOP. Community branches have their own Donate button.
- **SLAPorSLOP's fee:** a flat **5%** of each donation, charged to the maker.
- **No in-app purchases** (decided). Apps can't sell anything: no items, supporter packs, subscriptions, loot boxes or premium currencies. The monetization scan rejects them.
- **Anti-scam detection:** fake giveaways, phishing, credential prompts, undisclosed ads, crypto miners.

## 13. Private apps

- Publish something **only you can see**. It never appears in the store, search, recommendations or official team pages.
- **Same requirements** as public apps (at least one build, every scan) **except Slop Pulse**, and the source can be in a **private repo** that SLAPorSLOP is allowed to read (`SECURITY.md` §12). Builds stay on the owner's private release page. People it's shared with download through short-lived links, and SLAPorSLOP never stores the software. No store page, no donations, no voting.
- **Sharing:** with specific friends and/or **player teams** you belong to. **Never with or to official teams.**
- **Built 2026-10-20** (`crates/slop-api/src/private.rs`): "Who can see it?" when publishing; the owner shares by pasting someone's profile link (there are no friends lists yet) and can stop sharing; people see it once it has an approved release, in a Private section of their library, and are notified. Every request checks access on the server (`catalog::push_shared_with`); private apps can't be listed, and have no voting, reviews, donations, Interested or branches. Sharing with player teams comes with teams (§14). Also built: **making it public** (the owner asks once the repository is public; a reviewer lists it after SLAPorSLOP checks again, and the shares end since everyone can see it) and **the 14-day freeze** when SLAPorSLOP can't read the repository (`frozen.rs`, for every app: no new releases, the owner is told, then out of the store or no longer shared after 14 days; back when access returns, with "Check now" on the Manage page). **Private repositories outside GitHub (built 2026-10-20, `repo_tokens.rs`):** the maker gives a read-only token (GitLab project access token, Bitbucket repository access token, Gitea/Forgejo token) when publishing or on the Manage page. SLAPorSLOP checks it reads the repository, keeps it encrypted, proves ownership with it (linked GitLab/Bitbucket accounts, or the README code read through the host's API), fetches and checks releases with it, and runs the freeze check with it. People it's shared with get the host's own short-lived download link; a host that serves private files directly can't be shared through SLAPorSLOP, since SLAPorSLOP never relays files. Untested against the real hosts so far.

## 14. Teams

- **Player teams:** anyone can create or join (open or invite-only). Members share picks and game nights, and can see private apps shared with the team.
- **Official teams:** verified developer studios. They show the apps they publish. They can't receive private apps.
- **How a studio is verified (proposed):** through its **GitHub organization** (or the GitLab equivalent). An official team needs all of the following:
  1. A GitHub organization that the person setting it up is an **owner** of. Checked through GitHub sign-in.
  2. The org's own **verified domain** on GitHub (GitHub's "Verified" badge on the org). Or a DNS TXT record SLAPorSLOP asks them to add to the studio's website domain.
  3. **Two-factor authentication required** for every org member (a GitHub org setting SLAPorSLOP can check).
  4. The team's apps are published from repos **inside that org**, and their builds are signed by that org's CI (`SECURITY.md` §5.3).
  5. *(If they take donations)* the studio's Stripe account is verified as a **business** by Stripe.

  Verification is re-checked regularly. If any of these stop being true, the badge is removed.
- **Built 2026-10-20** (`crates/slop-api/src/teams.rs`, `/teams`):
  - **Player teams:** open or invite-only (by profile link); owners, admins and members; picks with a note; game nights that notify the other members; private apps shared with the team. The last owner hands over before leaving; the last member out ends the team.
  - **Official teams:** checked through SLAPorSLOP's GitHub App installed on the organization (owner role of the team's longest-standing owner, GitHub's verified domain or a `_slop-team.<domain>` TXT record, the organization's 2FA requirement, apps only from the organization's repositories, and Stripe business type when they take donations). "Check now" and a daily re-check; the owners are told when the badge goes. Verified teams are listed, show their apps, and their apps carry an "✓ Official" link to the team. Official teams can't receive Private or have picks.
  - Teams can be reported and hidden by a moderation decision. Not yet: GitLab organizations, and checking that builds are signed by the organization's own CI.

## 15. People (creative marketplace)

**Decided 2026-10-19.** People is a **separate section** from the store. Artists, composers, sound designers, writers, voice actors, testers and studios offer services to makers. It's the one place on SLAPorSLOP where something is **sold**: a creative's work to a maker (`LEGAL.md` §4). The aim is that hiring a real person through SLAPorSLOP is **easier than any other way**: one offer, one tap to pay, credit on the app page when it's done.

### 15.1 Profiles (different from app pages)
A creative's page is built to sell a service, so it isn't an app page with a different header:
- **Header:** name, "Individual" or "Studio · N people", headline, location, skills, "Taking work" or "Fully booked", typical turnaround, and the **AI-use disclosure** (required: No AI / AI-assisted / Mostly AI-generated, with an optional note; each has its own neutral colour, blue, teal and green, because no way of working is shunned, decided 2026-10-25). Whether they're a **business (trader)** is shown too (`LEGAL-GUIDANCE.md` HUM-1).
- **Rating:** % Slaps from **verified clients only** (a completed job through SLAPorSLOP), shown from 3 reviews, with the count.
- **Portfolio:** pictures, videos (YouTube) and **audio tracks** for composers. Like app pictures, SLAPorSLOP doesn't host them: they live in a public GitHub repository, listed in `.slop/portfolio.json`, read at one commit, checked, and **reviewed before they show** (§15.8). A first profile goes live when its first portfolio is approved. It's explained as the creative's shop window, with a guide for people and their AI assistants (`/portfolio.md`) and ready-made templates (`examples/portfolio-templates/`); "Check my portfolio now" lists every problem at once, each naming the work and field.
- **Services:** up to 8 packages (name, what's included, starting price, typical days, skill). "Hire" on a package starts a request with it filled in.
- **From an AI assistant:** the editor can open filled in from a handoff link (`/people/me#fill=…`, §8 "Handoff links"); the creative checks it, answers the business question and the 18+ one themselves, and presses Create.
- **Verified reviews:** each one says ✓ Verified client, the project and what was made, and the client's 👍/👎 with text. The creative can reply once, publicly.
- **Worked on:** the apps that credit them (§15.4).
- **Customisation:** an accent colour (from a palette that keeps contrast readable), a banner and avatar from their portfolio, the **order of the sections** (portfolio, services, reviews, worked on, about), the portfolio order, an About section (Markdown, through the safe renderer), and up to 5 links (website, ArtStation, Bandcamp…), shown with their domain.
- **Directory (`/people`):** the skills (Everyone, 2D Art, Pixel Art…) and the People search are the page's header, like the store's bar. Your jobs, and "Your People page" (with its status until it's live) or "Offer your services", are buttons beside the title. Filters for skill, taking work now, AI use (Any, or exactly one of No AI, AI-assisted, Mostly AI), studios only. Default order is a **daily shuffle**, so nobody is buried. "Best rated" and "Most clients" are sorts the viewer picks (HUM-10).
- **The day's shelves (`/v1/people/discover`):** before any skill, filter or search, the front page shows shelves above everyone, like the store's: **Taking work now** (can be hired, shuffled for the day), **Spotlight** (one skill a day, in turn, shuffled), **New on People** (went live in the last 60 days, newest first) and **Studios** (shuffled). None is ordered by ratings or clients: shelves of the most popular or best rated need the lawyer first (HUM-10).

### 15.2 Hiring, start to finish
Every job belongs to **one of the maker's projects**, at any stage (coming soon, beta or released; public or private). That's what lets the work be credited.

**The rule both sides sign up to (decided 2026-10-19): the creative delivers exactly what the agreement says, nothing more and nothing less.** The maker is warned, in plain words, before sending a request and again before accepting, and ticks a box each time. Anything not written down isn't part of the work. Any change after acceptance is new work (§15.3).

| Step | Who | What they see and do |
|---|---|---|
| 1. Get paid set-up | Creative, once | "Get paid": Stripe's own sign-up, a few minutes. Until it's done, the profile says "Not taking paid work yet". |
| 2. Request or inquiry | Maker | **Hire** (on a package or the profile) → pick the project → write exactly what they need, under the warning. Two buttons: **Send request**, or **Ask about it first**: an *inquiry* asking the creative to review it, give their opinion and a timeframe, and talk it through before any offer. |
| 3. Offer | Creative | Optionally open a conversation first (§15.5). The offer: **exactly what's included, price, payment (half now, half on accepting the delivery by default), due date, optional sign-off, who owns it, open-source use, credit**. Or decline. |
| 4. Accept | Maker | A **one-screen summary in plain words** under a "read every word" warning, the full Service Agreement one tap away, and the exact-wording box again. **Accept & pay $X**, or decline (the creative can send a new offer). |
| 5. Sign-off (optional) | Both | The creative sends the direction (a sketch, a sample). The maker **signs off**; from then on, changing the direction is a Change to Agreement. |
| 6. Deliver | Creative | **Mark delivered** with links. While money is still owed, this is a **preview** (watermarked or reduced). |
| 7. Accept the delivery | Maker | **Accept the delivery — pay the rest** (or just accept, if nothing's owed), or ask for a change, or open a problem. After **7 days** with no answer, the delivery counts as accepted and anything owed stays owed; an open problem holds this off until it closes. |
| 8. Final files | Creative | After the last payment, **send the final files**. The job completes. |
| 9. After | Both | The maker can review the creative. The creative is credited on the app page, and the app appears on their profile. |

- **Cancelling:** before acceptance, either side can decline. After it, the creative can cancel (and refunds from their Stripe dashboard); the maker can back out only before paying anything. EU/UK consumers get the legal **withdrawal button** (HUM-3) for the whole 14 days, even after the job completes (HUM-4); if they asked for work to start straight away, the right ends once the work is delivered. Once a job is declined, cancelled or withdrawn, its conversation stays closed.
- **If the maker doesn't pay the rest:** they never get the final files; until they've paid in full they have **no right to use the work** (copyright stays with the creative, so shipping it is infringement and the creative can have it taken down); the job shows as owed; **they can't hire anyone again until it's paid**; and creatives see a factual payment record on every job ("paid in full on 11 of 11 finished jobs"). Beyond that, the record is the creative's evidence for small claims.
- **If the creative doesn't deliver what was agreed:** the maker doesn't accept the delivery, opens a problem (§15.6), and reviews the creative. Their review is the consequence on the creative's side.

### 15.3 The contract
The **default Service Agreement** (`crates/slop-api/src/people/service_agreement.md`, versioned) is filled in from the offer. Both sides see the same plain summary:
- **What, exactly:** title and an exact scope. "This agreement binds both of you to these words": the creative delivers what's written, nothing more or less; anything unwritten isn't part of it.
- **Price and payment:** up front, **half now and half on accepting the delivery (default)**, or all on accepting the delivery. Delivery before the last payment is a preview; the final files follow it.
- **Due date**, and an optional **sign-off** on the direction.
- **Changes are new work.** There are no free "revisions": after acceptance, any change — added, removed, redone, a different style or direction, a new date — is a **Change to Agreement**: a separate document the creative writes, with its own scope, price (which may be zero) and due date. It applies only when the maker accepts it (ticking the exact-wording box again) and pays its price; until then the agreement stands as written. The creative may decline a requested change. Each accepted change is stored with its own SHA-256, referring to the agreement's.
- **Who owns it:** **"You own it"** (copyright is transferred to the maker on full payment; the creative may show it in their portfolio) or **"Licence for this project"** (the maker may use, change and publish it in the named project, forever; the creative keeps the copyright). Nothing until paid in full.
- **Open source:** whether the work may be published **under the project's open-source licence** (anyone can reuse it), or stays **all rights reserved** inside the repository. Asked plainly, because every public SLAPorSLOP app has public source.
- **Credit:** shown on the project's page (on by default).
- **AI use** for this job, and the creative's promise that it's true and that they have the rights to what they deliver.
- **Refunds:** the creative's policy (default: full refund before work starts; after that, for undelivered parts). Problems and mediation (§15.6).
- **Records and conversations** (§15.5), and the **law**: the creative's country, without removing the maker's consumer rights where they live.

"Accept" is the signature (US E-SIGN, EU eIDAS simple electronic signature). SLAPorSLOP stores the exact text both sides agreed to, its hash, the template version, and who accepted when. **Both sides can download the full record** (agreement, changes, timeline, payments, problems) at any time.

### 15.4 What a project gets: credit, a badge, and "since the new work"
Hiring people **never changes how votes count**. A rating is what players say (`LEGAL.md` §5.5). Instead:
- **Credits** on the app page: "Art by @creative · Music by @composer", linking to their profiles.
- **✋ Made with people badge** on the app page and store tiles, plus a store filter and shelf, once the app has at least one **completed, fully paid** job through SLAPorSLOP of $10 or more. It counts jobs, not money spent, so a small pixel-art job counts the same as a big one.
- **"Since the new work" rating** on the app's ratings panel: % Slaps from votes cast or changed **since the first release after the job completed**, beside the overall rating. Old 👎 votes about AI art no longer hide that the app improved, honestly, because only newer votes are in it.
- These count only for jobs done through SLAPorSLOP, which is the reason to stay: creatives can always take work elsewhere.

### 15.5 Conversations: recorded, opened by the creative
- **The creative starts conversations.** The maker can **ask for a conversation at any time** (with a short note), but only the creative opens it, and closes it. This keeps the maker from changing the requirements by message; changes go through §15.3.
- **Recorded, and said so everywhere.** Above every conversation, and in a box each side ticks before their first message: *"This conversation is recorded, so you both have proof of what was said."*
- **The two sides keep it, not SLAPorSLOP (decided 2026-10-19).** SLAPorSLOP holds the text while the conversation is open and for **30 days after it closes**, so both can **download their copy**; then it deletes the text and keeps only a **fingerprint** of each message (a SHA-256 chained to the one before). The fingerprints prove a copy is unchanged and complete, without SLAPorSLOP being able to read it.
- **SLAPorSLOP staff see a conversation only if a side submits their copy** for a problem or a review dispute. SLAPorSLOP checks it against the fingerprints and tells the mediator how many messages match.
- These job conversations are separate from personal chat (§19), which stays end-to-end encrypted.

### 15.6 Problems, mediation and review disputes
- **Problems.** Either side can open one on an accepted job: they state their case, the other answers, and either proposes outcomes (a refund, redoing a part at no charge, cancelling, something else). **Only what both accept happens**, and SLAPorSLOP carries it out where it can (cancelling; refunds are issued by the creative in Stripe). A **SLAPorSLOP mediator** (a person, or an AI reviewer with an admin token) can read the record — every look is logged — and **suggest** an outcome, which binds no one. If they can't agree, the mediator closes it unresolved and both keep the record for a card dispute or a court. **SLAPorSLOP never decides who's right about money** (LEGAL §4.1).
- **Review disputes.** A creative can dispute a review that breaks the rules, isn't about the job, was used as a threat ("more for free or I'll leave a bad review"), or states something the record shows is false. The reviewer can answer. A mediator then **removes it**, **keeps it with a factual note** (shown under the review), or **keeps it**, always with a written reason shown to both. Reviews are **never removed for being negative**: an honest opinion stays, and the creative's public reply is their answer (HUM-12, HUM-18).

### 15.7 Money in one line
The maker pays the creative **directly** (Stripe, the creative's own account). SLAPorSLOP takes **5%** as its platform fee, out of what the creative receives, and never holds the money. Details: `PAYMENTS.md` §7.

### 15.8 Review and safety
- New portfolios and portfolio changes wait for review in the admin panel (Review queue → People pages), like app pictures, and AI reviewers can see and decide them (`review.rs`). The creative is notified either way: their page is live, or why the portfolio was turned down.
- **Where a page stands (2026-10-25, `people::page_status`).** Only portfolios are reviewed; the profile's own text is saved as it is. A page is a **draft** (saved, no portfolio sent yet, or none could be read), **waiting** (its first portfolio waits for review), **turned down** (its newest portfolio was turned down, with the reviewer's reason, and none is approved yet), **live** (an approved portfolio; a newer one may be waiting, or have been turned down while the approved one keeps showing) or **hidden** (a moderation decision, or the account is suspended). Only the creative sees it before it's live. The creative sees where it stands, what happens next and what to do on their page, in the editor, in a "Your People page" card on People and on their account's Profile section; saving in the editor takes them to the page, which says it's saved and where it stands. **Preview (2026-10-25, `PreviewSet`).** While a newer portfolio waits for review (or was turned down), the creative's own page shows it by default (its works, avatar, banner and page design, from the same stored, sanitized set) under a sticky "PREVIEW: only you can see this" bar with where it stands and a **View your live page** link (`?view=live`: exactly what everyone sees, under its own "This is your live page" bar, with the way back), and the previewed page is outlined and labelled "Preview · not your live page". The server sends the newer set only to the creative; everyone else only ever gets the approved one. A reviewer's decision notifies the creative (category `jobs`): "Your People page is live" (or "Your new portfolio was approved"), or "Your portfolio wasn't approved" with the reason and how to send a new one.
- 18+ to sell or hire (confirmed on first use). Rules from `LEGAL-GUIDANCE.md` §4 (trader declaration, withdrawal, independence of creatives) apply.
- **SLAPorSLOP doesn't settle disputes.** It acts on rule-breaking (scams, fraud, repeated non-delivery).

## 16. Reporting & moderation

- Every app, branch, review, People profile and job message has a **Report** button. Reasons depend on what's reported: malware, the build not matching its source, scams, illegal content, child abuse material, intimate images shared without consent, threats, sexual content, copyright, open-source licences, pay-to-win and gambling, misleading pages, harassment, private information, fake reviews, spam, broken builds, other.
- **Anyone can report**, signed in or not (a signed-out reporter gives a name and email, optional for child abuse). Each report needs a good-faith statement and is **acknowledged with a reference number**; signed-in reporters see the outcome on their account page. One open report per person per thing; 20 a day at most.
- **Urgent** reports (child abuse, threats, intimate images) and **serious** ones (malware, scams, illegal content, source mismatch, sexual content, copyright) get a **human review within 24 hours**; the rest within 7 days. Malware and source-mismatch reports about an app **re-scan its live release at once** (built 2026-10-20, `rescan.rs`; every live release is also re-scanned weekly). Intimate-image requests are removed within 48 hours at the latest (TAKE IT DOWN Act).
- **Copyright reports** collect what a DMCA notice needs; the appeal against a copyright decision is the counter-notice. Three upheld copyright decisions end an account (repeat-infringer policy).
- **Decisions:** take an app out of the store, hide a review or profile, warn, or suspend an account. Acting closes every open report about the same thing. Each decision sends the person a **statement of reasons**: what was done, the facts, the Content Policy rule, the law if any, that a person decided it, and how to appeal. Never hide anything for being negative (REV-2).
- **Appeals** within six months, once per decision, on the account page. A person keeps or reverses the decision and says why; reversing undoes it. An app put back on appeal goes in the store only if it passes the usual listing checks: while frozen it goes back when the freeze ends; while pictures wait for review or it has no approved release, it waits under Ready to list for a reviewer. The maker and the reviewer are told why. A hidden bounty comes back for the time it had left (at least 7 days); its backers pledge again (PAYMENTS §5.4).
- **Suspended accounts** can still sign in to read SLAPorSLOP, see the decision, appeal, download their data or close the account; nothing else.
- Child-abuse reports are preserved for a year, and the moderator notes where they were filed (NCMEC, NCA) and police referrals for threats.
- The rules are the **Content Policy** (`/legal/content`); each decision names its section.

## 17. Decisions and open questions

**Decided 2026-10-17:**
0. **Company:** SLAPorSLOP is operated by **IronPoint Development LLC** (US). No in-app purchases. One Stripe account per app. Donors get nothing. Chat is between people and **end-to-end encrypted**. Launch in the **US, EU and UK**. **SLAPorSLOP stores no builds or source code:** builds live on makers' release pages, and every public app has public source.
1. **Missed deadline:** delisted until the project reaches the next stage.
2. **Dead projects:** 90 days without commits, only for Coming soon and Beta.
3. **Private apps:** private repos are allowed (GitHub, GitLab, Bitbucket, self-hosted).
4. **Votes:** weighted by playtime, with weighted and unweighted both shown. Public on profiles by default.
5. **Money:** Stripe, with SLAPorSLOP never holding money. Donors pay card fees. Donors anonymous by default. No wallet or top-ups. No SLAPorSLOP dispute resolution for People, and a 7-day completion window. Monthly **server donations**.
6. **Builds:** made by **Slop Build** on GitHub's free runners and kept on the maker's release page. SLAPorSLOP stores no builds or source. The pipeline signs, then SLAPorSLOP signs after scanning. No staged rollouts. Steam-style installs into a library folder (`ARCHITECTURE.md` §6).
7. **Platform:** Tauri everywhere, one cloud provider, PostgreSQL only, small budget.
8. **Accounts:** 16+ everywhere. UK accounts start in an under-18 view, and an age check unlocks 18+ content. No VPN detection. A 24-hour human review of serious reports.
9. **Company:** IronPoint Development LLC, **Wyoming**.
10. **Slop Build:** GitHub only at launch, for Godot, web/HTML5, Electron/Tauri, Python, Rust and Go. Unity in phase 2, Unreal later.
11. **Frozen apps** (repo or access gone) are delisted after a **14-day** grace period.
12. **Encrypted chat backups are on by default** (users can opt out).
13. **Sexually explicit content is banned.** Non-explicit romance is allowed, with a label.

**Open:**
1. **iOS:** EU-only alternative marketplace, App Store/TestFlight links outside the EU, or no iOS at launch?
2. **Official team verification:** is the GitHub-organization proposal in §14 enough?
4. **Keeping a copy of source code** (`SECURITY.md` §12.2).
6. **Downloads of private apps** through short-lived GitHub links (`ARCHITECTURE.md` §6.5).
7. **Where exactly "explicit" begins:** the wording for the Content Policy (`LEGAL.md` §5.11).

## 18. The mockup and the simulation

- **The real SLAPorSLOP** (website, desktop app and API) runs on the test deployment (`deploy/README.md`). Sign-in, scans, installs, Pulse and reports are real there; People payments are built but wait for a Stripe account.
- **The mockup** (`mockup/`) is the original design reference. All its data is invented and saved only in your browser, and its **Preview as** switch shows Windows, macOS, Linux, iOS and Android. It isn't updated as the real app changes, and its fake data and shortcuts are never copied into real code (`AGENTS.md`).
- **The simulation sandbox** (`docs/SIMULATION.md`) runs the real SLAPorSLOP on its own `_sim` database with invented apps, players, votes and play, to test recommendations, ratings and player counts against a known truth. The browser tests use the same simulation on throwaway databases.

## 19. Chat

A Steam-style communication system: lightweight, not a Discord. **Built 2026-10-20** (`crates/slop-api/src/chat.rs` and `friends.rs` on the server, `crates/slop-client/src/chat/` in the app, `web/src/pages/Chat.tsx` and `Friends.tsx`):
- **Friends:** requests from someone's profile (or by pasting their profile link), accepted under the 🔔; a friends page with requests, blocked people and "who can message you". Works on the website too.
- **Chats:** direct messages, group chats (up to 50 people, named inside the encryption, the starter can remove people) and one chat per invite-only player team (following who's in the team; open teams, which anyone can join, have none, and people in the UK under-18 view are never in one). Unread counts, mute, hide.
- **Messages:** Markdown-lite text (bold, italics, code, quotes, lists, links), replies, emoji reactions, editing and deleting your own, pictures, GIFs, short videos and files up to 15 MB (attach, paste or drop). A link to a picture or GIF (a meme) is fetched by the **sender's** app and sent as an encrypted attachment, shown inline; other links get a preview card made the same way, so recipients' apps never contact the site. Previews can be switched off.
- **Encryption:** MLS through OpenMLS in the app (SECURITY §8.5); SLAPorSLOP orders and relays encrypted messages and deletes them after 30 days. A new device is approved on an existing one by comparing a 9-character code; the approving device sends it the chat history. Encrypted backups are on by default with a 24-character recovery code shown once; "start over" (every device lost) needs a fresh sign-in and changes the person's security code, which every chat shows. Security codes can be compared per person.
- **Safety:** messages only between connections (friends, members of the same invite-only player team, the two sides of a People job that went ahead) unless someone allows messages from anyone; in the UK under-18 view, friends only (tightened 2026-10-22: joining an open team or sending a job request was a way to message anyone, children included; and 2026-10-23: team chats only for invite-only teams, and never with anyone in the under-18 view). Blocks apply to every way of sending (messages, attachments, typing, key updates, being added to a group), team invites respect them, and people added to a group don't see what came before; blocking ends a friendship, stops their messages and hides the chat; every message from someone else has Report (the app sends that one decrypted message with its franking key, checked by SLAPorSLOP; nothing else is revealed), with an option to block too; a warning before opening any link, and on messages asking for payment outside SLAPorSLOP (PayPal, gift cards, crypto…). Away from the app, a 🔔 notice (and email, if wanted) says there are new messages without saying what.
- **Typing indicators** ("Mia is typing…", a few seconds, ended by the message). They go through SLAPorSLOP unencrypted but say nothing except "typing", and are never stored.
- **Notifications:** the desktop app shows a system notification for new messages while SLAPorSLOP isn't in front (one per chat for a burst; muted chats never; the text can be left out), switchable under Account → This computer. **Phones and browsers** get web push for everything under the 🔔 (Account → Notifications): on Android in the browser, on iPhone once SLAPorSLOP is added to the Home Screen. A push carries nothing; the phone fetches the notice from SLAPorSLOP itself.
- **In-game overlay (Shift+Tab):** over a game started from SLAPorSLOP that's fullscreen, borderless fullscreen or maximized, Shift+Tab brings up a small panel with chats and friends over the dimmed game; Shift+Tab, Esc or a click outside returns. Anywhere else Shift+Tab is left alone: the shortcut is only taken while such a game is the window in front. Nothing is injected into the game (so games in exclusive fullscreen briefly switch to the desktop, like Alt+Tab). Windows, and Linux under X11 or XWayland. Can be switched off under Account → This computer.
- **Where it is (2026-10-24):** a **Friends & Chat** bar in the bottom right of every page for signed-in people (`web/src/components/ChatDock.tsx`), like Steam's. It opens a panel docked above it with chats and friends while the page behind stays usable; it stays open from page to page until closed (the bar, ✕ or Esc), and a chat opens inside it, with a way back and a link to the full chat page (`/chat`, `/friends`). On the website the panel has friends and says chat is in the app. On phone-sized screens Chat is a bottom tab instead. Chat isn't a top tab any more.
- **No setup step (2026-10-24):** the first time chat is opened on a computer (panel or page), the app sets the device up by itself, named after the computer (its hostname; "SLAPorSLOP for Windows" and so on if it has none); device names only show in the user's own devices list, and people are always named by their SLAPorSLOP display name. The first device still shows its recovery code once, a later device still waits for approval from an existing one, and a removed device is only set up again when the user clicks "Set up chat again".
- Not planned: voice and GIF search (paste a link instead). Not yet: chat in a phone app.

The first draft, for reference:
- **Friends and direct messages** between users, plus **group chats** for player teams.
- **People job conversations** are different: recorded, opened by the creative, kept by the two sides with fingerprints at SLAPorSLOP (§15.5). They aren't part of this end-to-end encrypted chat.
- **History backups are on by default** (opt-out). They're encrypted with a key only the user holds, so they stay end-to-end (`SECURITY.md` §8.5).
- **Privacy (decided):** messages are between people and **end-to-end encrypted**, so SLAPorSLOP can't read them (`SECURITY.md` §8.5). Link warnings run in the recipient's app. Moderators only see messages that a participant reports. Chat is in the SLAPorSLOP app only, not the website.
- **Safety:**
  - **UK under-18 view:** only connections (friends, team members, People job partners) can message the user, and the "anyone" setting is locked off.
  - Block and report buttons.
  - Warnings on off-platform payment requests and on links to unknown sites.
  - Only users who are friends or share a job or team can message each other, unless a user allows messages from anyone.
- **Open:**
  - How long encrypted attachments are kept.
  - Real-time delivery: settled 2026-10-20. Sync is a long poll that the server wakes the moment a message for the device arrives (an in-process broadcast), so delivery is immediate without WebSockets. With several API instances the wake-up will need Postgres LISTEN/NOTIFY (or similar) across them.
  - Voice chat: probably not.
