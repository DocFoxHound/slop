# Publishing on Slop

Slop is a store for apps and games made with AI: open source, checked before they're listed,
free to download, and rated by the people who play them. This page covers everything needed to
publish: what Slop asks of your project, how to set it up, and what happens after you submit.

> **Using an AI coding assistant?** Tell it to read this page (the plain-text copy is at
> `/publishing.md` on this site). It can work out what's required and what's optional, and set
> your project up for publishing on Slop. For example: *"Read this page, tell me what's required
> and what's optional for publishing on Slop, then set up my project so it's ready to deploy
> there."*

**Want your agent to do all of it, store page included?** Make it a token under **Account → AI
agents** (`/account/agents`: publish, all your apps), accept the Publisher Agreement that page
shows you, and connect it with the command it gives; see
[Working with your AI agent](#working-with-your-ai-agent). With a token it creates the store page,
submits releases and follows the review itself. Without one it can only prepare the repository.

It's written so that a person or an AI agent can follow it from start to finish. Agents: the
[checklist for agents](#checklist-for-agents) near the end lists every step in order, and
[What Slop asks of your project](#what-slop-asks-of-your-project) says what's required and what's
optional.

> Each app's first release is reviewed by a person. Things marked *(coming)* below are planned
> but not built yet.

## Why publish on Slop

- **Free to list.** No fees to publish or to download. Slop never sells anything and takes no cut
  of sales, because there are none. Players can donate to you if you turn
  [donations](#donations) on: paid straight to you, with a flat 5% Slop fee.
- **Players who are looking for AI-made software.** Store pages, categories, tags and search,
  with Coming soon, Beta and Released stages so players can follow a project from the start.
- **Installs and updates handled for you.** The Slop desktop app (Windows, Ubuntu/Debian,
  RHEL/Rocky/Alma 10) installs your app, checks every file against Slop's signature first, and
  updates it when you publish a new release.
- **Trust.** Every app links its source code. Every build is tied to the exact commit it came from
  and checked before it's offered to players.
- **Player counts.** Add a few lines of code ([Slop Pulse](#slop-pulse-player-counts)) and your
  store page shows how many people are playing now and the peak this month.
- *(coming)* **Not Slop / Slop ratings** from people who actually launched your app.

## What Slop asks of your project

### Required

1. **Open source.** A public Git repository (GitHub, GitLab, Bitbucket or self-hosted) with an
   [OSI-approved licence](https://opensource.org/licenses) in a `LICENSE` file. Slop detects the
   licence from your repository when you create your store page. The app can't depend on
   closed-source libraries (the source check notes NVIDIA's old Cg shader library, for example);
   the operating system's own libraries and graphics drivers are fine.
2. **Builds on your own release page, for the platforms you choose.** Slop stores no builds. On
   your store page you tick what your app runs on, **Windows**, **Linux** or both, and every
   release must then include exactly these files, attached to a release in your repository:

   | You tick | Every release needs |
   |---|---|
   | Windows | A **`.zip`** of the app's files, with the program to start inside it (e.g. `MyGame.exe`) |
   | Linux | A **`.tar.gz`** of the app's files, with the program to start inside it (e.g. `bin/my-game`), **plus** the packages you tick: |
   | &nbsp;&nbsp;Linux `.deb` | A **`.deb`** package (Debian, Ubuntu, Mint) |
   | &nbsp;&nbsp;Linux `.rpm` | An **`.rpm`** package (Fedora, RHEL, Rocky, Alma) |

   Linux needs at least one package: `.deb`, `.rpm` or both. The Slop app installs the `.zip` or
   `.tar.gz` into the player's library folder and starts the program you name; keep its
   executable permission in the `.tar.gz` (Slop also sets it). The `.deb`/`.rpm` are offered on
   your store page for players who prefer a system package. macOS and Android aren't offered yet.
   Slop Build makes all of these files for you.
   - **Where the files must be:** on GitHub, under
     `https://github.com/<owner>/<repo>/releases/download/<tag>/<file>`. On other hosts, on the same
     site as the repository.
3. **The commit the build came from.** Every release names the full 40-character commit SHA. Tags
   can move; commits can't.
4. **The rules.** No sexual content, no gambling or loot boxes, no pay-to-win, nothing for sale,
   no malware, no hidden tracking, nothing stolen, and no code from outside Slop: an app only
   runs code that came in its release or in a mod Slop approved, never code or mods a server
   sends. See [Online play and mods](#online-play-and-mods). Rate your app honestly (Everyone, 3+, 7+, 12+,
   16+ or 18+; 18+ apps are hidden from players in the UK's under-18 view).
5. **Local saves in Slop's save folder.** If your app keeps saves, settings or logs on the
   player's computer, keep them in the folder named by `SLOP_SAVE_DIR` when it's set. See
   [Saves and sandbox mode](#saves-and-sandbox-mode). Saves on your own server are fine too.
   Slop's source check reports where each release saves.
6. **A maker signature** on each build: a GitHub artifact attestation, made by your GitHub Actions
   build (Slop Build does this for you). It proves the file came from your repository at that
   commit, so someone who only stole your Slop password can't publish anything. *Test servers
   may not require it (`/v1/server` says); GitHub only makes attestations for private repositories
   on its Enterprise plan.*

### Recommended

- **Slop Pulse** in games, so Slop can show player counts. See
  [Slop Pulse](#slop-pulse-player-counts).
- **Slop Build**, so releases build, sign and submit themselves. See
  [Option A](#option-a-slop-build-recommended).

## Release stages

| Stage | What it means | Time limit |
|---|---|---|
| **Coming soon** | A store page with no build yet. Players can mark it Interested. | 12 months, then it must reach Beta |
| **Beta** | Playable builds, still changing. Players can rate it (marked "Rated in Beta") and donate. | 12 months, then it must be released |
| **Released** | Finished (and still updated) | none |

A Coming soon or Beta project with **no commits for 90 days** is labelled a dead project. It stays
visible but isn't recommended, and comes back with your next commit. Missing a stage deadline
takes the page out of the store until you reach the next stage; existing installs keep working.

## Step by step

### 1. Sign in

On the Slop website, choose **Sign in → Continue with GitHub**, then pick a display name and your
country. Slop never sees your password.

Optionally, **add a passkey** under Account → Sign-in & security: a quicker way to sign in that
nobody can phish. Nothing on Slop requires one.

### 2. Create the store page

Go to **Publish → New app** and fill in:

| Field | Notes |
|---|---|
| Name | Up to 80 characters |
| Store address | Your app's slug, e.g. `my-game` (lowercase letters, numbers and dashes; can't change later) |
| One-line summary | Up to 200 characters |
| Description | Markdown; no HTML or images |
| Type | Game or App |
| Stage | Coming soon, Beta or Released |
| Source repository | `https://github.com/<owner>/<repo>` (or your GitLab, Bitbucket or own server) |
| Licence | Detected from the repository; pick one or choose Other for any SPDX ID |
| Runs on | Windows and/or Linux; for Linux, also `.deb` and/or `.rpm` (see the table above) |
| Age rating | Everyone, 3+, 7+, 12+, 16+ or 18+ |
| Tags | Up to 10 |

It's saved as a draft that only you can see.

### Pictures and videos

Optional, but pages with them get far more clicks, and every app should have an **icon**.

Slop doesn't host pictures. They live **in your repository**, in a `.slop/` folder at the top
level, listed in `.slop/media.json`:

```
.slop/
  media.json
  icon.png
  thumbnail.png
  screenshots/
    title.png
    level-1.png
    trailer-poster.png
```

```json
{
  "icon": "icon.png",
  "thumbnail": "thumbnail.png",
  "gallery": [
    { "image": "screenshots/title.png", "caption": "The title screen" },
    { "image": "screenshots/level-1.png" },
    { "youtube": "https://www.youtube.com/watch?v=VIDEO_ID", "poster": "screenshots/trailer-poster.png", "caption": "Trailer" }
  ]
}
```

| Key | Notes |
|---|---|
| `icon` | Recommended. A **square** PNG (or JPEG or WebP), best at **512×512** (at least 256×256, at most 1024×1024, up to 4 MB). It stands for your app in players' libraries, on store tiles and next to its name on its page, shown as small as 32×32, so keep it simple and readable: a logo, a character or a symbol from the app, not a screenshot with text |
| `thumbnail` | Optional. Your tile in the store. Best at **920×430** (at least 460×215); other shapes are cropped from the middle |
| `gallery` | Optional. Up to **20** items, each either `image` (a screenshot) or `youtube` (a video link) |
| `image` | A picture at least 320×180 |
| `youtube` | A YouTube link (`watch?v=`, `youtu.be/`, `shorts/` or `embed/`) |
| `poster` | Optional, videos only: a picture shown until someone presses play. Without one, the page shows a plain card |
| `caption` | Optional, one line of up to 200 characters |

Rules:

- Paths are relative to `.slop/`, using letters, numbers, `.`, `_`, `-` and `/` (no spaces, no `..`).
- Pictures are PNG, JPEG, WebP or GIF, up to 8 MB each, and at most 8192 pixels on a side. The
  icon can't be a GIF, and is up to 4 MB.
- **No icon yet?** Your app still works and can go in the store: Slop shows a square crop from the
  middle of your thumbnail instead, or your app's first letter. **Edit store page** says "No icon
  yet" until you add one, and can make a square crop of one of your pictures for you to download
  and commit. To add an icon to an existing `media.json`, add the line `"icon": "icon.png",` and
  put the picture at `.slop/icon.png`.
- The icon must be your app's own: from its art, logo or a screenshot. One that looks like another
  maker's app is pointed out to the reviewer.
- Each image or video can appear once.
- The repository must be public on GitHub so players can see the pictures.
- Slop reads `.slop/media.json` when you press **Check my repository now** (under **Edit store
  page**), and again at every release's commit. It links to the files **at that commit**, so
  later pushes don't change what players see until Slop reads them again.
- **Every new set of pictures is reviewed** before it shows, and before your app first goes in the
  store. The previous set stays up meanwhile. No sexually explicit content, here or anywhere on
  Slop.
- You choose the **order** players see the gallery in on **Edit store page**. That isn't reviewed,
  and pictures added later go at the end. The order in `media.json` is only the starting order.

### 3. Submit a release

A release is one version of your app. On Slop it's a **GitHub release** on your repository with
your builds attached. Open your app's **Releases** page (from **Publish**, choose **Releases &
progress**) and use one of the options below.

#### Option A: pick a GitHub release (simplest)

1. On GitHub, open **Releases → Draft a new release**. Give it a tag such as `v1.0.0`, attach
   the files your store page asks for (see the table above), and publish it. Don't mark it as a pre-release if you want Slop to pick up later ones by itself.
2. On Slop's Releases page, under **Submit a release**, choose that release. Slop reads the
   version from the tag (`v1.0.0` → `1.0.0`) and finds the commit the tag points at.
3. Tick one file for each platform. Slop matches them by type (`.zip` → Windows, `.tar.gz` →
   Linux, `.deb`, `.rpm`) and tells you if one is missing. For the `.zip` and `.tar.gz`, enter
   the program to start, as its path inside the archive.
4. Choose **Send version …**.

**Later versions arrive by themselves.** With **Pick up new GitHub releases** on (the default, in
the **Updates** box), Slop checks your repository every 10 minutes if you've released something
in the last week, and less often (up to every two hours) for quieter apps. For pickup within a
minute, install **Slop's GitHub App** on the repository (the link is in the **Updates** box): GitHub
then tells Slop the moment you publish a release. The App only reads releases and code. When you publish a new
release (not a draft or pre-release), Slop submits it using your previous release as the pattern:
the same platforms and programs, with files whose names are the old ones with the new version in
place of the old (`game-1.0.0-windows.zip` → `game-1.1.0-windows.zip`), or exactly the same
names. If a file is missing, Slop waits up to two hours for it to be uploaded, then shows what it
couldn't find so you can pick the release by hand.

#### Option B: Slop Build

Slop Build is a shared GitHub Actions workflow. Add one file to your repository and every push to
your main branch (or every `v*` tag) builds your app for Windows and Linux, packs it (a `.zip`
for Windows; a `.tar.gz`, `.deb` and `.rpm` for Linux), signs every file, attaches them to a release on your repository, and sends the release to Slop. No keys
or secrets.

On the Releases page, choose **Add to my repository on GitHub**: it opens GitHub with the file
filled in, and you click **Commit**. The file looks like this:

```yaml
# .github/workflows/slop.yml
name: Slop Build
on:
  push:
    branches: [main]
    tags: ["v*"]
  workflow_dispatch:

permissions:
  contents: write
  id-token: write
  attestations: write

jobs:
  slop:
    uses: DocFoxHound/slop-build/.github/workflows/build.yml@v1
    with:
      app: my-game                # your store address
      slop-url: https://slop.example   # the Slop site
      project-type: rust          # rust, go or custom
```

| Input | Meaning |
|---|---|
| `app` | Your app's store address on Slop (required) |
| `slop-url` | The Slop site (required) |
| `project-type` | What to build (required): see **Project types** below |
| `project-path` | Where the project is in the repository (default: the top) |
| `platforms` | Comma-separated, from `windows,linux` (default: both). Match your store page. |
| `linux-packages` | Comma-separated, from `deb,rpm` (default: both). Slop uses the ones your store page lists. Packages install the app in `/opt/<app>` with `/usr/bin/<app>` starting it. |
| `build-command` | `custom`: the command that builds your app (run with bash). `web`, `electron`, `tauri`: an extra step before building, e.g. `npm run build` |
| `output-dir` | `custom`: the folder the build produces; it's packed as-is. `web`: the folder with your built `index.html` |
| `exec` | The program to start, relative to the build; `{ext}` becomes `.exe` on Windows. Needed for `custom`, worked out for every other type |
| `godot-version` | `godot`: your Godot version, e.g. `4.3` (default) or `4.4.1`. The standard build, not .NET |
| `godot-dotnet` | `godot`: `true` for C# projects |
| `dotnet-version`, `dotnet-project` | `dotnet` (and C# Godot): the .NET SDK (default `8.0.x`) and, if there's more than one, the program project to ship |
| `godot-preset-windows`, `godot-preset-linux` | `godot`: the export presets to use (default `Windows Desktop` and `Linux`) |
| `python-version`, `python-entry`, `python-windowed` | `python`: Python version (default 3.12), the script that starts the app (default `main.py`), and `true` for apps with their own window, such as pygame games |
| `node-version` | `electron`, `tauri`, `web`: Node.js version (default 22) |
| `linux-runtime` | `steam-sniper` builds the Linux program inside Valve's Steam Runtime 3 (sniper), so it runs on older Linux systems too (anything with glibc 2.31, e.g. Ubuntu 20.04 or Debian 11). For `rust`, `go` and `custom`; Rust is installed in the container, following your `rust-toolchain.toml`. The container is a fixed version Slop tested, so builds don't change unexpectedly |
| `publish` | `false` builds and checks only: no release, nothing sent to Slop, the files kept as the run's artifacts for a day. For trying it out |
| `sign` | `true` by default; set `false` for private repositories without GitHub Enterprise |
| `itch-target` | Optional: also publish each build to your itch.io page, as `user/game` (from `user.itch.io/game`). Windows and Linux go to the `windows` and `linux` channels with itch's own tool. Needs the `BUTLER_API_KEY` secret (itch.io → Settings → API keys). Slop stays your store page and checks; itch is extra reach |
| `flatpak-app-id` | Optional: a Flatpak ID such as `io.github.you.Game`. Each release then carries a draft Flatpak manifest for the Linux build, for you to finish (permissions, metainfo, screenshots) and submit to Flathub |
| `signpath-organization-id` | Optional: sign the Windows program for free with SignPath Foundation (see [Signing](#signing)). Needs the `SIGNPATH_API_TOKEN` secret |
| `signpath-project-slug` | Your SignPath project (default: the app's store address) |
| `signpath-signing-policy-slug` | Your SignPath signing policy (default: `release-signing`) |

Every Slop Build also checks your Windows build with **Microsoft Defender** (today's definitions,
on GitHub's Windows runner), looks for secrets committed to your source with
[gitleaks](https://github.com/gitleaks/gitleaks), and lists dependencies with known
vulnerabilities with [OSV-Scanner](https://github.com/google/osv-scanner). Only you and Slop's
reviewers see the source checks; they never stop a release.

**Project types.** The **Add to my repository** button picks one by looking at your repository.

| `project-type` | What it does |
|---|---|
| `rust` | `cargo build --release --locked`; ships the first binary |
| `go` | `go build`; the program is named after the repository |
| `godot` | Installs Godot (`godot-version`) and its export templates, imports the project, and exports your presets named `Windows Desktop` and `Linux`. Commit `export_presets.cfg`. For C# projects add `godot-dotnet: true` (Godot's .NET build) |
| `unity` | Unity through [GameCI](https://game.ci): the editor version from `ProjectSettings/ProjectVersion.txt`, Windows and Linux players. Needs your Unity licence in your repository's secrets: see [Unity](#unity) |
| `dotnet` | `dotnet publish`, self-contained (players don't need .NET), for console, MonoGame, Avalonia and similar apps. Finds the program project, or set `dotnet-project` |
| `python` | PyInstaller, as a folder: installs `requirements.txt`, then builds `python-entry` |
| `electron` | `npm ci`, then electron-builder as an unpacked folder (no installer: Slop installs and updates it) |
| `tauri` | `npm ci` if there's a `package.json`, then `tauri build --no-bundle`; ships the program |
| `web` | An HTML5 game or web app: builds it (`build-command`), then puts `output-dir` in a small window of its own (a Tauri shell), so it runs like any other app. Linux packages depend on WebKitGTK |
| `custom` | Any build: `build-command`, `output-dir` and `exec` |

For example, a Godot game:

```yaml
    with:
      app: my-godot-game
      slop-url: https://slop.example
      project-type: godot
      godot-version: "4.3"
```

A Python game: PyInstaller programs are often flagged by antivirus heuristics, so signing them
([Signing](#signing)) helps most there. Electron on Linux needs unprivileged user namespaces for
its sandbox; most distributions have them on.

### Unity

Slop Build builds Unity projects with [GameCI](https://game.ci) on your repository's own GitHub
Actions, so your Unity licence stays in your repository: Slop never sees it.

1. Get a licence file: follow [GameCI's activation guide](https://game.ci/docs/github/activation)
   (for Unity Personal, it gives you a `.ulf` file).
2. In your repository: Settings → Secrets and variables → Actions → New repository secret. Add
   `UNITY_LICENSE` (the `.ulf` file's contents), `UNITY_EMAIL` and `UNITY_PASSWORD`. Plus or Pro
   licences use `UNITY_SERIAL` instead of `UNITY_LICENSE`.
3. Add Slop Build from your Releases page; the file it fills in passes those secrets on.

Builds use the Mono scripting backend on Linux runners. Large projects take a while the first
time; the `Library` folder is cached after that. Commit with Git LFS if your assets need it.

### Games over 2 GiB

GitHub release files are limited to 2 GiB. Slop Build splits a bigger `.zip` or `.tar.gz` into
numbered parts (`game.zip.001`, `game.zip.002`…, up to 64) and signs each one; players' desktop
apps download them in order, check every part, and join them back into the file Slop checked.
`.deb` and `.rpm` packages can't be split, so turn `linux-packages` off for a game that big.
Releases picked from GitHub (Option A) or entered by hand don't take parts yet: use Slop Build.

**Launch settings (`slop.toml`).** Optional, at the top of your repository (or the project's
folder). It says which program to start and with what arguments, for every platform or per
platform. A workflow's own `exec` wins over it.

```toml
[launch]
exec = "game{ext}"          # relative to the build; {ext} is .exe on Windows
args = ["--fullscreen"]

[launch.windows]
args = ["--fullscreen", "--renderer=d3d12"]
```

**Shared runtimes.** If your Windows build needs a runtime players may not have, name it and the
Slop app installs it before the first start (once the player agrees), straight from Microsoft,
after Windows confirms the download is signed by Microsoft. Don't ship these installers in your
build. With Slop Build, list them in `slop.toml`; otherwise tick them on the release form.

```toml
runtimes = ["vcredist-x64"]  # at the top, before [launch]
```

| ID | Runtime |
|---|---|
| `vcredist-x64` | Microsoft Visual C++ Redistributable (64-bit) |
| `vcredist-x86` | Microsoft Visual C++ Redistributable (32-bit) |
| `dotnet-desktop-8` | .NET Desktop Runtime 8 |
| `dotnet-desktop-9` | .NET Desktop Runtime 9 |
| `directx-9` | DirectX End-User Runtime (DirectX 9 extras, e.g. `d3dx9_43.dll`) |

Leaving `runtimes` out keeps the last release's list; `runtimes = []` clears it. Need one that
isn't listed? Ask us.

**Build now.** Every push to your main branch builds. To build without pushing, open the
workflow on GitHub (the **Build now** button on your Releases page) and press **Run workflow**.

Builds made by Slop Build are labelled **Built by Slop Build from source**.

#### Option C: build and submit yourself

1. Build each platform and pack it as your store page asks (Windows `.zip`; Linux `.tar.gz` plus
   `.deb`/`.rpm`).
2. Sign the files in GitHub Actions with
   [`actions/attest-build-provenance`](https://github.com/actions/attest-build-provenance):

   ```yaml
   permissions:
     contents: write
     id-token: write
     attestations: write
   steps:
     # … build your app into dist/ …
     - uses: actions/attest-build-provenance@v2
       with:
         subject-path: dist/*
   ```

3. Attach the files to a release on your repository.
4. On Slop's Releases page, pick the release (Option A), or open **Enter a release by hand** and
   fill in the version (e.g. `1.0.0`), the full commit SHA, and for each platform the file's URL
   on your release page, its type, and for zips the program to start (and optional arguments).
   Repositories that aren't on GitHub always use the form.
5. **Windows builds:** add [Slop Check](#slop-check) to your repository so Microsoft Defender
   checks them. Slop Build does this for you.

### 4. What happens next

Your app's **Releases** page shows a **Publishing progress** checklist, and refreshes itself while
anything is under way, so there's no need to reload it:

1. **Store page:** saved.
2. **Release:** received.
3. **Files checked:** Slop downloads each file once, records its size and SHA-256, and keeps
   nothing else. If a file can't be downloaded, the release fails with the reason. Files are
   checked one at a time (and releases one at a time, oldest first), so the checklist shows each
   file: done (passed, a note, or on hold), being checked ("Checking file 2 of 4: Linux (.deb) —
   Scanning for malware"), or waiting, with how long files take on Slop and about how long is
   left. Scanning takes a few minutes per file, longer for big programs (up to 30 minutes), so a
   release with four builds can take 20 minutes or more. Your AI agent sees the same in
   `list_releases` (`progress.summary`).
4. **Maker signature:** the attestation is checked: it must name the file's hash and come from
   your repository at the release's commit.
5. **Scan:** Slop scans every file with ClamAV, YARA (YARA-Forge's rules) and capa, unpacking
   archives first. Nothing is ever run. Windows builds also need a clean check from
   **Microsoft Defender** (Slop Build or [Slop Check](#slop-check)). If something is found, the
   release waits for a person: scanners make mistakes, and a reviewer can clear a false positive
   (the reason is shown to you). A scan says what was found, not that a program is safe. Each file
   also has a **Look up on VirusTotal** link for its hash, an optional second opinion: it's not
   required, and uploading a file there shares it with VirusTotal's subscribers, so don't for a
   private app.
6. **Source check:** Slop reads your repository at the release's commit once (on GitHub, GitLab
   or Codeberg; a private repository outside GitHub needs a read-only token saved on the app) and
   notes three things for you and the reviewer: whether your licence file
   matches the store page's licence; anything that looks like selling in the app (payment
   libraries, in-app purchase APIs, checkout links, loot boxes, premium currencies), which Slop
   doesn't allow; code that loads code from outside Slop, updates itself, or uses an unsafe data
   format (see [Online play and mods](#online-play-and-mods)); and the packages in your lockfiles. Notes don't stop a release by themselves, but
   an update with a new one waits for a person. After that, Slop checks those packages against
   [OSV](https://osv.dev) every day and tells you when a new security advisory affects your live
   release.
7. **Review by a person:** a person at Slop reviews your app's first release, usually within a
   day or two. Approval signs it; a rejection comes with a reason, and **To fix** on the Releases
   page (or `get_fixes` for your AI agent) says what to change. Either way you get a notification
   (under 🔔, and by email).
8. **In the store:** Slop staff put the app in the store.

**Updates.** With **Approve updates automatically** on (the default), every later release that
passes every check is approved and signed by Slop straight away, with no wait for a person.
Combined with **Pick up new GitHub releases** (or Slop Build), publishing an update is just
publishing a release on GitHub. Players' apps show a notice and update when the player chooses.
A person looks again if a release fails a check, or if one of your releases was withdrawn (by
Slop or by you) or rejected since the last one a person approved. Slop can withdraw a release that turns out to be
harmful; it then stops launching for everyone.

**Where things stand:** **Publish** lists each of your apps with the step it's waiting on or
stuck at. **Releases & progress** has the full checklist and every release's history.

### 5. Managing your app

Each app has a **Manage** page (Publish → **Manage**):

- **Not reviewed yet.** Releases Slop is still checking or that wait for review. Slop tells you
  about every release it receives; if you didn't send one, or sent it by mistake, **Withdraw** it
  there. It never reaches players, and your next release is reviewed by a person. If you didn't
  send it, also secure your accounts and repository.
- **What players get.** By default players get the newest approved release. You can instead hold
  them at any approved release, to roll back a bad update or to wait before sending a new one out,
  and switch back later. New releases are still checked and approved while you hold; they just
  don't go out until you switch.
- **Repository.** Before anyone has your app you can change it freely. Once players have it, a
  move to another repository is always checked by a person, and the first release from the new
  repository is reviewed by a person too. Players who installed from the old repository reinstall
  before their Slop app takes updates from the new one.
- **Updates.** The same **Pick up new GitHub releases** and **Approve updates automatically**
  settings as on Releases & progress.
- **Requests.** Every change you asked for, with its decision and any reason. You can cancel
  one that's still waiting.

Changes that would have players download something different are checked before they happen.
Switching to another approved release follows the same rule as updates: it goes through straight
away when automatic approval applies (the release already passed every check), otherwise a person
looks. Everything else, like the store page, platforms and settings, changes straight away.

## Donations

Players can donate to you. Donations are optional, and they never unlock anything: no extra
content, no perks, nothing in the app changes for someone who donated (and your app can't tell
who did).

- **Where to turn them on:** for each app, the **Get paid** section at the top of its **Edit store
  page** (Publish → your app → Edit page; the same settings are under Manage → Donations), or right
  on your app's store page, where the Donate button's panel shows you what's missing. The first
  time, that starts **Stripe's own sign-up**: you create (or connect) your Stripe account on
  Stripe's site, and donations go straight to it. Until you've set it up, Account → Money points there too, and
  Publish shows **Donations: off · Set up** on each released or beta app.
- **Who sees the Donate button:** everyone, next to Play / Install on every Beta or released
  app's page and in their library. While donations are off it's quiet and says why ("The maker
  hasn't turned on donations", say), so players know it isn't hidden.
- **18 or older:** receiving donations is for adults; you confirm it when you sign up.
- **Slop's fee is 5%** of each donation (VAT included where there is VAT). Donors pay the card
  processing on top, so you receive what they chose less 5%. Account → Money has your monthly fee
  statements.
- Donations may be taxable income for you; Slop doesn't give tax advice.
- **Your AI agent can't do this for you:** signing up with Stripe and changing where money goes
  are for you alone.

## Online play and mods

Slop checks your app's code, but it can't check the servers your app talks to, and those can
change at any time. A game that runs whatever a server sends would let anyone who runs (or
breaks into) a server take over players' computers. So these are rules, and Slop's source check
looks for code that breaks them. What it finds goes to a reviewer as a note: a note isn't a
rejection, but a real break is.

**Agents:** copy this section into the project's own instructions (`AGENTS.md`, `CLAUDE.md` or
similar) and follow it whenever you write networking, save files or mod support.

### What a game may get from a server

| What | Allowed? |
|---|---|
| Game data your code reads: positions, chat, match state, scores | Yes |
| Content your code reads: maps, levels, skins, pictures, sounds, settings | Yes, read safely (below) |
| Mods: anything that adds or changes code, scripts or rules | Only mods approved on Slop, which the player gets from Slop |
| Programs, libraries (`.dll`, `.so`), scripts, Godot `.pck` packs, Unity asset bundles with code | Never |
| A new version of the game itself | Never: updates come through Slop |

Community servers can't hand players mods either. A server can **name** the Slop mods it
needs, and the game asks the Slop app for them (see [Getting a server's mods](#getting-a-servers-mods)).

### Loading mods in your game

Players add mods to your game in the Slop app, from your game's page in their library, or when
your game asks for them. Mods **may contain code** your game loads: scripts, Godot `.pck` packs,
.NET assemblies, Unreal `.pak` files. Every mod on Slop is published, scanned and reviewed like
an app, so this is the one place your game may load code that didn't come in its own release.
Your game gets mods one of two ways; pick one under **Mods** on Edit store page.

**1. Your game's own mods folder (use what your engine or mod loader expects).** Name the
folder your game already reads mods from, in its files (`mods`, `Content/Paks/~mods`) or in its
save folder, and how mods are laid out there: each in its own folder (named after the mod's
Slop address), or all mods' files straight in the folder (for games that read `.pak` or `.zip`
files there). Before every start, and when mods are added while the game runs, the Slop app
makes that folder hold **exactly the mods the player added from Slop**:

- their files are copied in, and files that changed are put back;
- anything else in the folder (a mod someone dropped in by hand) is moved to Slop's quarantine
  folder, and the player is told that mods must be published on Slop;
- in **sandbox mode** the folder is read-only, so the game can't put anything there itself.

Your game loads whatever is in the folder, its usual way. Don't download into it or anywhere
else.

**2. `SLOP_MODS`.** When it starts your game, the Slop app sets `SLOP_MODS`: the folders of the
mods the player added, separated like `PATH` (`;` on Windows, `:` elsewhere), each holding one
mod's files exactly as Slop checked them. If it isn't set, no mods were added.

```gdscript
# Godot
var mods := OS.get_environment("SLOP_MODS")
var sep := ";" if OS.get_name() == "Windows" else ":"
for dir in mods.split(sep, false):
    for file in DirAccess.get_files_at(dir):
        if file.ends_with(".pck"):
            ProjectSettings.load_resource_pack(dir.path_join(file))
```

Either way:

- Load mods only from there. Never load mods, packs or scripts a server or another player sent,
  or anything downloaded outside Slop.
- **Your game loads its mods itself.** Injectors that patch the game from outside (BepInEx,
  MelonLoader, Doorstop) aren't allowed.
- Read mod data as carefully as anything else (rules 3 to 5 below).
- Tell modders in your README which files and layout your game reads.

**Engines.** Godot has no mod system of its own: load `.pck` packs from the mods folder with
`ProjectSettings.load_resource_pack`, or use a mod loader that reads `mods/` next to the game
(name it as a folder in the game's files). Unity: load assemblies or asset bundles from the
mods folder. Unreal: name `Content/Paks/~mods` (or your game's paks folder), all files straight
in the folder.

### Getting a server's mods

When a server or lobby needs mods, your game asks the Slop app for them by Slop address (and
version, if you like). The Slop app shows the player the list, with what each mod is, whether it
contains code and whether it goes online, and the player says yes or no. On yes, it downloads
each from Slop, checks it against Slop's signature, adds it to your game, and tells your game.
Only approved mods for your game can be added; the player sees any that aren't.

When it starts your game, the Slop app sets **`SLOP_LINK`**: a folder for this run of your game,
with `requests/` and `answers/` in it. It works the same in sandbox mode and offline.

1. Write the request as `requests/<name>.json`. Write `<name>.tmp` first, then rename it, so Slop
   never reads half a file. `<name>` is letters, digits, `-` and `_`:
   `{"mods": [{"slug": "dragons", "version": "1.2.0"}], "for": "Castle Rock server"}`. `for` is
   shown to the player.
2. Every second or two, read `answers/<name>.json`. It says `{"status": "waiting"}` until the
   player answers, then:
   - `done`: `mods` says for each `added`, `already_added`, `version_differs` (another version
     was added) or `unavailable`, and `folders` lists all its mod folders now, as in
     `SLOP_MODS`. With your own mods folder, they're already in it.
   - `declined`: the player said no.
   - `failed`: `message` says why (a bad request, or one already waiting for the player).

Then load the new mods (or ask the player to restart). Servers should list mods by Slop address
and version so every player has the same files. If `SLOP_LINK` isn't set, the game wasn't started
from Slop: tell the player which mods to add in their Slop library.

```gdscript
# Godot: ask for a server's mods, then wait for the player
var link := OS.get_environment("SLOP_LINK")   # empty when not started from Slop

func request_mods(names: Array, server: String) -> void:
    var name := "r%d" % Time.get_ticks_msec()
    var tmp := link.path_join("requests/%s.tmp" % name)
    var f := FileAccess.open(tmp, FileAccess.WRITE)
    f.store_string(JSON.stringify({"mods": names.map(func(n): return {"slug": n}), "for": server}))
    f.close()
    DirAccess.rename_absolute(tmp, link.path_join("requests/%s.json" % name))
    var answer_path := link.path_join("answers/%s.json" % name)
    while true:
        await get_tree().create_timer(1.0).timeout
        if not FileAccess.file_exists(answer_path):
            continue
        var answer = JSON.parse_string(FileAccess.get_file_as_string(answer_path))
        if answer and answer["status"] != "waiting":
            mods_answered(answer)   # your code: load the new mods, or say they were declined
            return
```

### Publishing a mod

Anyone can publish a mod for any app on Slop: on the app's page, under **Community**, choose
**Publish a mod**. A mod lives in your own repository, and each release is **one `.zip` of the
mod's files for every platform**, with no program to start (platform `any` when submitting by
hand). Slop checks and reviews it like an app, and lists it in the Community area of the app's
page. Players add it to the installed game in the Slop app, which hands it to the game when it
starts (see [Loading mods in your game](#loading-mods-in-your-game)). Say in your mod's
description which version of the game it's for.

A mod **may contain code**. Then the source check marks it **mod with code** and a reviewer
reads all of it. A mod with networking code is marked **goes online**: that can still be
approved, but players see it, and sandbox mode is suggested for the game it's added to. Say in
your description what the code does and what it connects to. A mod may only add to its game:
no program to start, no code from a server, no updating itself, no loading other mods.

### Publishing a community build

A community build (a modded version, port, fix or translation) is built from **your own fork**
of an app on Slop: on the app's page, under **Community**, choose **Register a branch from your
fork**. It's held to the same rules as a new app. The form asks what creating an app asks: its
licence, platforms, age rating and content labels, what it connects to online, and whether it
works in sandbox mode. Every release is scanned, its source checked and a person reviews it.
It starts from the original's answers. It **can't be rated below the original** and keeps the
original's content labels; add to them if your build adds stronger content.

Players install it beside the original and can pick it from the list on the original's
**Play** button. It's an app of its own, so it gets **its own save folder** (`SLOP_SAVE_DIR`
names it). Keep the original's save format if you can: players are offered to copy their saves
across when they pick your build.

### Rules for the code

1. **Never run code from the network or from downloaded files.** (Mods from Slop are the
   exception: see [Loading mods in your game](#loading-mods-in-your-game).) No `eval`, `exec`,
   `loadstring`, `new Function`, `Assembly.Load` on downloaded bytes, Java class loaders,
   `dlopen`/`LoadLibrary` on downloaded files, Godot `load_resource_pack` or setting
   `GDScript.source_code`, or Unity `AssetBundle` loads from a server.
2. **Never update yourself.** No Electron `autoUpdater`, Squirrel, Velopack, NetSparkle,
   `self_update` or similar. Slop delivers updates.
3. **Use data formats that can't rebuild objects.** For anything from a server, another player,
   or a file a player could have been sent (saves, levels), use JSON, MessagePack, protobuf,
   FlatBuffers or your own plain format. Never use .NET `BinaryFormatter` (or `SoapFormatter`,
   `NetDataContractSerializer`, Json.NET with `TypeNameHandling` other than `None`), Python
   `pickle`/`marshal`/`yaml.load` (use `yaml.safe_load`), Java `ObjectInputStream`, Godot
   `bytes_to_var_with_objects`, `get_var(true)` or `str_to_var`, or Lua `loadstring` as a
   deserializer.
4. **Treat every message as hostile.** Check sizes and counts before allocating, check every
   index and enum, and drop messages that don't parse. Don't trust a client's claims: the server
   decides what happens in the game (where players are, what they hit, what they own).
5. **Keep downloaded content in its place.** Write only inside the game's own data folder.
   Reject archive entries with `..`, absolute paths or links. Never use a name from the network
   as a file path without checking it.
6. **Never start programs from what a server says.** No shell commands or opening URLs a server
   sends without the player seeing and confirming the address.
7. **Encrypt connections** to your own servers (TLS or the engine's encrypted transport), so
   nobody between the player and the server can change what's sent.
8. **Say what your app connects to.** Under **Online** on your store page, tick what it connects
   to (your servers or database, servers players host, direct connections between players,
   other online services) and name your servers. Slop Pulse doesn't count.

### What Slop does

- The **source check** reads your code at each release's commit and notes the patterns above
  (code loading, self-updaters, unsafe data formats). A reviewer looks at each. An update with a
  new note waits for a person.
- Anyone can report an app that **runs code from outside Slop**; reports are reviewed within
  24 hours.
- The source check also notes **networking code**. If your code goes online but your store
  page doesn't say so, that's a note, and an update waits for a person.
- Store pages of apps that go online tell players that Slop can't check the servers, and
  suggest running the app in **sandbox mode** in the Slop app; see
  [Saves and sandbox mode](#saves-and-sandbox-mode).

## Saves and sandbox mode

Players can run any app in **sandbox mode** in the Slop app: the app can use only its own
files, mods and **save folder**, plus the screen, sound, graphics card and controllers. Slop
suggests it for apps that go online. So players can switch between sandbox and normal mode
without losing anything, **local saves must live in Slop's save folder**.

When the Slop app starts your app, it sets **`SLOP_SAVE_DIR`**: a folder that belongs to your
app, the same in both modes, outside the installed files (updates never touch it), and kept when
the player uninstalls.

**Where it is.** Each app has its own, named after its store address:

| | The save folder for `your-app` |
|---|---|
| Linux | `~/.local/share/dev.ironpoint.slop/library/saves/your-app/` |
| Windows | `%APPDATA%\dev.ironpoint.slop\library\saves\your-app\` |

(On Linux, `~/.local/share` is `$XDG_DATA_HOME` when that's set.) Don't write these paths into
your code: **read `SLOP_SAVE_DIR`**, which is always right. Organise the folder however you like
(`slot1.json`, `settings.cfg`, `logs/`…). A community build of your app has its own address, so
it gets its own folder.

1. **If your app keeps anything on the player's computer** (saves, settings, logs,
   screenshots), and `SLOP_SAVE_DIR` is set, keep it there and nowhere else. When it isn't set,
   your app is running outside Slop: use your usual place.
2. **Saves on your own server are fine,** and often better. This rule only covers what's
   written on the player's computer.
3. **Saves are data, never code.** Plain formats only (JSON, MessagePack, your own format), read
   carefully, as in rules 3 and 5 under [Rules for the code](#rules-for-the-code). Never put
   programs, libraries, scripts or packs in the save folder. Slop moves programs and libraries
   out of it before every start, and takes execute permission off its files.
4. Don't write anywhere else: not the home folder, Documents, the desktop, or `%APPDATA%` by
   name. In sandbox mode those don't exist, and the app will fail or lose its saves.
5. Keep caches out of the save folder. On Linux, `XDG_CACHE_HOME` points to a separate cache
   folder; elsewhere, a `cache` subfolder you're happy to lose is fine.

**Engines that need no change:** the Slop app points the usual per-user folders inside the save
folder, so Godot's `user://` works as it is on Linux and Windows, and so do most engines on Linux
(they follow `XDG_DATA_HOME` and `XDG_CONFIG_HOME`). **Unity on Windows, .NET's
`Environment.SpecialFolder`, Electron's `app.getPath` and anything that asks Windows for a
known folder don't follow it:** read `SLOP_SAVE_DIR` instead.

```csharp
// Unity / .NET
var dir = Environment.GetEnvironmentVariable("SLOP_SAVE_DIR");
var saves = string.IsNullOrEmpty(dir) ? Application.persistentDataPath : dir;
```

```js
// Electron: before the app is ready
if (process.env.SLOP_SAVE_DIR) app.setPath("userData", process.env.SLOP_SAVE_DIR);
```

```python
saves = os.environ.get("SLOP_SAVE_DIR") or platformdirs.user_data_dir("my-game")
```

Also in sandbox mode: no other programs' files, and the network unless the player blocked it
(offered only when your store page says the app doesn't go online). On Linux there's no session
bus (no system tray, notifications or keyring). On Windows the app runs in an **AppContainer**,
the sandbox Store apps use: Windows' known folders (`FOLDERID_RoamingAppData` and the like) point
into the container, not the save folder, so read `SLOP_SAVE_DIR`; temporary files (`TEMP`) go
to a cache folder; and it can't log the player off or switch desktops. If your app follows the rules above, tick **Works in sandbox mode**
on its store page.

**What Slop checks.** Every release's source check says where it saves, on your Releases page
and to the reviewer:

| The check shows | Means |
|---|---|
| Saves in Slop's save folder | The code reads `SLOP_SAVE_DIR`, or uses Godot's `user://` |
| No local saves found | Neither, and no writes to the player's folders: fine if it saves nothing locally or only on your server |
| Saves outside Slop's save folder | It writes to fixed per-user folders (Documents, `%APPDATA%`, the home folder, Unity's `persistentDataPath`, Electron's `userData`…), listed with file and line |

Saving outside the folder is a note asking you to move them, not a rejection. But while your
newest release does, players aren't told your app works in sandbox mode, even if you ticked it,
and the reviewer asks you to fix it or untick it.

## Slop in your code

Ready-made code for the three things a game does with Slop besides Pulse: **saves** in
`SLOP_SAVE_DIR`, **mods** from your own mods folder and `SLOP_MODS`, and **asking for a server's
mods** through `SLOP_LINK`. Each works unchanged outside the Slop app (your own save folder, no
mods, `no_link`). Slop's tests run every one of them against the real Slop app code, so copy
them as they are. For Godot, use the GDScript in [Loading mods in your game](#loading-mods-in-your-game)
and [Getting a server's mods](#getting-a-servers-mods).

| Language | Saves | Mods | Server's mods |
|---|---|---|---|
| Python | `save_json`, `load_json` | `mod_dirs` | `request_mods` |
| Rust (with `serde_json`) | `save`, `load` | `mod_dirs` | `request_mods` (blocks: use a thread) |
| C# (Unity, .NET, Godot C#) | `Slop.Save`, `Slop.Load` | `Slop.ModDirs` | `await Slop.RequestMods` |
| C++17 (Unreal, your own engine) | `slop::save`, `slop::load` | `slop::mod_dirs` | `slop::request_mods` (blocks: use a thread) |

Pass your usual save folder as the fallback (in Unity, `Application.persistentDataPath`; in
Unreal, `FPaths::ProjectSavedDir()`), and your mods folder's path if you named one on your
store page.

**Python** (`slop.py`)

```python
# Slop for Python games: saves, mods and asking the Slop app for a server's mods.
# Standard library only. Outside the Slop app it falls back: your own save folder, no mods,
# and request_mods answers "no_link".
import json, os, time

def save_dir(fallback):
    """Where saves, settings and logs go: Slop's save folder, else `fallback`."""
    path = os.environ.get("SLOP_SAVE_DIR") or fallback
    os.makedirs(path, exist_ok=True)
    return path

def save_json(fallback, name, data):
    """Saves safely: a new file first, renamed over the old one."""
    path = os.path.join(save_dir(fallback), name)
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        json.dump(data, f)
    os.replace(path + ".tmp", path)

def load_json(fallback, name):
    """A save, or {} when it's missing, too big or not a JSON object."""
    path = os.path.join(save_dir(fallback), name)
    try:
        if os.path.getsize(path) > 1024 * 1024:
            return {}
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}

def mod_dirs(own_folder=None):
    """Every mod folder: your game's own mods folder (if you named one on your store page),
    then SLOP_MODS. Each holds one mod from Slop."""
    found, seen = [], set()
    if own_folder and os.path.isdir(own_folder):
        for name in sorted(os.listdir(own_folder)):
            if os.path.isdir(os.path.join(own_folder, name)):
                found.append(os.path.join(own_folder, name))
                seen.add(name)
    for d in filter(None, os.environ.get("SLOP_MODS", "").split(os.pathsep)):
        address = os.path.basename(os.path.dirname(d)) if os.path.basename(d) == "files" else os.path.basename(d)
        if address not in seen:
            seen.add(address)
            found.append(d)
    return found

def request_mods(mods, for_whom, give_up_after=600):
    """Asks the Slop app for mods by address, e.g. [{"slug": "dragons", "version": "1.2.0"}].
    The player confirms. Returns the answer: "done" (with "mods" and "folders"), "declined",
    "failed" (with "message"), "no_link" or "timeout"."""
    link = os.environ.get("SLOP_LINK")
    if not link:
        return {"status": "no_link"}
    name = "r%d" % time.time_ns()
    tmp = os.path.join(link, "requests", name + ".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"mods": mods, "for": for_whom}, f)
    os.replace(tmp, os.path.join(link, "requests", name + ".json"))
    answer_path = os.path.join(link, "answers", name + ".json")
    until = time.monotonic() + give_up_after
    while time.monotonic() < until:
        time.sleep(0.5)
        try:
            with open(answer_path, encoding="utf-8") as f:
                answer = json.load(f)
        except (OSError, ValueError):
            continue
        if answer.get("status") != "waiting":
            return answer
    return {"status": "timeout"}
```

**Rust** (`slop.rs`)

```rust
//! Slop for Rust games: saves, mods and asking the Slop app for a server's mods. Needs
//! `serde_json`. Outside the Slop app it falls back: your own save folder, no mods, and
//! `request_mods` answers "no_link".
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Where saves, settings and logs go: Slop's save folder, else `fallback`.
pub fn save_dir(fallback: &Path) -> PathBuf {
    env::var_os("SLOP_SAVE_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(|| fallback.to_path_buf(), PathBuf::from)
}

/// Saves safely: a new file first, renamed over the old one.
pub fn save(fallback: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let dir = save_dir(fallback);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, dir.join(name))
}

/// A save, or `None` when it's missing or over 1 MiB.
pub fn load(fallback: &Path, name: &str) -> Option<Vec<u8>> {
    let path = save_dir(fallback).join(name);
    (fs::metadata(&path).ok()?.len() <= 1024 * 1024).then(|| fs::read(&path).ok())?
}

/// Every mod folder: your game's own mods folder (if you named one on your store page), then
/// `SLOP_MODS`. Each holds one mod from Slop.
pub fn mod_dirs(own_folder: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    if let Some(own) = own_folder {
        let mut subs: Vec<PathBuf> = fs::read_dir(own)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        found.extend(subs);
    }
    let address = |p: &Path| -> Option<String> {
        let p = if p.file_name()? == "files" { p.parent()? } else { p };
        Some(p.file_name()?.to_string_lossy().into_owned())
    };
    for dir in env::split_paths(&env::var_os("SLOP_MODS").unwrap_or_default()) {
        if !dir.as_os_str().is_empty() && !found.iter().any(|f| address(f) == address(&dir)) {
            found.push(dir);
        }
    }
    found
}

/// Asks the Slop app for mods by address, e.g. `json!([{"slug": "dragons", "version": "1.2.0"}])`.
/// The player confirms. Returns the answer: "done" (with "mods" and "folders"), "declined",
/// "failed" (with "message"), "no_link" or "timeout". Blocks: call it off your main loop.
pub fn request_mods(mods: serde_json::Value, for_whom: &str, give_up_after: Duration) -> serde_json::Value {
    let Some(link) = env::var_os("SLOP_LINK").filter(|l| !l.is_empty()).map(PathBuf::from) else {
        return serde_json::json!({ "status": "no_link" });
    };
    let name = format!("r{}", now_ns());
    let tmp = link.join("requests").join(format!("{name}.tmp"));
    let body = serde_json::json!({ "mods": mods, "for": for_whom });
    if fs::write(&tmp, body.to_string())
        .and_then(|_| fs::rename(&tmp, link.join("requests").join(format!("{name}.json"))))
        .is_err()
    {
        return serde_json::json!({ "status": "failed", "message": "couldn't write the request" });
    }
    let answer_path = link.join("answers").join(format!("{name}.json"));
    let until = Instant::now() + give_up_after;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(500));
        let answer = fs::read(&answer_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if let Some(answer) = answer.filter(|a| a["status"] != "waiting") {
            return answer;
        }
    }
    serde_json::json!({ "status": "timeout" })
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}
```

**C#** (`Slop.cs`)

```csharp
// Slop for C# games (Unity, .NET, Godot C#): saves, mods and asking the Slop app for a server's
// mods. No packages needed. Outside the Slop app it falls back: your own save folder, no mods,
// and RequestMods answers "no_link".
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;

public static class Slop
{
    /// Where saves, settings and logs go: Slop's save folder, else `fallback`
    /// (in Unity, Application.persistentDataPath).
    public static string SaveDir(string fallback)
    {
        var dir = Environment.GetEnvironmentVariable("SLOP_SAVE_DIR");
        dir = string.IsNullOrEmpty(dir) ? fallback : dir;
        Directory.CreateDirectory(dir);
        return dir;
    }

    /// Saves safely: a new file first, then swapped in for the old one.
    public static void Save(string fallback, string name, string text)
    {
        var path = Path.Combine(SaveDir(fallback), name);
        File.WriteAllText(path + ".tmp", text);
        if (File.Exists(path)) File.Replace(path + ".tmp", path, null);
        else File.Move(path + ".tmp", path);
    }

    /// A save, or null when it's missing or over 1 MiB.
    public static string Load(string fallback, string name)
    {
        var file = new FileInfo(Path.Combine(SaveDir(fallback), name));
        return file.Exists && file.Length <= 1024 * 1024 ? File.ReadAllText(file.FullName) : null;
    }

    /// Every mod folder: your game's own mods folder (if you named one on your store page), then
    /// SLOP_MODS. Each holds one mod from Slop.
    public static List<string> ModDirs(string ownFolder = null)
    {
        var found = new List<string>();
        if (ownFolder != null && Directory.Exists(ownFolder))
            found.AddRange(Directory.GetDirectories(ownFolder).OrderBy(d => d, StringComparer.Ordinal));
        string Address(string d) =>
            Path.GetFileName(d) == "files" ? Path.GetFileName(Path.GetDirectoryName(d)) : Path.GetFileName(d);
        var list = Environment.GetEnvironmentVariable("SLOP_MODS") ?? "";
        foreach (var dir in list.Split(new[] { Path.PathSeparator }, StringSplitOptions.RemoveEmptyEntries))
            if (!found.Any(f => Address(f) == Address(dir))) found.Add(dir);
        return found;
    }

    /// Asks the Slop app for mods by address (and version, or null). The player confirms.
    /// Returns the answer's JSON; its "status" is "done", "declined", "failed", "no_link" or
    /// "timeout" (see Status). Await it; it doesn't block your game.
    public static async Task<string> RequestMods(IEnumerable<(string slug, string version)> mods, string forWhom,
        double giveUpAfterSeconds = 600)
    {
        var link = Environment.GetEnvironmentVariable("SLOP_LINK");
        if (string.IsNullOrEmpty(link)) return "{\"status\":\"no_link\"}";
        var name = "r" + DateTime.UtcNow.Ticks;
        var body = new StringBuilder("{\"mods\":[");
        body.Append(string.Join(",", mods.Select(m =>
            m.version == null ? $"{{\"slug\":{Quote(m.slug)}}}" : $"{{\"slug\":{Quote(m.slug)},\"version\":{Quote(m.version)}}}")));
        body.Append("],\"for\":").Append(Quote(forWhom)).Append('}');
        var tmp = Path.Combine(link, "requests", name + ".tmp");
        File.WriteAllText(tmp, body.ToString());
        File.Move(tmp, Path.Combine(link, "requests", name + ".json"));
        var answerPath = Path.Combine(link, "answers", name + ".json");
        var until = DateTime.UtcNow.AddSeconds(giveUpAfterSeconds);
        while (DateTime.UtcNow < until)
        {
            await Task.Delay(500);
            string answer;
            try { answer = File.ReadAllText(answerPath); } catch (IOException) { continue; }
            var status = Status(answer);
            if (status != null && status != "waiting") return answer;
        }
        return "{\"status\":\"timeout\"}";
    }

    /// An answer's "status". (Parse the rest with your JSON library: JsonUtility, System.Text.Json…)
    public static string Status(string answerJson)
    {
        var m = Regex.Match(answerJson ?? "", "\"status\"\\s*:\\s*\"([a-z_]+)\"");
        return m.Success ? m.Groups[1].Value : null;
    }

    static string Quote(string s)
    {
        var sb = new StringBuilder("\"");
        foreach (var c in s)
        {
            if (c == '"' || c == '\\') sb.Append('\\').Append(c);
            else if (c < ' ') sb.Append("\\u").Append(((int)c).ToString("x4"));
            else sb.Append(c);
        }
        return sb.Append('"').ToString();
    }
}
```

**C++17** (`slop.hpp`): in Unreal, standard C++ works in game code as it is.

```cpp
// Slop for C++ games (Unreal, or your own engine): saves, mods and asking the Slop app for a
// server's mods. C++17, standard library only. Outside the Slop app it falls back: your own
// save folder, no mods, and request_mods answers "no_link".
#pragma once
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <optional>
#include <sstream>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace slop {
namespace fs = std::filesystem;

inline std::string env(const char* name) {
    const char* v = std::getenv(name);
    return v ? v : "";
}

// Where saves, settings and logs go: Slop's save folder, else `fallback`.
inline fs::path save_dir(const fs::path& fallback) {
    std::string dir = env("SLOP_SAVE_DIR");
    fs::path path = dir.empty() ? fallback : fs::u8path(dir);
    fs::create_directories(path);
    return path;
}

// Saves safely: a new file first, renamed over the old one.
inline bool save(const fs::path& fallback, const std::string& name, const std::string& data) {
    fs::path path = save_dir(fallback) / name, tmp = path;
    tmp += ".tmp";
    {
        std::ofstream out(tmp, std::ios::binary);
        if (!(out << data)) return false;
    }
    std::error_code ec;
    fs::rename(tmp, path, ec);
    return !ec;
}

// A save, or nothing when it's missing or over 1 MiB.
inline std::optional<std::string> load(const fs::path& fallback, const std::string& name) {
    fs::path path = save_dir(fallback) / name;
    std::error_code ec;
    auto size = fs::file_size(path, ec);
    if (ec || size > 1024 * 1024) return std::nullopt;
    std::ifstream in(path, std::ios::binary);
    std::stringstream buf;
    buf << in.rdbuf();
    return buf.str();
}

// Every mod folder: your game's own mods folder (if you named one on your store page), then
// SLOP_MODS. Each holds one mod from Slop.
inline std::vector<fs::path> mod_dirs(const fs::path& own_folder = {}) {
    std::vector<fs::path> found;
    std::error_code ec;
    if (!own_folder.empty() && fs::is_directory(own_folder, ec)) {
        for (auto& e : fs::directory_iterator(own_folder, ec))
            if (e.is_directory()) found.push_back(e.path());
        std::sort(found.begin(), found.end());
    }
    auto address = [](const fs::path& p) {
        return p.filename() == "files" ? p.parent_path().filename() : p.filename();
    };
#ifdef _WIN32
    const char sep = ';';
#else
    const char sep = ':';
#endif
    std::stringstream list(env("SLOP_MODS"));
    for (std::string item; std::getline(list, item, sep);) {
        if (item.empty()) continue;
        fs::path dir = fs::u8path(item);
        bool known = false;
        for (auto& f : found) known = known || address(f) == address(dir);
        if (!known) found.push_back(dir);
    }
    return found;
}

inline std::string quote(const std::string& s) {
    std::string out = "\"";
    for (char c : s) {
        if (c == '"' || c == '\\') { out += '\\'; out += c; }
        else if (static_cast<unsigned char>(c) < 0x20) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); out += b; }
        else out += c;
    }
    return out + "\"";
}

// An answer's "status": done, declined, failed, waiting, no_link or timeout.
inline std::string status(const std::string& answer) {
    auto at = answer.find("\"status\"");
    if (at == std::string::npos) return "";
    auto start = answer.find('"', answer.find(':', at) + 1);
    auto end = answer.find('"', start + 1);
    return start == std::string::npos || end == std::string::npos ? "" : answer.substr(start + 1, end - start - 1);
}

// Asks the Slop app for mods by address and version ("" for any). The player confirms. Returns
// the answer's JSON (see status()). Blocks: call it on a worker thread.
inline std::string request_mods(const std::vector<std::pair<std::string, std::string>>& mods,
                                const std::string& for_whom, std::chrono::seconds give_up_after = std::chrono::seconds(600)) {
    std::string link = env("SLOP_LINK");
    if (link.empty()) return "{\"status\":\"no_link\"}";
    fs::path dir = fs::u8path(link);
    std::string name = "r" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    std::string body = "{\"mods\":[";
    for (size_t i = 0; i < mods.size(); ++i) {
        body += (i ? "," : "") + std::string("{\"slug\":") + quote(mods[i].first);
        if (!mods[i].second.empty()) body += ",\"version\":" + quote(mods[i].second);
        body += "}";
    }
    body += "],\"for\":" + quote(for_whom) + "}";
    fs::path tmp = dir / "requests" / (name + ".tmp");
    { std::ofstream out(tmp, std::ios::binary); out << body; }
    std::error_code ec;
    fs::rename(tmp, dir / "requests" / (name + ".json"), ec);
    if (ec) return "{\"status\":\"failed\",\"message\":\"couldn't write the request\"}";
    fs::path answer_path = dir / "answers" / (name + ".json");
    auto until = std::chrono::steady_clock::now() + give_up_after;
    while (std::chrono::steady_clock::now() < until) {
        std::this_thread::sleep_for(std::chrono::milliseconds(500));
        std::ifstream in(answer_path, std::ios::binary);
        if (!in) continue;
        std::stringstream buf;
        buf << in.rdbuf();
        std::string s = status(buf.str());
        if (!s.empty() && s != "waiting") return buf.str();
    }
    return "{\"status\":\"timeout\"}";
}

}  // namespace slop
```

## Browser demos for new games

For a new game's first 14 days on Slop, players can try it in their browser from its store page,
with nothing to install. Export a web build (Godot: the Web export; any engine whose build runs in
a browser), zip it with `index.html` at the top, attach it to a release on your repository, and
paste its address under **Browser demo** on your Manage page. Slop scans it and hosts it from a
separate address until the 14 days are up, then deletes it.

- Up to 100 MB zipped (250 MB unpacked).
- Demos can't go online: they reach only their own files. Show the part of the game that works
  offline.
- There are a limited number of demo slots, and a daily traffic limit; when it's reached,
  players are asked to install the game instead.

## Working with your AI agent

Your own AI coding agent can do most of this for you, from your editor: write and create the
store page, submit releases with notes, read Slop's checks of each release and fix what they find,
register community builds and mods, and offer fixes to the apps you build on. Creatives can let it
watch People jobs, send quotes and talk in job conversations.

1. **Account → AI agents → Make token.** Choose what it can do (look, or also publish), which
   apps, whether it handles People jobs, and how long it lasts (a day up to 12 months). A token
   that publishes needs the Publisher Agreement: the page shows it for you to accept when you
   make the token. If it changes later, your agent is told it's waiting, and the page shows it
   again.
2. **Set it up on your computer, never in an AI chat.** The token is a password for your Slop
   account: don't paste it into a chat with Claude, ChatGPT, Codex, Grok or any AI. The page
   walks you through it for Windows, macOS and Linux: save the token once in an environment
   variable, `SLOP_TOKEN`, in your own terminal window (not inside the AI chat, and not with
   Claude Code's `!`), then connect your tool with a reference to that variable, so the token
   itself never appears in a command or setting the AI sees. For example, Claude Code:
   `claude mcp add --transport http --scope user slop <MCP address> --header 'Authorization: Bearer ${SLOP_TOKEN}'`
   (keep the single quotes); Codex: `codex mcp add slop --url <MCP address> --bearer-token-env-var SLOP_TOKEN`.
   Other tools: the MCP address the page shows, with the header
   `Authorization: Bearer <your token>`, set in the tool's MCP or connector settings (or Slop's
   API with the same header).
3. **Ask it to read `maker_guide` first**, then give it the job: "publish this game on Slop",
   "fix what Slop found in my last release", "offer this fix to the original". No token is
   needed in the chat.

**When a token runs out:** the page shows how long each token has left. Slop tells you a week
and a day before one expires (and when it has), and so does your agent. **Make a replacement**
on the token fills in the same settings; set the new one up the same way (save it in
`SLOP_TOKEN` again: the commands that connect your tools stay the same), then remove the old
one. If a token ever ends up in a chat, remove it at once and make a new one.

The agent works in your session and your repository; Slop runs no AI for it. It can't pay,
accept agreements or offers, change where money goes, or change your account: those stay with
you. Releases it submits are checked and reviewed like any other. Remove a token any time
(Account → AI agents, or sign it out like a device).

**Fixes:** every problem Slop's checks find comes with exact instructions, on your Releases page
("Copy for your AI") and from the agent tool `get_fixes`.

## Keeping Windows from blocking your app

Players shouldn't see "Windows protected your PC" or have Defender delete your game. Here's what
Slop does, and the one optional step that helps most.

**What Slop does for you:**

- **No SmartScreen prompt.** Windows' "unrecognized app" prompt appears for files a browser
  downloaded (they're marked as coming from the internet). Slop's app downloads, checks and
  unpacks your files itself, like Steam, so that mark isn't added.
- **Defender checks your build before players get it.** Slop Build (or Slop Check) scans your
  Windows files with Microsoft Defender and today's definitions. If Defender flags them, the
  release waits for a person instead of reaching players and being quarantined there.
- **Slop's own scans** (ClamAV, YARA, capa) on every file, and a person on the first release.

**What helps most: sign your Windows program.** Defender and Windows' Smart App Control trust
signed programs far more than unsigned ones. Open-source projects can sign for free, see
[Signing](#signing).

**If Defender flags your program by mistake** (common for new, unsigned programs, especially
ones made with Python, AutoHotkey, or packers like UPX):

1. Submit the file at [Microsoft's submission page](https://www.microsoft.com/en-us/wdsi/filesubmission),
   choosing **Software developer** and **Incorrectly detected as malware**. Microsoft usually
   answers within a few days and updates its definitions for everyone. Only you can do this:
   Microsoft has no way for a store to submit for you.
2. Build again (or re-run Slop Check) once they've answered. You can also ask Slop's reviewer to
   clear it as a false positive in the meantime.
3. Don't pack your program with UPX or similar tools; that's the most common reason for false
   detections.

### Signing

[SignPath Foundation](https://signpath.org) signs open-source programs for free. Your program is
signed with SignPath Foundation's certificate, and SignPath checks each build came from your
repository on GitHub's runners, so nobody can sign anything else in your name.

1. **Apply** at [signpath.org/apply](https://signpath.org/apply). Your project needs an
   OSI-approved licence, no proprietary parts, and to be actively maintained and released.
   SignPath Foundation reviews applications by hand; it can take a few weeks.
2. **Once accepted,** in SignPath:
   - link your GitHub repository as a **trusted build system** for the project;
   - create a **CI user**, and add its API token to your repository's secrets as
     `SIGNPATH_API_TOKEN` (GitHub → Settings → Secrets and variables → Actions);
   - set the project's **artifact configuration** to sign your own program (Slop Build sends
     the build folder as a zip):

     ```xml
     <?xml version="1.0" encoding="utf-8"?>
     <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
       <zip-file>
         <pe-file path="my-game.exe">
           <authenticode-sign/>
         </pe-file>
       </zip-file>
     </artifact-configuration>
     ```

     Add a `<pe-file>` for each of your own `.exe` and `.dll` files. Don't sign other people's
     files (SignPath Foundation's rules); those keep their own signatures.
3. **In your Slop Build file,** uncomment the SignPath lines (the file Slop gives you has them):

   ```yaml
   jobs:
     slop:
       uses: DocFoxHound/slop-build/.github/workflows/build.yml@v1
       with:
         app: my-game
         slop-url: https://slop.example
         project-type: rust
         signpath-organization-id: 00000000-0000-0000-0000-000000000000
       secrets:
         SIGNPATH_API_TOKEN: ${{ secrets.SIGNPATH_API_TOKEN }}
   ```

If your SignPath policy asks for a person to approve each signature, approve it in SignPath while
the build waits (up to an hour). Your Releases page shows **SIGNED by …** once it works.

Building yourself instead? Use SignPath's
[GitHub Action](https://github.com/signpath/github-action-submit-signing-request) in your own
workflow, or any code-signing certificate you have.

### Signing builds you make yourself

Building somewhere other than GitHub Actions (GitLab, Bitbucket, your own server, your computer)?
Sign each file with an SSH key registered on your account instead:

1. Make an Ed25519 key if you don't have one: `ssh-keygen -t ed25519`.
2. Add its public half (the line in `~/.ssh/id_ed25519.pub`) under **Account → Signing keys**. It
   starts counting **72 hours later**, and you're told when a key is added, so nobody with a
   stolen session can quietly add one and sign a release in your name.
3. For each file, sign it and attach the `.sig` next to it on the release page:

   ```sh
   ssh-keygen -Y sign -f ~/.ssh/id_ed25519 -n slop-release my-game-1.0.0-windows.zip
   ```

Slop checks the signature when it fetches the release, and the Slop app checks it again before
installing. Your Releases page shows **SIGNED BY YOUR KEY**.

### Slop Check

For apps not built with Slop Build. When you publish a GitHub release, Slop Check downloads its
Windows files on GitHub's Windows runner, scans them with Microsoft Defender, notes whether they're
signed, and tells Slop. Add this file to your repository:

```yaml
# .github/workflows/slop-check.yml
name: Slop Check
on:
  release:
    types: [published]
  workflow_dispatch:
    inputs:
      tag:
        description: The release tag to check
jobs:
  check:
    uses: DocFoxHound/slop-build/.github/workflows/check.yml@v1
    permissions:
      contents: read
      id-token: write
    with:
      tag: ${{ inputs.tag }}
      app: my-game                     # your store address
      slop-url: https://slop.example   # the Slop site
```

Slop only accepts the check from your app's own repository. If you published before adding it,
run it by hand from the repository's **Actions** tab (**Run workflow**, with the release's tag).
Repositories that aren't on GitHub can't run it yet: a reviewer checks those Windows builds by
hand, so expect a longer wait.

## Slop Pulse (player counts)

Optional, recommended for games. With Pulse, your store page shows **N playing now** and the
**peak this month**, and players' playtime makes their ratings count more.

**How it works.** When a signed-in player starts your game from the Slop desktop app, the app sets
three environment variables for your game:

| Variable | Meaning |
|---|---|
| `SLOP_PULSE_TOKEN` | A one-time token for this play session |
| `SLOP_PULSE_URL` | Where to send pings, e.g. `https://slop.example/v1/pulse` |
| `SLOP_PULSE_INTERVAL` | Seconds between heartbeats (60) |

Your game sends the token, and nothing else, as JSON `{"token": "<SLOP_PULSE_TOKEN>"}`:

| Request | When |
|---|---|
| `POST {SLOP_PULSE_URL}/start` | Once, when the game starts |
| `POST {SLOP_PULSE_URL}/heartbeat` | Every `SLOP_PULSE_INTERVAL` seconds while running |
| `POST {SLOP_PULSE_URL}/end` | When the game quits (optional) |

Every call answers `204 No Content`. `422` means the token is unknown, expired or ended: stop
sending. If the variables aren't set (the game wasn't started from Slop, or the player turned off
"Share my playtime"), do nothing. **Never send anything else**: no device IDs, no gameplay data.
The token must be used within 15 minutes of launch, and a session ends after 10 minutes without a
heartbeat. Each account counts once, however many copies it runs.

Pick the snippet for your language and call it once at startup. Each is complete and has no
dependencies beyond the standard library unless noted.

**Python**

```python
import json, os, threading, time, urllib.error, urllib.request

def start_slop_pulse():
    token, url = os.environ.get("SLOP_PULSE_TOKEN"), os.environ.get("SLOP_PULSE_URL")
    every = int(os.environ.get("SLOP_PULSE_INTERVAL", "60"))
    if not token or not url:
        return

    def post(path):
        req = urllib.request.Request(f"{url}/{path}", data=json.dumps({"token": token}).encode(),
                                     headers={"Content-Type": "application/json"}, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=10) as res:
                return res.status
        except urllib.error.HTTPError as e:
            return e.code
        except OSError:
            return None  # offline: try again next time

    if post("start") != 204:
        return

    def beat():
        while True:
            time.sleep(every)
            if post("heartbeat") == 422:
                return

    threading.Thread(target=beat, daemon=True).start()
```

**JavaScript / TypeScript (Node 18+, Electron, Tauri's Node side)**

```js
export async function startSlopPulse() {
  const { SLOP_PULSE_TOKEN: token, SLOP_PULSE_URL: url } = process.env;
  const every = Number(process.env.SLOP_PULSE_INTERVAL ?? 60) * 1000;
  if (!token || !url) return;
  const post = (path) =>
    fetch(`${url}/${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token }),
    }).then((r) => r.status, () => 0);
  if ((await post("start")) !== 204) return;
  const timer = setInterval(async () => {
    if ((await post("heartbeat")) === 422) clearInterval(timer);
  }, every);
  timer.unref?.();
}
```

**Rust** (with [`ureq`](https://crates.io/crates/ureq) and `serde_json`)

```rust
pub fn start_slop_pulse() {
    let (Ok(token), Ok(url)) = (std::env::var("SLOP_PULSE_TOKEN"), std::env::var("SLOP_PULSE_URL")) else {
        return;
    };
    let every: u64 = std::env::var("SLOP_PULSE_INTERVAL").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let post = move |path: &str| ureq::post(&format!("{url}/{path}")).send_json(serde_json::json!({ "token": token })).is_ok();
    std::thread::spawn(move || {
        if !post("start") {
            return;
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(every));
            post("heartbeat");
        }
    });
}
```

**Go**

```go
func StartSlopPulse() {
	token, url := os.Getenv("SLOP_PULSE_TOKEN"), os.Getenv("SLOP_PULSE_URL")
	if token == "" || url == "" {
		return
	}
	every, err := strconv.Atoi(os.Getenv("SLOP_PULSE_INTERVAL"))
	if err != nil || every <= 0 {
		every = 60
	}
	body, _ := json.Marshal(map[string]string{"token": token})
	post := func(path string) int {
		res, err := http.Post(url+"/"+path, "application/json", bytes.NewReader(body))
		if err != nil {
			return 0
		}
		res.Body.Close()
		return res.StatusCode
	}
	go func() {
		if post("start") != 204 {
			return
		}
		for range time.Tick(time.Duration(every) * time.Second) {
			if post("heartbeat") == 422 {
				return
			}
		}
	}()
}
```

**C# (.NET, Unity, Godot C#)**

```csharp
public static class SlopPulse
{
    public static async void Start()
    {
        var token = Environment.GetEnvironmentVariable("SLOP_PULSE_TOKEN");
        var url = Environment.GetEnvironmentVariable("SLOP_PULSE_URL");
        if (string.IsNullOrEmpty(token) || string.IsNullOrEmpty(url)) return;
        var every = int.TryParse(Environment.GetEnvironmentVariable("SLOP_PULSE_INTERVAL"), out var s) ? s : 60;
        using var http = new System.Net.Http.HttpClient();
        async Task<int> Post(string path)
        {
            try
            {
                var body = new System.Net.Http.StringContent($"{{\"token\":\"{token}\"}}", System.Text.Encoding.UTF8, "application/json");
                return (int)(await http.PostAsync($"{url}/{path}", body)).StatusCode;
            }
            catch { return 0; }
        }
        if (await Post("start") != 204) return;
        while (true)
        {
            await Task.Delay(every * 1000);
            if (await Post("heartbeat") == 422) return;
        }
    }
}
```

**Godot 4 (GDScript)**: add this as an autoload (Project → Project Settings → Globals).

```gdscript
extends Node

var token := OS.get_environment("SLOP_PULSE_TOKEN")
var url := OS.get_environment("SLOP_PULSE_URL")

func _ready() -> void:
	if token.is_empty() or url.is_empty():
		return
	var every := OS.get_environment("SLOP_PULSE_INTERVAL")
	var timer := Timer.new()
	timer.wait_time = float(every) if every.is_valid_int() else 60.0
	timer.timeout.connect(func(): _post("heartbeat"))
	add_child(timer)
	_post("start")
	timer.start()

func _post(path: String) -> void:
	var http := HTTPRequest.new()
	add_child(http)
	http.request_completed.connect(func(_r, _code, _h, _b): http.queue_free())
	http.request(url + "/" + path, ["Content-Type: application/json"], HTTPClient.METHOD_POST,
		JSON.stringify({"token": token}))
```

**Unreal Engine 5 (C++)**: add `"HTTP"` to your module's `Build.cs`, and call
`FSlopPulse::Start()` once, from your GameInstance's `Init()`. (Slop's tests don't have Unreal,
so this one isn't compiled by them; the C++ above is.)

```cpp
#include "HttpModule.h"
#include "Interfaces/IHttpRequest.h"
#include "Containers/Ticker.h"
#include "HAL/PlatformMisc.h"

class FSlopPulse
{
public:
    static void Start()
    {
        const FString Token = FPlatformMisc::GetEnvironmentVariable(TEXT("SLOP_PULSE_TOKEN"));
        const FString Url = FPlatformMisc::GetEnvironmentVariable(TEXT("SLOP_PULSE_URL"));
        if (Token.IsEmpty() || Url.IsEmpty()) return;
        const FString Every = FPlatformMisc::GetEnvironmentVariable(TEXT("SLOP_PULSE_INTERVAL"));
        const float Seconds = Every.IsNumeric() ? FCString::Atof(*Every) : 60.f;
        Post(Url, Token, TEXT("start"));
        FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([Url, Token](float)
        {
            Post(Url, Token, TEXT("heartbeat"));
            return true;
        }), Seconds);
    }

private:
    static void Post(const FString& Url, const FString& Token, const TCHAR* Path)
    {
        TSharedRef<IHttpRequest, ESPMode::ThreadSafe> Request = FHttpModule::Get().CreateRequest();
        Request->SetURL(Url + TEXT("/") + Path);
        Request->SetVerb(TEXT("POST"));
        Request->SetHeader(TEXT("Content-Type"), TEXT("application/json"));
        // The token is URL-safe base64: nothing to escape.
        Request->SetContentAsString(FString::Printf(TEXT("{\"token\":\"%s\"}"), *Token));
        Request->ProcessRequest();
    }
};
```

**Testing it.** Launch your game from the Slop desktop app while signed in, then watch your store
page's "playing now", or check by hand:

```sh
curl -i -X POST "$SLOP_PULSE_URL/start" -H 'Content-Type: application/json' -d "{\"token\":\"$SLOP_PULSE_TOKEN\"}"
```

## Checklist for agents

To prepare someone's project for Slop, in order. Steps marked **(person)** always need the
project owner; steps marked **(token)** are yours if the owner gave you a Slop token, and theirs
if not.

0. **Ask for a token first** if you don't have Slop's tools (`maker_guide`, `create_store_page`…).
   The owner signs in to Slop, opens **Account → AI agents → Make token** (`/account/agents`),
   chooses **Publish** and **All my apps** (a token for some apps can't create new ones),
   accepts the Publisher Agreement the page shows them, and follows the setup steps it shows, in
   their own terminal window: the token goes into the `SLOP_TOKEN` environment variable, and
   your tool is connected with a reference to it (see
   [Working with your AI agent](#working-with-your-ai-agent)). Never ask them to paste the token
   into the chat, and never print `SLOP_TOKEN`; if a token shows up in the conversation, tell
   them to remove it on Account → AI agents and make a new one. Then read `maker_guide`. Without
   a token, do steps 1–6 and 10 and hand the rest to the owner.
1. Confirm the project is open source in a Git repository, and that it has a `LICENSE` file with an
   OSI-approved licence. If not, ask the owner which licence to use; don't choose one for them.
2. Make sure the project builds to a runnable program for each target platform, from a clean
   checkout, with a single command. Note the program's path relative to the build folder.
3. If the project talks to servers or other players, or loads mods, saves or levels: follow
   [Online play and mods](#online-play-and-mods), and copy that section into the project's
   agent instructions (`AGENTS.md` or similar). Replace anything that runs downloaded code,
   updates the app itself, or uses an unsafe data format.
4. If the app keeps anything on the player's computer (saves, settings, logs): keep it in
   `SLOP_SAVE_DIR` when that's set, as in [Saves and sandbox mode](#saves-and-sandbox-mode), as
   plain data, and nowhere else. Read the variable; never hard-code the folder's path. Then the
   owner can tick "Works in sandbox mode". Slop's source check looks for `SLOP_SAVE_DIR` (or
   Godot's `user://`) in every release.
5. Add Slop Pulse to games (optional but recommended): pick the snippet above for the project's
   language and call it once at startup. Send nothing but the token.
6. Either publish a GitHub release with the files the store page's platforms need (Option A; archives need the
   program path, which the owner enters on Slop), or add `.github/workflows/slop.yml` (Option B) with `app` set to the store address the
   owner will use, `slop-url` set to the Slop site, and `project-type` set to `rust`, `go` or
   `custom` (with `build-command`, `output-dir` and `exec`). For private repositories without
   GitHub Enterprise, add `sign: false`. Off GitHub, or for full control, set up Option C. For
   Option A, name files so the version is the only part that changes between releases
   (e.g. `game-1.0.0-windows.zip`), so Slop can pick up later releases by itself.
7. **(token)** Create the store page (`create_store_page`, or **Publish → New app** on the
   website) with the same store address, the repository URL, licence, type, stage, age rating,
   tags, and **Online** if the project connects to anything (see step 3). Write the description
   from the repository. If a tool answers `terms_required` (the Publisher Agreement changed
   since the owner accepted it), tell them it's waiting for them on Slop under **Account → AI
   agents** (the website also shows it when they next publish), wait until they've accepted it,
   then carry on.
8. Option A: **(token)** submit the GitHub release (`submit_github_release`, or on the website
   under **Releases & progress → Submit a release**).
   Option B: push to the main branch (or a `v*` tag) and check the run in the Actions tab.
9. **(token)** Follow the first release (`list_releases`, `get_fixes`; on the website **Publish**
   or **Releases & progress**) until it's approved and the app is in the store, and fix what
   Slop's checks find. A person reviews the first release; later ones are picked up and approved
   automatically if they pass every check.
10. Optional but recommended, ideally before step 8: add `.slop/media.json` with a square icon
   (512×512 PNG, `"icon": "icon.png"`), a 920×430 thumbnail and some screenshots (see
   [Pictures and videos](#pictures-and-videos)), push it, and run `check_pictures` (or have the
   owner press **Check my repository now** under **Edit store page**). An agent can capture
   screenshots from the running app if the owner wants, and can make the icon from the app's own
   art: its logo, a character, or a square crop of a screenshot. Never invent an icon that shows
   something the app doesn't have, or that suggests another game or brand; if there's nothing
   suitable, leave the icon out and tell the owner (Slop crops the thumbnail until there is one).
   `get_fixes` and `check_repository` list a missing icon under `recommendations`; it never
   blocks a release. Leave the gallery order to the owner; they set it on the website.

Facts an agent may need:

- Pictures live in the repository under `.slop/`, listed in `.slop/media.json`; Slop reads them
  at one commit (`check_pictures`, and at each release) and a person reviews each new set. The
  icon: `"icon": "icon.png"`, square PNG/JPEG/WebP, 256×256 to 1024×1024 (512×512 is best), up to
  4 MB. The thumbnail: 920×430 is best (at least 460×215). Screenshots: at least 320×180. Up to 20
  gallery items, 8 MB each.
- Store addresses: 3–60 lowercase letters, numbers and single dashes. Unique: if one is taken, Slop's answer
  names a free one (`that address is taken; my-game-2 is free`). The name itself needn't be unique.
- Versions: up to 40 characters of letters, numbers, `.`, `_`, `+` and `-`, starting with a letter
  or number (e.g. `1.2.0`, `build.17`).
- Store page platforms (the `platforms` field): `windows`, `linux`, `linux-deb`, `linux-rpm`.
  `linux` needs `linux-deb` and/or `linux-rpm`. Every release includes one file for each, and no
  others.
- Files by platform: `windows` → `archive`, a `.zip`; `linux` → `archive`, a `.tar.gz` (or
  `.tgz`); `linux-deb` → `installer`, a `.deb`; `linux-rpm` → `installer`, an `.rpm`. Archives
  need the program path; packages don't.
- A release can't reuse a version. Up to three releases can wait for review at once.
- Tags become versions with a leading `v` removed when a digit follows (`v1.2.0` → `1.2.0`).
- The program path inside an archive must be relative, use `/`, and not contain `..`. Archives
  may not contain symbolic links.

## Questions

**Can my app be closed source?** No. Every app on Slop links its source.

**Can I charge for my app or sell things in it?** No. Apps on Slop are free and sell nothing.
Players can donate to you if you turn donations on; donations unlock nothing.

**Does Slop host my builds?** No. They stay on your release page; Slop records each file's hash
and players' apps download from you and check it.

**What if I don't use GitHub?** Use Option B with your host's releases. Slop Build and maker
signatures are GitHub-only for now.

**Do I need Pulse?** No. It's what powers player counts, and players' playtime makes their votes
count more, so it's worth the few lines.
