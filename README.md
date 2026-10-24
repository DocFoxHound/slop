<p align="center">
  <img src="web/public/logo.svg" alt="Slop logo" width="96" height="96">
</p>

<h1 align="center">Slop</h1>

<p align="center">
  <b>The store for apps and games made with AI.</b><br>
  Open source, every one. Checked before it's listed. Free to download. Rated by the people who played it.
</p>

<p align="center">
  <a href="https://slopai.games"><b>slopai.games</b></a> ·
  <a href="docs/OVERVIEW.md">What Slop is</a> ·
  <a href="docs/DESIGN.md">The full design</a> ·
  <a href="docs/DEVELOPING.md">Developing</a>
</p>

---

AI has made it easy for anyone to make software. It hasn't made it easy to tell what's worth
installing. Some of it is brilliant, a lot of it is slop, and most stores either ban it or bury
it.

**Slop puts it all in the open and lets players decide.** Every app shows its source code. Every
build is checked before anyone can install it. And every app carries one honest number: how many
of the people who actually played it say it's **👍 Not Slop**.

## For players

- **Ratings you can trust.** Only people who launched an app through Slop can vote, one vote per
  account, weighted by how long they played. Every store page shows **% Not Slop** and a chart of
  how it changed over time, so you can see whether updates made it better.
- **Checked before it's listed.** Every release file is scanned (ClamAV, YARA rules and capa for
  what a program can do), Windows builds are checked with Microsoft Defender, and a person reviews
  each app before it reaches the store. Slop signs what it approved, and the Slop app checks that
  signature, and the maker's own build signature, every time it installs or starts an app. If
  something bad slips through, Slop can switch it off everywhere.
- **Source for everything.** Every public app is open source under an OSI-approved licence, with
  its code one click away. Don't like something? Anyone can fix it.
- **Free, with nothing to buy.** No prices, no in-app purchases, no loot boxes, no pay-to-win, no
  premium currencies, and no paid placement in the store. Slop's checks look for them in the code.
- **Know what an app does.** Every store page says whether it goes online and where, how it saves,
  whether it loads outside code or runs its own installer, which accessibility features it has
  (confirmed by players), and what Slop's checks found.
- **Play it your way.** Run any app in sandbox mode, keep your saves in sync between your
  computers (end-to-end encrypted, so Slop can't read them), and install community builds side by
  side with the original.
- **Community builds fill the gaps.** If the maker only ships Windows and someone else ports it
  to Linux, Linux players see that build too, clearly labelled and checked the same way.
- **Friends and chat.** Add friends, chat one-to-one or in groups, and share pictures and links.
  Messages are end-to-end encrypted on your devices.

## For makers

- **Free distribution, no gatekeeping on how it was made.** Made it with AI? That's the point.
  Ship small tools, experiments or full games.
- **One file to publish.** Link your repository and accept one ready-made pipeline file. **Slop
  Build** then builds your app in GitHub Actions, signs it, checks it with Defender, audits its
  dependencies, scans for leaked secrets, and sends every new release to Slop on its own.
- **Keep 95% of what people give you.** Players can donate, once or monthly for apps with servers.
  The money goes straight into your own Stripe account; Slop takes a flat 5% fee and never holds
  your money. Donations unlock nothing, so your app stays free for everyone.
- **Launch before you're done.** Put up a **Coming soon** page with just your source, move to
  **Beta**, then **Released**. Players mark projects *Interested*, so you see the demand and they
  hear the moment you ship. New games can offer a playable browser demo for their first two weeks.
- **Real feedback.** See players online now and monthly peaks (with the optional, tiny Slop Pulse
  library), bug reports with crash output, and how each release landed: its rating, reviews and
  bug reports, version by version. Reply to reviews in public.
- **A community that improves your work.** Players can fork your app to fix, mod, translate or
  port it, offer their fixes back to you as pull requests, and put up **fix bounties** that pay
  whoever builds the fix.
- **Hire real people.** **People** is a marketplace of artists, composers, writers, voice actors
  and testers who can replace AI-made assets with crafted ones. One offer, one payment, and their
  credit on your store page.
- **Work with your AI agent.** Give your coding agent a limited token and it can manage your store
  pages, releases, checks and fixes over Slop's API or as an MCP server. Payments, agreements and
  account settings always stay with you.
- **Share privately.** Keep a project to yourself, or share it with friends and your team, even
  from a private repository.

## For artists, musicians and writers

AI-made apps are full of placeholder art, music and words. **People** is where makers find you to
replace them: a portfolio page, requests and quotes, a written contract, payment straight to your
own Stripe account (Slop takes 5% and never holds it), and credit on every app you worked on.

## How an app gets to you

1. **The maker pushes their code.** Slop Build turns it into a signed release on the maker's own
   release page. Slop never stores the software itself.
2. **Slop checks it.** Malware scans, Defender, the maker's signature, the source's licence and
   anything that looks like a purchase or loot box, then a person reviews it.
3. **Slop signs what it approved.** The signed record names every file by its fingerprint.
4. **Your Slop app installs it** from the maker's release page, refuses anything that doesn't
   match, and checks again every time you start it.

Anyone can check a file against what Slop approved, at
[slopai.games/verify](https://slopai.games/verify). Slop's server is open source too, and every
staff decision is recorded in an append-only log.

## Where it's at

Slop is in **private testing**, at [slopai.games](https://slopai.games). The desktop app runs on
**Windows and Linux** today; macOS and phones come later. The Slop app updates itself.

## Get involved

- **Build it:** [`docs/DEVELOPING.md`](docs/DEVELOPING.md) has setup, tests, the API and what's
  built so far. Start with [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`AGENTS.md`](AGENTS.md).
- **Read the plans:** [`DESIGN.md`](docs/DESIGN.md) (what and why),
  [`ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`SECURITY.md`](docs/SECURITY.md),
  [`PAYMENTS.md`](docs/PAYMENTS.md) and the [Slop open standard](docs/standard/README.md) for
  signed releases.
- **Found a security problem?** See [`CONTRIBUTING.md`](CONTRIBUTING.md#security-problems) and
  please don't open a public issue.
- **Questions:** [slopaigames@outlook.com](mailto:slopaigames@outlook.com).

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
