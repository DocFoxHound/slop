# Slop — Design Document

**Status:** Concept · clickable mockup in this folder
**Last updated:** 2026-10-17
**Related:** [`ARCHITECTURE.md`](ARCHITECTURE.md) (how it's built) · [`PAYMENTS.md`](PAYMENTS.md) (donations and People payments) · [`SECURITY.md`](SECURITY.md) (security and trust) · [`METRICS.md`](METRICS.md) (donation & vote history) · [`LEGAL.md`](LEGAL.md) (legal framework)

---

## 1. Vision

Slop is a Steam-style store for **apps and games made with AI**, from small utilities to full games. Lots of AI-made software is low effort; Slop doesn't hide that. It makes it **visible, scanned and fixable**:

- Players vote on whether something is **Slop or Not Slop**.
- Everything is **open source**, so anyone can improve it through **branches**.
- Everything is **scanned** before anyone can install it.
- Everything is **free**. Makers are supported by **optional donations** that go straight to them. Slop charges makers a flat 5% platform fee and never holds the money.

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
- **Phone:** iOS and Android. It's the same app, with a phone layout and bottom tabs.
- Publishers can ship builds for any of the five platforms.
- **Send to phone:** on a computer, a user can send a phone app to a linked phone, which installs it next time it's online.
- **iOS constraint:** Apple only allows third-party app marketplaces in the EU. **iOS is undecided** (§17). Android allows direct APK installs.

## 4. Accounts

- Sign in with **Google, Apple, Microsoft, GitHub or Discord**. Slop stores no passwords and only receives a name and email.
- **Accounts are 16+ everywhere.** UK users (detected from IP, billing, app-store and stated country) start in an **under-18 view**: 18+ content is hidden, and only connections can message them. A real age check unlocks 18+ content. Donating and People jobs are 18+ everywhere (`LEGAL.md` §6).
- Browsing works signed out. Anything personal requires an account: library, Interested, votes, donations, reports, publishing, teams.
- **One account = one vote per app = one Slop Pulse count per game.** This is the main defence against botting.
- Users can link devices (computer and phones) to their account.
- **Linked sign-ins** (built 2026-10-20): GitHub, Google, GitLab and Bitbucket (each on once the server has its app keys; Apple, Microsoft and Discord later). Account → Sign-ins links more from a signed-in session after a recent sign-in, and only by signing in to the account being linked, so nobody can link someone else's; never because emails match. Any linked sign-in opens the account; the last one can't be unlinked. Linked Git accounts are also how Slop knows which repositories are yours (§8).
- **Combining accounts:** if a sign-in being linked already has its own Slop account, signing in to it proves it's yours, and after a confirmation page everything moves into the signed-in account (apps, library, playtime, votes and reviews, friends, chats, teams, People jobs and page, donations, notifications, reports and decisions). Where both have the same thing, the signed-in account's stays; the other is closed and points at it. Not combined: two People pages or two donation Stripe accounts (one goes first), and suspended or closed accounts. Chat history stays on the other account's devices, which are removed; its chats continue on this account's devices.
- **Your data and closing your account** (built 2026-10-20). The account page downloads everything linked to the account as one file. **Closing** an account bans that sign-in and freezes what others rely on, instead of erasing it:
  - the sign-in can never be used on Slop again; the email, settings, library and playtime are removed;
  - votes keep counting, re-keyed to a random ID (METRICS §6);
  - a creative's page stays readable, frozen, with a "closed" notice, and takes no new work; job records, agreements and reviews stay;
  - the maker's apps leave the store (installs keep working).

  Open People jobs, unpaid amounts and open problems must be settled first.
- **Terms** (built 2026-10-20). The Terms of Service are accepted at sign-up, the Publisher Agreement before a first app, and the People Terms before a creative profile or a hire. Each acceptance records the version. Changes are published 15 days before they take effect: a notice shows on every page, and once in effect, only the actions behind that agreement wait for it (accepting takes one tick). Every version stays readable at /legal. The texts are drafts until the lawyer has read them (`crates/slop-api/legal/`).

## 5. Store & discovery

**Front page, top to bottom:**
1. Featured carousel
2. **Popular this month**: most added to libraries this calendar month, with live player counts for games
3. **Coming soon & in beta**, with two views:
   - **For you:** upcoming apps that match tags in your library (weight 1) and your Interested list (weight 2), boosted by log(total interest). Each card says why it's shown ("Because you like Cozy & Roguelike"). Dead projects are never recommended.
   - **Most anticipated:** ranked by Interested count.
4. Browse by category
5. Most Not Slop right now
6. Community branches worth installing
7. New & updated
8. "Fresh slop": the most 👎 Slop-voted apps, for fun

**Sexually explicit content is banned** (decided, `LEGAL.md` §5.11). Non-explicit romance and suggestive themes are allowed, with a content label.

**Categories and tags** have two levels:
- **Broad categories:** Games, Tools, Creative, Utilities, Learning, Play style, Vibe.
- **Specific tags** sit under each category (Roguelike, Note-taking, Pixel Art, Touch Controls…).
- Every app has several tags. Every category and tag has its own browsable page.

**Search and filters:** filter by tag, % Not Slop, runs on my platform (including community builds), and released/upcoming. Sort by Not Slop, popular this month, most reviewed, or recently updated. **Built 2026-10-20:** all of these on every list page ("this computer" marks your platform; a community build counts only while it's in the store).

## 6. Library

- **Add to library** is free and instant. Slop then **asks whether to install now**. An app can sit in the library uninstalled.
- The install prompt lists every build for your platform: the official one first, then community builds with caveats (see §9).
- **Interested** upcoming apps appear in the library before they have a build, and users get notified when they enter beta or release.
- **Private Slop** you own or that's shared with you has its own section.
- For each installed app the library shows:
  - Play/Launch.
  - A branch switcher (official vs community).
  - Queued updates, with their scan progress.
  - Uninstall (keeps the app in the library) and Remove.

## 7. Ratings: Slop or Not Slop

- **Two buttons only:** 👍 **Not Slop** and 👎 **Slop**, plus an optional written review.
- **Only users who have launched the app through Slop** can vote. That's the same launch that counts them in Slop Pulse. One vote per account, changeable at any time.
- **Weighted by playtime** (decided): votes from people who played longer count for more. Store pages show **both** the weighted and unweighted % (`METRICS.md` §2).
- Votes and reviews are **public on profiles by default**. Users can make them private.
- **Built 2026-10-20** (`crates/slop-api/src/reviews.rs`): an optional written review under your vote (shown only while you have a vote; edit or delete any time), a Reviews list on app pages with 👍/👎 filters and a Report button on each, and profiles at `/u/{id}` with your apps, People page, and votes and reviews when public. In the UK under-18 view they're private by default (PRIV-6); a private profile's reviews show as "A player". Closing an account deletes its written reviews (its votes stay, anonymised).
- Displayed as **% Not Slop**, with a label so the direction is always clear:

| % Not Slop | Label |
|---|---|
| 95–100 | Overwhelmingly Not Slop |
| 80–94 | Very Not Slop |
| 70–79 | Mostly Not Slop |
| 40–69 | Mixed |
| 20–39 | Mostly Slop |
| 0–19 | Overwhelmingly Slop |

- **Ratings over time:** a monthly chart of % Not Slop shows whether updates and branches are improving the app.
- **Since the new work:** apps that hired creatives through Slop also show % Not Slop from votes since the first release with that work (§15.4). Hiring never changes how votes count.
- Voting opens at **full release**. Coming soon and beta pages show Interested counts instead.

## 8. Publishing

**Requirements for every listing:**
1. **Open source**, in a public repo (GitHub, GitLab, Bitbucket or self-hosted) under an OSI-approved license. Private Slop may use a private repo (§13).
   - **Only the repository's owner can put it on Slop** (built 2026-10-20, `repo_owner.rs`). Collaborators can't, even with admin rights. One app per repository; anyone else builds on it as a branch from their own fork, which must be theirs too. Checked whenever a repository is linked or changed (apps linked before this weren't re-checked). How it's proved:
     - **GitHub:** a GitHub account linked to the Slop account owns it, or is an owner of the organization that does (through Slop's GitHub App on the organization).
     - **GitLab.com:** a linked GitLab account owns the project, or is an Owner of its group.
     - **Bitbucket Cloud:** a linked Bitbucket account owns it.
     - **Anything else** (self-hosted GitLab, Gitea, Forgejo…, and GitLab groups or Bitbucket workspaces Slop can't ask about): a code Slop gives the publisher, added to the README (or a `slop-verify.txt` at the top) on the default branch. Slop reads it with Git, so any public https host works. This proves the publisher can push to it, the most such hosts let anyone check.
2. **At least one build** for at least one platform, on the **maker's own release page**. Normally it's made by **Slop Build**: the maker just verifies their account, links the repo, and accepts one ready-made pipeline file (`ARCHITECTURE.md` §6.2). The build is signed in the pipeline. App files are preferred, but an installer or APK is allowed. Coming soon pages are the only exception.
3. **Passes every scan** (§10).
4. **Games:** **Slop Pulse** (§11) is recommended, not required (decided 2026-10-18).
5. **Publisher Agreement** (`LEGAL.md` §11): owns the rights (including AI output), no loot boxes, no pay-to-win, no scams, no hidden telemetry, nothing unlocked by donations.

**Tracked branch:** the publisher picks one branch as the official version. Each push to it runs Slop Build and **queues an update**. The update is scanned, signed by Slop, and then delivered to everyone who installed from that branch, all at once (no staged rollout).

**Release stages:**

| Stage | Needs | Time limit |
|---|---|---|
| **Coming soon** | Source + every scan (games: Pulse SDK found in source). No build. | 30 days, then must move to Beta |
| **Beta** | At least one build + every scan (games: live Pulse handshake) | 12 months, then must fully release |
| **Released** | Same as Beta | — |

- **Dead project:** a Coming soon or Beta project with **no commits for 90 days** is labelled dead (decided; **only** for Coming soon and Beta, never Released; the value is configurable). It stays visible but is never recommended, and it comes back with the next commit.
- **Missed deadline:** the page is **delisted** (hidden from the store, search and recommendations) until it moves to the next stage (decided). Existing installs and libraries keep working.
- People can mark Coming soon and Beta apps **Interested**, which puts them in their library and gives publishers a demand signal. (Built 2026-10-20: the count shows on tiles and pages, "Most anticipated" sorts by it, Interested tags count double in "For you", and everyone Interested is notified when the app reaches beta or release.)

### Notifications (built 2026-10-20)
- A 🔔 in the top bar with an unread count, and a notifications page. Kinds: **People jobs** (every step by the other side, and new messages, merged while unread), **Interested** apps moving on, and **moderation** (decisions, appeal answers, report outcomes).
- Each kind can also be **emailed**, on by default and switched off per kind on the account page. Product emails only, never marketing (TERMS-3). Signed-out reporters get their report's outcome by email.
- Emails go out only when the server has SMTP set (`deploy/README.md`).

## 9. Branches & community builds

- Anyone can fork the repo and **register a branch**: mods, fixes, translations, ports.
- Branches with builds are scanned like any other upload and can be installed directly.
- **Filling platform gaps:** if the publisher ships only Windows and a branch adds a Linux build, Linux users see it as an option. It's clearly labelled **Community** and comes with these caveats:
  - Made by @author, not the publisher.
  - Passed the same security scan.
  - May lag behind official updates.
  - Links to the branch's README, and requires an "I understand" checkbox before installing.
- Community builds of games must include Slop Pulse too.
- **Built 2026-10-20** (`crates/slop-api/src/branches.rs`): a branch is an app of its own (the author's fork, releases reviewed and signed like any app, its own votes, reviews and Donate button) linked to the original. "Register a branch" on an app's page (mod, fix, translation, port or other; your own fork, not the original repository). It isn't listed in the store on its own: the original's page lists its Community builds, ones that add a platform first ("adds Linux"), and there's a Community builds shelf and page. A branch's page says whose it is, and installing needs the caveats ticked. Built since: community builds of games whose original counts players with Pulse can't be listed before their own first Pulse handshake (the branch's Manage page says so), and "runs on my platform" in search includes community builds.

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
- **Microsoft Defender:** Windows builds are checked with Defender and the day's definitions on GitHub's Windows runner, in the maker's own repository: Slop Build does it on every build, and **Slop Check** (`slop-build/.github/workflows/check.yml`) does it for makers who build elsewhere. The result reaches Slop with the run's GitHub OIDC token, which proves which repository and workflow sent it. With scanning on, a Windows file needs a clean Defender check before approval.
- **Signing:** Slop Build signs the Windows program with the maker's free SignPath Foundation certificate when they have one, and every check records who signed it.
- **Dependency audit and secrets:** Slop Build runs OSV-Scanner and gitleaks on the source. Only the maker and reviewers see the results, and they never stop a release.
- **False positives:** a reviewer can clear a scan finding or a Defender detection with a reason (10+ characters, audit-logged, shown to the maker).
- **Public report:** the app page says what the newest approved release was scanned with, Defender's result and who signed it. Never "safe".

Why not upload to Microsoft, VirusTotal and the like for makers: Microsoft has no submission API (false positives are submitted by the developer, by hand), and the free tiers of VirusTotal, MetaDefender and Hybrid Analysis don't allow commercial use. Players aren't shown SmartScreen's "unrecognized app" prompt in the first place, because the Slop app adds no Mark-of-the-Web to what it downloads.

Results are **passed**, **note**, **on hold** (fixable, e.g. a license mismatch or a leaked key), **rejected** (a policy violation, e.g. a loot box) or **blocked** (malicious; the account is flagged for human review). Every app has a public security report.

## 11. Slop Pulse (player count)

- **Purpose:** show "N playing now" and a monthly peak for games. Nothing else.
- **How it works:**
  1. When the Slop client launches a game, it passes a **one-time signed launch token**.
  2. A tiny open-source library in the game sends `start` and then a heartbeat every few minutes, carrying only the app id and that token.
- **Anti-botting:**
  - Tokens are only issued for real launches by signed-in installs, and are single-use per session.
  - Each account counts once per game.
  - Heartbeats must chain from a valid token and are rate-limited.
  - Spikes from new accounts are flagged.
  - Copies launched outside Slop simply aren't counted.
- **Optional** (decided 2026-10-18; it was planned as required for public games). Recommended for games: it powers "playing now", the monthly peak and playtime-weighted votes. The publishing guide (`web/public/publishing.md`) has the protocol and code for common languages.
- **Privacy:** no device IDs or gameplay data. Extra payload fails the scan.

## 12. Money

Details are in `PAYMENTS.md`, and the legal reasoning is in `LEGAL.md` §3–4.
- **Nothing costs money to download.**
- **Slop never holds money.** Every payment goes straight into the maker's or creative's own Stripe account. Slop receives only its 5% platform fee.
- **Donate button:** presets around the publisher's suggested amount, from $1 to $500. **The donor pays the card fees**, and the total is shown before paying. **Donors are never listed publicly,** and get nothing but a thank-you.
- **Donations unlock nothing,** and apps can't tell who donated. That's what keeps a donation from being a purchase.
- **Server donation:** a monthly recurring donation, only for apps that declare they run servers. It never controls access to those servers. Cancel any time.
- **Built 2026-10-20** (`PAYMENTS.md` §10): the Donate box on app pages, server donations, the maker's settings, totals and refunds on the Manage page, and donation history, cancelling and fee statements on the account page.
- **Where the money goes:** **one** Stripe account per app, picked by the publisher. If several people made it, the publisher splits the money with them outside Slop. Community branches have their own Donate button.
- **Slop's fee:** a flat **5%** of each donation, charged to the maker.
- **No in-app purchases** (decided). Apps can't sell anything: no items, supporter packs, subscriptions, loot boxes or premium currencies. The monetization scan rejects them.
- **Anti-scam detection:** fake giveaways, phishing, credential prompts, undisclosed ads, crypto miners.

## 13. Private Slop

- Publish something **only you can see**. It never appears in the store, search, recommendations or official team pages.
- **Same requirements** as public apps (at least one build, every scan) **except Slop Pulse**, and the source can be in a **private repo** that Slop is allowed to read (`SECURITY.md` §12). Builds stay on the owner's private release page. People it's shared with download through short-lived links, and Slop never stores the software. No store page, no donations, no voting.
- **Sharing:** with specific friends and/or **player teams** you belong to. **Never with or to official teams.**
- **Built 2026-10-20** (`crates/slop-api/src/private.rs`): "Who can see it?" when publishing; the owner shares by pasting someone's profile link (there are no friends lists yet) and can stop sharing; people see it once it has an approved release, in a Private Slop section of their library, and are notified. Every request checks access on the server (`catalog::push_shared_with`); private apps can't be listed, and have no voting, reviews, donations, Interested or branches. Sharing with player teams comes with teams (§14). Also built: **making it public** (the owner asks once the repository is public; a reviewer lists it after Slop checks again, and the shares end since everyone can see it) and **the 14-day freeze** when Slop can't read the repository (`frozen.rs`, for every app: no new releases, the owner is told, then out of the store or no longer shared after 14 days; back when access returns, with "Check now" on the Manage page). Repositories outside GitHub are checked only when public, since Slop doesn't take deploy tokens yet.

## 14. Teams

- **Player teams:** anyone can create or join (open or invite-only). Members share picks and game nights, and can see Private Slop shared with the team.
- **Official teams:** verified developer studios. They show the apps they publish. They can't receive Private Slop.
- **How a studio is verified (proposed):** through its **GitHub organization** (or the GitLab equivalent). An official team needs all of the following:
  1. A GitHub organization that the person setting it up is an **owner** of. Checked through GitHub sign-in.
  2. The org's own **verified domain** on GitHub (GitHub's "Verified" badge on the org). Or a DNS TXT record Slop asks them to add to the studio's website domain.
  3. **Two-factor authentication required** for every org member (a GitHub org setting Slop can check).
  4. The team's apps are published from repos **inside that org**, and their builds are signed by that org's CI (`SECURITY.md` §5.3).
  5. *(If they take donations)* the studio's Stripe account is verified as a **business** by Stripe.

  Verification is re-checked regularly. If any of these stop being true, the badge is removed.
- **Built 2026-10-20** (`crates/slop-api/src/teams.rs`, `/teams`):
  - **Player teams:** open or invite-only (by profile link); owners, admins and members; picks with a note; game nights that notify the other members; Private Slop shared with the team. The last owner hands over before leaving; the last member out ends the team.
  - **Official teams:** checked through Slop's GitHub App installed on the organization (owner role of the team's longest-standing owner, GitHub's verified domain or a `_slop-team.<domain>` TXT record, the organization's 2FA requirement, apps only from the organization's repositories, and Stripe business type when they take donations). "Check now" and a daily re-check; the owners are told when the badge goes. Verified teams are listed, show their apps, and their apps carry an "✓ Official" link to the team. Official teams can't receive Private Slop or have picks.
  - Teams can be reported and hidden by a moderation decision. Not yet: GitLab organizations, and checking that builds are signed by the organization's own CI.

## 15. People (creative marketplace)

**Decided 2026-10-19.** People is a **separate section** from the store. Artists, composers, sound designers, writers, voice actors, testers and studios offer services to makers. It's the one place on Slop where something is **sold**: a creative's work to a maker (`LEGAL.md` §4). The aim is that hiring a real person through Slop is **easier than any other way**: one offer, one tap to pay, credit on the app page when it's done.

### 15.1 Profiles (different from app pages)
A creative's page is built to sell a service, so it isn't an app page with a different header:
- **Header:** name, "Individual" or "Studio · N people", headline, location, skills, "Taking work" or "Fully booked", typical turnaround, and the **AI-use disclosure** (required: No AI / AI-assisted / Mostly AI-generated, with an optional note). Whether they're a **business (trader)** is shown too (`LEGAL-GUIDANCE.md` HUM-1).
- **Rating:** % Not Slop from **verified clients only** (a completed job through Slop), shown from 3 reviews, with the count.
- **Portfolio:** pictures, videos (YouTube) and **audio tracks** for composers. Like app pictures, Slop doesn't host them: they live in a public GitHub repository, listed in `.slop/portfolio.json`, read at one commit, checked, and **reviewed before they show** (§15.8). A first profile goes live when its first portfolio is approved.
- **Services:** up to 8 packages (name, what's included, starting price, typical days, skill). "Hire" on a package starts a request with it filled in.
- **Verified reviews:** each one says ✓ Verified client, the project and what was made, and the client's 👍/👎 with text. The creative can reply once, publicly.
- **Worked on:** the apps that credit them (§15.4).
- **Customisation:** an accent colour (from a palette that keeps contrast readable), a banner and avatar from their portfolio, the **order of the sections** (portfolio, services, reviews, worked on, about), the portfolio order, an About section (Markdown, through the safe renderer), and up to 5 links (website, ArtStation, Bandcamp…), shown with their domain.
- **Directory (`/people`):** filters for skill, taking work now, AI use, studios only. Default order is a **daily shuffle**, so nobody is buried. "Best rated" and "Most clients" are sorts the viewer picks (HUM-10).

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
| 7. Accept the delivery | Maker | **Accept the delivery — pay the rest** (or just accept, if nothing's owed), or ask for a change, or open a problem. After **7 days** with no answer, the delivery counts as accepted and anything owed stays owed. |
| 8. Final files | Creative | After the last payment, **send the final files**. The job completes. |
| 9. After | Both | The maker can review the creative. The creative is credited on the app page, and the app appears on their profile. |

- **Cancelling:** before acceptance, either side can decline. After it, the creative can cancel (and refunds from their Stripe dashboard); the maker can back out only before paying anything. EU/UK consumers get the legal **withdrawal button** (HUM-3).
- **If the maker doesn't pay the rest:** they never get the final files; until they've paid in full they have **no right to use the work** (copyright stays with the creative, so shipping it is infringement and the creative can have it taken down); the job shows as owed; **they can't hire anyone again until it's paid**; and creatives see a factual payment record on every job ("paid in full on 11 of 11 finished jobs"). Beyond that, the record is the creative's evidence for small claims.
- **If the creative doesn't deliver what was agreed:** the maker doesn't accept the delivery, opens a problem (§15.6), and reviews the creative. Their review is the consequence on the creative's side.

### 15.3 The contract
The **default Service Agreement** (`crates/slop-api/src/people/service_agreement.md`, versioned) is filled in from the offer. Both sides see the same plain summary:
- **What, exactly:** title and an exact scope. "This agreement binds both of you to these words": the creative delivers what's written, nothing more or less; anything unwritten isn't part of it.
- **Price and payment:** up front, **half now and half on accepting the delivery (default)**, or all on accepting the delivery. Delivery before the last payment is a preview; the final files follow it.
- **Due date**, and an optional **sign-off** on the direction.
- **Changes are new work.** There are no free "revisions": after acceptance, any change — added, removed, redone, a different style or direction, a new date — is a **Change to Agreement**: a separate document the creative writes, with its own scope, price (which may be zero) and due date. It applies only when the maker accepts it (ticking the exact-wording box again) and pays its price; until then the agreement stands as written. The creative may decline a requested change. Each accepted change is stored with its own SHA-256, referring to the agreement's.
- **Who owns it:** **"You own it"** (copyright is transferred to the maker on full payment; the creative may show it in their portfolio) or **"Licence for this project"** (the maker may use, change and publish it in the named project, forever; the creative keeps the copyright). Nothing until paid in full.
- **Open source:** whether the work may be published **under the project's open-source licence** (anyone can reuse it), or stays **all rights reserved** inside the repository. Asked plainly, because every public Slop app has public source.
- **Credit:** shown on the project's page (on by default).
- **AI use** for this job, and the creative's promise that it's true and that they have the rights to what they deliver.
- **Refunds:** the creative's policy (default: full refund before work starts; after that, for undelivered parts). Problems and mediation (§15.6).
- **Records and conversations** (§15.5), and the **law**: the creative's country, without removing the maker's consumer rights where they live.

"Accept" is the signature (US E-SIGN, EU eIDAS simple electronic signature). Slop stores the exact text both sides agreed to, its hash, the template version, and who accepted when. **Both sides can download the full record** (agreement, changes, timeline, payments, problems) at any time.

### 15.4 What a project gets: credit, a badge, and "since the new work"
Hiring people **never changes how votes count**. A rating is what players say (`LEGAL.md` §5.5). Instead:
- **Credits** on the app page: "Art by @creative · Music by @composer", linking to their profiles.
- **✋ Made with people badge** on the app page and store tiles, plus a store filter and shelf, once the app has at least one **completed, fully paid** job through Slop of $10 or more. It counts jobs, not money spent, so a small pixel-art job counts the same as a big one.
- **"Since the new work" rating** on the app's ratings panel: % Not Slop from votes cast or changed **since the first release after the job completed**, beside the overall rating. Old 👎 votes about AI art no longer hide that the app improved, honestly, because only newer votes are in it.
- These count only for jobs done through Slop, which is the reason to stay: creatives can always take work elsewhere.

### 15.5 Conversations: recorded, opened by the creative
- **The creative starts conversations.** The maker can **ask for a conversation at any time** (with a short note), but only the creative opens it, and closes it. This keeps the maker from changing the requirements by message; changes go through §15.3.
- **Recorded, and said so everywhere.** Above every conversation, and in a box each side ticks before their first message: *"This conversation is recorded, so you both have proof of what was said."*
- **The two sides keep it, not Slop (decided 2026-10-19).** Slop holds the text while the conversation is open and for **30 days after it closes**, so both can **download their copy**; then it deletes the text and keeps only a **fingerprint** of each message (a SHA-256 chained to the one before). The fingerprints prove a copy is unchanged and complete, without Slop being able to read it.
- **Slop staff see a conversation only if a side submits their copy** for a problem or a review dispute. Slop checks it against the fingerprints and tells the mediator how many messages match.
- These job conversations are separate from personal chat (§19), which stays end-to-end encrypted.

### 15.6 Problems, mediation and review disputes
- **Problems.** Either side can open one on an accepted job: they state their case, the other answers, and either proposes outcomes (a refund, redoing a part at no charge, cancelling, something else). **Only what both accept happens**, and Slop carries it out where it can (cancelling; refunds are issued by the creative in Stripe). A **Slop mediator** (a person, or an AI reviewer with an admin token) can read the record — every look is logged — and **suggest** an outcome, which binds no one. If they can't agree, the mediator closes it unresolved and both keep the record for a card dispute or a court. **Slop never decides who's right about money** (LEGAL §4.1).
- **Review disputes.** A creative can dispute a review that breaks the rules, isn't about the job, was used as a threat ("more for free or I'll leave a bad review"), or states something the record shows is false. The reviewer can answer. A mediator then **removes it**, **keeps it with a factual note** (shown under the review), or **keeps it**, always with a written reason shown to both. Reviews are **never removed for being negative**: an honest opinion stays, and the creative's public reply is their answer (HUM-12, HUM-18).

### 15.7 Money in one line
The maker pays the creative **directly** (Stripe, the creative's own account). Slop takes **5%** as its platform fee, out of what the creative receives, and never holds the money. Details: `PAYMENTS.md` §7.

### 15.8 Review and safety
- New portfolios and portfolio changes wait for review in the admin panel, like app pictures, and AI reviewers can see and decide them (`review.rs`).
- 18+ to sell or hire (confirmed on first use). Rules from `LEGAL-GUIDANCE.md` §4 (trader declaration, withdrawal, independence of creatives) apply.
- **Slop doesn't settle disputes.** It acts on rule-breaking (scams, fraud, repeated non-delivery).

## 16. Reporting & moderation

- Every app, branch, review, People profile and job message has a **Report** button. Reasons depend on what's reported: malware, the build not matching its source, scams, illegal content, child abuse material, intimate images shared without consent, threats, sexual content, copyright, open-source licences, pay-to-win and gambling, misleading pages, harassment, private information, fake reviews, spam, broken builds, other.
- **Anyone can report**, signed in or not (a signed-out reporter gives a name and email, optional for child abuse). Each report needs a good-faith statement and is **acknowledged with a reference number**; signed-in reporters see the outcome on their account page. One open report per person per thing; 20 a day at most.
- **Urgent** reports (child abuse, threats, intimate images) and **serious** ones (malware, scams, illegal content, source mismatch, sexual content, copyright) get a **human review within 24 hours**; the rest within 7 days. Serious reports will also trigger an **automatic re-scan** once the scan pipeline exists (§10). Intimate-image requests are removed within 48 hours at the latest (TAKE IT DOWN Act).
- **Copyright reports** collect what a DMCA notice needs; the appeal against a copyright decision is the counter-notice. Three upheld copyright decisions end an account (repeat-infringer policy).
- **Decisions:** take an app out of the store, hide a review or profile, warn, or suspend an account. Acting closes every open report about the same thing. Each decision sends the person a **statement of reasons**: what was done, the facts, the Content Policy rule, the law if any, that a person decided it, and how to appeal. Never hide anything for being negative (REV-2).
- **Appeals** within six months, once per decision, on the account page. A person keeps or reverses the decision and says why; reversing undoes it.
- **Suspended accounts** can still sign in to read Slop, see the decision, appeal, download their data or close the account; nothing else.
- Child-abuse reports are preserved for a year, and the moderator notes where they were filed (NCMEC, NCA) and police referrals for threats.
- The rules are the **Content Policy** (`/legal/content`); each decision names its section.

## 17. Decisions and open questions

**Decided 2026-10-17:**
0. **Company:** Slop is operated by **IronPoint Development LLC** (US). No in-app purchases. One Stripe account per app. Donors get nothing. Chat is between people and **end-to-end encrypted**. Launch in the **US, EU and UK**. **Slop stores no builds or source code:** builds live on makers' release pages, and every public app has public source.
1. **Missed deadline:** delisted until the project reaches the next stage.
2. **Dead projects:** 90 days without commits, only for Coming soon and Beta.
3. **Private Slop:** private repos are allowed (GitHub, GitLab, Bitbucket, self-hosted).
4. **Votes:** weighted by playtime, with weighted and unweighted both shown. Public on profiles by default.
5. **Money:** Stripe, with Slop never holding money. Donors pay card fees. Donors anonymous by default. No wallet or top-ups. No Slop dispute resolution for People, and a 7-day completion window. Monthly **server donations**.
6. **Builds:** made by **Slop Build** on GitHub's free runners and kept on the maker's release page. Slop stores no builds or source. The pipeline signs, then Slop signs after scanning. No staged rollouts. Steam-style installs into a library folder (`ARCHITECTURE.md` §6).
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
6. **Private Slop downloads** through short-lived GitHub links (`ARCHITECTURE.md` §6.5).
7. **Where exactly "explicit" begins:** the wording for the Content Policy (`LEGAL.md` §5.11).

## 18. What the mockup simulates

All data is invented. Sign-in, scans, installs, Pulse handshakes, donations, reports, GitHub links, phone sending and quotes are simulated in the browser. Everything you do is saved only in your browser. Use the **Preview as** switch in the title bar to see Windows, macOS, Linux, iOS and Android.

## 19. Chat

A Steam-style communication system: lightweight, not a Discord. **Built 2026-10-20** (`crates/slop-api/src/chat.rs` and `friends.rs` on the server, `crates/slop-client/src/chat/` in the app, `web/src/pages/Chat.tsx` and `Friends.tsx`):
- **Friends:** requests from someone's profile (or by pasting their profile link), accepted under the 🔔; a friends page with requests, blocked people and "who can message you". Works on the website too.
- **Chats:** direct messages, group chats (up to 50 people, named inside the encryption, the starter can remove people) and one chat per player team (following who's in the team). Unread counts, mute, hide.
- **Messages:** Markdown-lite text (bold, italics, code, quotes, lists, links), replies, emoji reactions, editing and deleting your own, pictures, GIFs, short videos and files up to 15 MB (attach, paste or drop). A link to a picture or GIF (a meme) is fetched by the **sender's** app and sent as an encrypted attachment, shown inline; other links get a preview card made the same way, so recipients' apps never contact the site. Previews can be switched off.
- **Encryption:** MLS through OpenMLS in the app (SECURITY §8.5); Slop orders and relays encrypted messages and deletes them after 30 days. A new device is approved on an existing one by comparing a 9-character code; the approving device sends it the chat history. Encrypted backups are on by default with a 24-character recovery code shown once; "start over" (every device lost) needs a fresh sign-in and changes the person's security code, which every chat shows. Security codes can be compared per person.
- **Safety:** messages only between connections (friends, player teammates, People job partners) unless someone allows messages from anyone (never in the UK under-18 view); blocking ends a friendship, stops their messages and hides the chat; every message from someone else has Report (the app sends that one decrypted message with its franking key, checked by Slop; nothing else is revealed), with an option to block too; a warning before opening any link, and on messages asking for payment outside Slop (PayPal, gift cards, crypto…). Away from the app, a 🔔 notice (and email, if wanted) says there are new messages without saying what.
- **Typing indicators** ("Mia is typing…", a few seconds, ended by the message). They go through Slop unencrypted but say nothing except "typing", and are never stored.
- **Notifications:** the desktop app shows a system notification for new messages while Slop isn't in front (one per chat for a burst; muted chats never; the text can be left out), switchable under Account → This computer. **Phones and browsers** get web push for everything under the 🔔 (Account → Notifications): on Android in the browser, on iPhone once Slop is added to the Home Screen. A push carries nothing; the phone fetches the notice from Slop itself.
- **In-game overlay (Shift+Tab):** over a game started from Slop that's fullscreen, borderless fullscreen or maximized, Shift+Tab brings up a small panel with chats and friends over the dimmed game; Shift+Tab, Esc or a click outside returns. Anywhere else Shift+Tab is left alone: the shortcut is only taken while such a game is the window in front. Nothing is injected into the game (so games in exclusive fullscreen briefly switch to the desktop, like Alt+Tab). Windows, and Linux under X11 or XWayland. Can be switched off under Account → This computer.
- Not planned: voice and GIF search (paste a link instead). Not yet: chat in a phone app.

The first draft, for reference:
- **Friends and direct messages** between users, plus **group chats** for player teams.
- **People job conversations** are different: recorded, opened by the creative, kept by the two sides with fingerprints at Slop (§15.5). They aren't part of this end-to-end encrypted chat.
- **History backups are on by default** (opt-out). They're encrypted with a key only the user holds, so they stay end-to-end (`SECURITY.md` §8.5).
- **Privacy (decided):** messages are between people and **end-to-end encrypted**, so Slop can't read them (`SECURITY.md` §8.5). Link warnings run in the recipient's app. Moderators only see messages that a participant reports. Chat is in the Slop app only, not the website.
- **Safety:**
  - **UK under-18 view:** only connections (friends, team members, People job partners) can message the user, and the "anyone" setting is locked off.
  - Block and report buttons.
  - Warnings on off-platform payment requests and on links to unknown sites.
  - Only users who are friends or share a job or team can message each other, unless a user allows messages from anyone.
- **Open:**
  - How long encrypted attachments are kept.
  - Real-time delivery (WebSockets) and notifications.
  - Voice chat: probably not.
