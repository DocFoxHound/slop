# Getting someone onto SLAPorSLOP

This page is for AI assistants. A person asked you to help them get onto SLAPorSLOP: a store for
open-source apps and games made with AI, with **People**, where artists, composers, writers,
voice actors, testers and small studios offer their services to the people making games. Read
this page, then the one guide it sends you to.

Many of the people who ask have built their app with AI and have never used Git, GitHub or a
terminal. Talk to them in plain words, one step at a time, and wait for each step to be done
before the next. Do everything you can yourself; ask them only for what only they can do.

## First: what do they want to do?

Ask once, if it isn't clear already:

1. **Publish an app or game they made.** Read [the publishing guide](/publishing.md) and follow
   its [checklist for agents](/publishing.md#checklist-for-agents). Their project's code must be in
   a public repository (GitHub is easiest); if it isn't yet, see
   [Starting from nothing](#starting-from-nothing-no-github-yet) below first.
2. **Offer their services on People**, alone or as a studio. Read [the portfolio guide](/portfolio.md)
   and follow its checklist. Their portfolio is a public GitHub repository too; if they have no
   GitHub account, see [Starting from nothing](#starting-from-nothing-no-github-yet).

Both is fine: do one, then the other.

## What only the person can do

- Sign in to SLAPorSLOP (with GitHub, Google, GitLab, Bitbucket, Microsoft or Discord) and pick a
  display name and country. **Continue with GitHub** is best for publishing: SLAPorSLOP then knows
  their repositories are theirs.
- Accept SLAPorSLOP's agreements (the Publisher Agreement to publish, the People Terms to offer
  services), confirm they're 18 or older for People, and anything to do with money (donations,
  Stripe for People).
- Choose the licence for their code (see below), and anything else that's their decision.
- Approve their AI tool's connection to SLAPorSLOP, and their GitHub sign-in, in their browser.

Everything else is yours: the repository, the code changes SLAPorSLOP needs, the store page or
People page text, the pictures, the releases, and fixing what SLAPorSLOP's checks find.

**Never ask for passwords, tokens or keys in the chat**, for SLAPorSLOP or GitHub. Every sign-in
here happens in the person's own browser or their own terminal.

## Starting from nothing (no GitHub yet)

A **repository** is a folder online that holds the project's code and its history. SLAPorSLOP reads
the code from there, players can see it (that's what open source means), and SLAPorSLOP builds and
checks each version from it. GitHub is the easiest place to keep one, and it's free.

### 1. A GitHub account (the person)

Send them to <https://github.com/signup>: an email address, a password and a username (it shows in
their repository's address, e.g. `github.com/username/my-game`). They confirm their email, and
GitHub may ask them to set up two-factor sign-in, which is worth doing.

### 2. If you can run commands on their computer

(Claude Code, Codex, Cursor, Grok Build and other coding agents.)

1. **Git:** check with `git --version`. If it's missing, install it: Windows
   `winget install --id Git.Git -e`; macOS `xcode-select --install`; Linux the system's package
   manager (e.g. `sudo apt install git`).
2. **GitHub's command-line tool:** check with `gh --version`. If it's missing: Windows
   `winget install --id GitHub.cli -e`; macOS `brew install gh` (or the installer from
   <https://cli.github.com>); Linux the instructions at <https://cli.github.com>. On Windows, a new
   terminal window may be needed before `git` and `gh` are found.
3. **Sign in to GitHub:** ask the person to run `gh auth login --web --git-protocol https` in
   **their own terminal window** (it's interactive). It shows a one-time code and opens their
   browser, where they sign in and approve. No password goes through you. Then run
   `gh auth setup-git`, and check with `gh auth status`.
4. **Nothing secret in the code.** Projects built with AI often have API keys, passwords or
   tokens written into the code or a `.env` file. Look for them before anything is uploaded: a
   public repository is public forever, history included. Move them out of the code (an
   environment variable, or a file listed in `.gitignore`) and tell the person which keys were
   there, so they can replace any that might have leaked elsewhere.
5. **A `.gitignore`:** build output, downloaded dependencies (`node_modules/`, `target/`,
   `.venv/`…), `.env` files, saves and logs.
6. **A licence:** SLAPorSLOP needs an open-source licence in a `LICENSE` file. Ask the person to
   choose; don't choose for them. In plain words: **MIT** lets anyone use the code for anything as
   long as they keep the notice; **GPL-3.0** makes anyone who shares a changed version share their
   code too. <https://choosealicense.com> explains the rest. Then add the licence's text with
   their name and the year.
7. **Make the repository and upload it,** from the project's folder:

   ```sh
   git init -b main
   git add .
   git commit -m "First version"
   gh repo create my-game --public --source . --push
   ```

   Use the project's name instead of `my-game` (lowercase letters, numbers and dashes). Check
   `git status` before the commit: nothing secret and nothing huge (GitHub refuses files over
   100 MB; build files go in releases, not the repository).
8. Tell the person their repository's address (`gh repo view --web` opens it), then carry on with
   the guide you came from.

From then on, after each change: `git add .`, `git commit -m "What changed"`, `git push`.

### 3. If you can't run commands (a chat on a website or in an app)

Walk the person through **GitHub Desktop**, one step at a time:

1. Download it from <https://desktop.github.com> and sign in with their GitHub account.
2. **File → Add local repository**, choose the project's folder; when it says it isn't a
   repository yet, choose **create a repository** there. Give it the project's name and pick
   **None** for the licence there (you'll write the `LICENSE` file with them, after they choose
   one: see step 6 above), and a `.gitignore` for the project's language if one fits.
3. Before the first commit, go through the list of files with them: nothing secret (step 4 above),
   no build output or downloaded dependencies.
4. Write a summary such as "First version" and press **Commit to main**.
5. **Publish repository**, and **untick "Keep this code private"**: SLAPorSLOP needs it public.
6. Later changes: GitHub Desktop lists them; **Commit**, then **Push origin**.

For a very small project, GitHub's website works too: **New repository** (public, add a licence
from its list after the person chooses one), then **uploading an existing file** and drag the
project's files in (up to 100 files at a time, 25 MB each).

## Connecting your tools to SLAPorSLOP

With SLAPorSLOP's tools you can create the store page, submit releases, follow the review and fix
what it finds. The person connects you once: they add this site's `/mcp` address (e.g.
`https://slaporslop.games/mcp`) to their AI tool and approve it in their browser; no token to
copy. The publishing guide's [Working with your AI agent](/publishing.md#working-with-your-ai-agent)
has the steps for each tool (Claude Code, Claude, ChatGPT, Codex, Cursor, Gemini CLI, Grok
Build). Then call `maker_guide`, and `whats_left`: it lists every step still open, saying
who does it, with a link.

Without a connection you can still do the repository and the code, check them, and give the
person the steps to do on the website. These work for anyone, with no account (GET, JSON with a
`summary` first, nothing saved):

- `/v1/check/repository?url=https://github.com/OWNER/REPO`: is the project ready to publish?
- `/v1/check/store-address?slug=my-game`: is the store address free?
- `/v1/check/portfolio?url=https://github.com/OWNER/portfolio`: is the People portfolio right?

And you can fill in the store page or the People page for them: build one link, they open it, check
it and press Create themselves. See [Hand the person one link](/publishing.md#hand-the-person-one-link)
and [the People page link](/portfolio.md#hand-the-person-one-link-for-their-people-page).
