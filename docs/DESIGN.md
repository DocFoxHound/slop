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

**Search and filters:** filter by tag, % Not Slop, runs on my platform (including community builds), and released/upcoming. Sort by Not Slop, popular this month, most reviewed, or recently updated.

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
- People can mark Coming soon and Beta apps **Interested**, which puts them in their library and gives publishers a demand signal.

## 9. Branches & community builds

- Anyone can fork the repo and **register a branch**: mods, fixes, translations, ports.
- Branches with builds are scanned like any other upload and can be installed directly.
- **Filling platform gaps:** if the publisher ships only Windows and a branch adds a Linux build, Linux users see it as an option. It's clearly labelled **Community** and comes with these caveats:
  - Made by @author, not the publisher.
  - Passed the same security scan.
  - May lag behind official updates.
  - Links to the branch's README, and requires an "I understand" checkbox before installing.
- Community builds of games must include Slop Pulse too.

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
- **Where the money goes:** **one** Stripe account per app, picked by the publisher. If several people made it, the publisher splits the money with them outside Slop. Community branches have their own Donate button.
- **Slop's fee:** a flat **5%** of each donation, charged to the maker.
- **No in-app purchases** (decided). Apps can't sell anything: no items, supporter packs, subscriptions, loot boxes or premium currencies. The monetization scan rejects them.
- **Anti-scam detection:** fake giveaways, phishing, credential prompts, undisclosed ads, crypto miners.

## 13. Private Slop

- Publish something **only you can see**. It never appears in the store, search, recommendations or official team pages.
- **Same requirements** as public apps (at least one build, every scan) **except Slop Pulse**, and the source can be in a **private repo** that Slop is allowed to read (`SECURITY.md` §12). Builds stay on the owner's private release page. People it's shared with download through short-lived links, and Slop never stores the software. No store page, no donations, no voting.
- **Sharing:** with specific friends and/or **player teams** you belong to. **Never with or to official teams.**

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

## 15. People (creative marketplace)

**Decided 2026-10-19.** People is a **separate section** from the store. Artists, composers, sound designers, writers, voice actors, testers and studios offer services to makers. It's the one place on Slop where something is **sold**: a creative's work to a maker (`LEGAL.md` §4). The aim is that hiring a real person through Slop is **easier than any other way**: one offer, one tap to pay, credit on the app page when it's done.

### 15.1 Profiles (different from app pages)
A creative's page is built to sell a service, so it isn't an app page with a different header:
- **Header:** name, "Individual" or "Studio · N people", headline, location, skills, "Taking work" or "Fully booked", typical turnaround, and the **AI-use disclosure** (required: No AI / AI-assisted / Mostly AI-generated, with an optional note). Whether they're a **business (trader)** is shown too (`LEGAL-GUIDANCE.md` HUM-1).
- **Rating:** % Not Slop from **verified clients only** (a completed job through Slop), shown from 3 reviews, with the count.
- **Portfolio:** pictures, videos (YouTube) and **audio tracks** for composers. Like app pictures, Slop doesn't host them: they live in a public GitHub repository, listed in `.slop/portfolio.json`, read at one commit, checked, and **reviewed before they show** (§15.6). A first profile goes live when its first portfolio is approved.
- **Services:** up to 8 packages (name, what's included, starting price, typical days, skill). "Hire" on a package starts a request with it filled in.
- **Verified reviews:** each one says ✓ Verified client, the project and what was made, and the client's 👍/👎 with text. The creative can reply once, publicly.
- **Worked on:** the apps that credit them (§15.4).
- **Customisation:** an accent colour (from a palette that keeps contrast readable), a banner and avatar from their portfolio, the **order of the sections** (portfolio, services, reviews, worked on, about), the portfolio order, an About section (Markdown, through the safe renderer), and up to 5 links (website, ArtStation, Bandcamp…), shown with their domain.
- **Directory (`/people`):** filters for skill, taking work now, AI use, studios only. Default order is a **daily shuffle**, so nobody is buried. "Best rated" and "Most clients" are sorts the viewer picks (HUM-10).

### 15.2 Hiring, start to finish
Every job belongs to **one of the maker's projects**, at any stage (coming soon, beta or released; public or private). That's what lets the work be credited.

| Step | Who | What they see and do |
|---|---|---|
| 1. Get paid set-up | Creative, once | "Get paid": Stripe's own sign-up, a few minutes. Until it's done, the profile says "Not taking paid work yet". |
| 2. Request | Maker | **Hire** (on a package or the profile) → pick the project → say what they need (and a hoped-for date). |
| 3. Offer | Creative | The offer is pre-filled from the package: **what you'll get, price, payment (half now, half on delivery by default), due date, revisions, who owns it, open-source use, credit**. They adjust and send. Or decline. |
| 4. Accept | Maker | A **one-screen summary in plain words**, with the full Service Agreement one tap away. **Accept & pay $X** (the first, largest number is what they pay). Or "Ask for changes" with a note, or decline. |
| 5. Work | Creative | Starts once the first payment is in (or straight away for "all on delivery"). |
| 6. Deliver | Creative | **Mark delivered** with a note and links to the files. |
| 7. Finish | Maker | **Looks good** (paying the rest, if any), or **Ask for a revision** while revisions are left. After **7 days** with no answer, the job completes on its own. |
| 8. After | Both | The maker can review the creative. The creative is credited on the app page, and the app appears on their profile. |

- **Job records, not chat:** the request, offer, notes on changes, delivery and revision notes are short texts that are **part of the contract record** (kept for contract claims, `LEGAL-GUIDANCE.md` §15). Free conversation belongs in end-to-end encrypted chat (§19), which isn't built yet.
- **Cancelling:** before acceptance, either side can decline. After it, the creative can cancel (and refunds from their Stripe dashboard). EU/UK consumers get the legal **withdrawal button** (HUM-3). Slop doesn't decide disputes.

### 15.3 The contract
The **default Service Agreement** (`crates/slop-api/src/service_agreement.md`, versioned) is filled in from the offer. Both sides see the same plain summary:
- **What:** title and scope.
- **Price and payment:** up front, **half now and half on delivery (default)**, or all on delivery.
- **Due date and revisions.**
- **Who owns it:** **"You own it"** (copyright is transferred to the maker on full payment; the creative may show it in their portfolio) or **"Licence for this project"** (the maker may use, change and publish it in the named project, forever; the creative keeps the copyright).
- **Open source:** whether the work may be published **under the project's open-source licence** (anyone can reuse it), or stays **all rights reserved** inside the repository. Asked plainly, because every public Slop app has public source.
- **Credit:** shown on the project's page (on by default).
- **AI use** for this job, and the creative's promise that it's true and that they have the rights to what they deliver.
- **Refunds:** the creative's policy (default: full refund before work starts; after that, for undelivered parts).
- **Law:** the creative's country, without removing the client's consumer rights where they live.

"Accept" is the signature (US E-SIGN, EU eIDAS simple electronic signature). Slop stores the exact text both sides agreed to, its hash, the template version, and who accepted when.

### 15.4 What a project gets: credit, a badge, and "since the new work"
Hiring people **never changes how votes count**. A rating is what players say (`LEGAL.md` §5.5). Instead:
- **Credits** on the app page: "Art by @creative · Music by @composer", linking to their profiles.
- **✋ Made with people badge** on the app page and store tiles, plus a store filter and shelf, once the app has at least one **completed, fully paid** job through Slop of $10 or more. It counts jobs, not money spent, so a small pixel-art job counts the same as a big one.
- **"Since the new work" rating** on the app's ratings panel: % Not Slop from votes cast or changed **since the first release after the job completed**, beside the overall rating. Old 👎 votes about AI art no longer hide that the app improved, honestly, because only newer votes are in it.
- These count only for jobs done through Slop, which is the reason to stay: creatives can always take work elsewhere.

### 15.5 Money in one line
The maker pays the creative **directly** (Stripe, the creative's own account). Slop takes **5%** as its platform fee, out of what the creative receives, and never holds the money. Details: `PAYMENTS.md` §7.

### 15.6 Review and safety
- New portfolios and portfolio changes wait for review in the admin panel, like app pictures, and AI reviewers can see and decide them (`review.rs`).
- 18+ to sell or hire (confirmed on first use). Rules from `LEGAL-GUIDANCE.md` §4 (trader declaration, withdrawal, independence of creatives) apply.
- **Slop doesn't settle disputes.** It acts on rule-breaking (scams, fraud, repeated non-delivery).

## 16. Reporting & moderation

- Every app, branch, review and People profile has a **Report** button. Reasons: malware, license violation, illegal content, scam, pay-to-win/loot boxes, source doesn't match build, misleading page, broken build, other.
- Serious reports (malware, illegal content, scams, source mismatch) trigger an **automatic re-scan** and a **human review within 24 hours**.

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

## 19. Chat (first draft)

A Steam-style communication system. Not designed in detail yet.
- **Friends and direct messages** between users, plus **group chats** for player teams.
- **People conversations:** each quote or job gets its own thread between the creative and the client, with the Service Agreement and the delivery and completion steps shown in the thread. This is how creatives contact their clients.
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
