# Publishing on Slop

Slop is a store for apps and games made with AI: open source, checked before they're listed,
free to download, and rated by the people who play them. This page covers everything needed to
publish: what Slop asks of your project, how to set it up, and what happens after you submit.

> **Using an AI coding assistant?** Tell it to read this page (the plain-text copy is at
> `/publishing.md` on this site). It can work out what's required and what's optional, and set
> your project up for publishing on Slop. For example: *"Read this page, tell me what's required
> and what's optional for publishing on Slop, then set up my project so it's ready to deploy
> there."*

It's written so that a person or an AI agent can follow it from start to finish. Agents: the
[checklist for agents](#checklist-for-agents) near the end lists every step in order, and
[What Slop asks of your project](#what-slop-asks-of-your-project) says what's required and what's
optional.

> **This is a test server.** Builds aren't automatically scanned yet and maker signatures aren't
> required. Each app's first release is reviewed by a person. Things marked *(coming)* below are planned
> but not built yet.

## Why publish on Slop

- **Free to list.** No fees to publish or to download. Slop never sells anything and takes no cut
  of sales, because there are none. *(coming: optional donations, paid straight to you, with a
  flat 5% Slop fee.)*
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
   licence from your repository when you create your store page.
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
   no malware, no hidden tracking, nothing stolen. Rate your app honestly (Everyone, 3+, 7+, 12+,
   16+ or 18+; 18+ apps are hidden from players in the UK's under-18 view).
5. **A maker signature** on each build: a GitHub artifact attestation, made by your GitHub Actions
   build (Slop Build does this for you). It proves the file came from your repository at that
   commit, so someone who only stole your Slop password can't publish anything. *Not required on
   this test server; GitHub only makes attestations for private repositories on its Enterprise
   plan.*

### Recommended

- **Slop Pulse** in games, so Slop can show player counts. See
  [Slop Pulse](#slop-pulse-player-counts).
- **Slop Build**, so releases build, sign and submit themselves. See
  [Option A](#option-a-slop-build-recommended).

## Release stages

| Stage | What it means | Time limit |
|---|---|---|
| **Coming soon** | A store page with no build yet. Players can mark it Interested. | 30 days, then it must reach Beta |
| **Beta** | Playable builds, still changing | 12 months, then it must be released |
| **Released** | Finished (and still updated) | none |

A Coming soon or Beta project with **no commits for 90 days** is labelled a dead project. It stays
visible but isn't recommended, and comes back with your next commit. Missing a stage deadline
takes the page out of the store until you reach the next stage; existing installs keep working.

## Step by step

### 1. Sign in

On the Slop website, choose **Sign in → Continue with GitHub**, then pick a display name and your
country. Slop never sees your password.

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

### Pictures and videos (optional, but pages with them get far more clicks)

Slop doesn't host pictures. They live **in your repository**, in a `.slop/` folder at the top
level, listed in `.slop/media.json`:

```
.slop/
  media.json
  thumbnail.png
  screenshots/
    title.png
    level-1.png
    trailer-poster.png
```

```json
{
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
| `thumbnail` | Optional. Your tile in the store. Best at **920×430** (at least 460×215); other shapes are cropped from the middle |
| `gallery` | Optional. Up to **20** items, each either `image` (a screenshot) or `youtube` (a video link) |
| `image` | A picture at least 320×180 |
| `youtube` | A YouTube link (`watch?v=`, `youtu.be/`, `shorts/` or `embed/`) |
| `poster` | Optional, videos only: a picture shown until someone presses play. Without one, the page shows a plain card |
| `caption` | Optional, one line of up to 200 characters |

Rules:

- Paths are relative to `.slop/`, using letters, numbers, `.`, `_`, `-` and `/` (no spaces, no `..`).
- Pictures are PNG, JPEG, WebP or GIF, up to 8 MB each, and at most 8192 pixels on a side.
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
| `project-type` | `rust` (`cargo build --release`, ships the first binary), `go` (`go build`, binary named after the repository) or `custom` (required) |
| `platforms` | Comma-separated, from `windows,linux` (default: both). Match your store page. |
| `linux-packages` | Comma-separated, from `deb,rpm` (default: both). Slop uses the ones your store page lists. Packages install the app in `/opt/<app>` with `/usr/bin/<app>` starting it. |
| `build-command` | `custom` only: the command that builds your app (run with bash) |
| `output-dir` | `custom` only: the folder the build produces; it's packed as-is |
| `exec` | `custom` only: the program to start, relative to `output-dir`; `{ext}` becomes `.exe` on Windows |
| `sign` | `true` by default; set `false` for private repositories without GitHub Enterprise |
| `signpath-organization-id` | Optional: sign the Windows program for free with SignPath Foundation (see [Signing](#signing)). Needs the `SIGNPATH_API_TOKEN` secret |
| `signpath-project-slug` | Your SignPath project (default: the app's store address) |
| `signpath-signing-policy-slug` | Your SignPath signing policy (default: `release-signing`) |

Every Slop Build also checks your Windows build with **Microsoft Defender** (today's definitions,
on GitHub's Windows runner), looks for secrets committed to your source with
[gitleaks](https://github.com/gitleaks/gitleaks), and lists dependencies with known
vulnerabilities with [OSV-Scanner](https://github.com/google/osv-scanner). Only you and Slop's
reviewers see the source checks; they never stop a release.

Godot, web/HTML5, Electron/Tauri and Python projects use `custom` for now, for example:

```yaml
    with:
      app: my-godot-game
      slop-url: https://slop.example
      project-type: custom
      build-command: ./build.sh          # e.g. godot --headless --export-release "Linux" build/game
      output-dir: build
      exec: "game{ext}"
```

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

Your app's **Releases** page shows a **Publishing progress** checklist:

1. **Store page:** saved.
2. **Release:** received.
3. **Files checked:** Slop downloads each file once, records its size and SHA-256, and keeps
   nothing else. If a file can't be downloaded, the release fails with the reason.
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
6. **Review:** a person at Slop reviews your app's first release. Approval signs it; a rejection
   comes with a reason.
7. **In the store:** Slop staff put the app in the store.

**Updates.** With **Approve updates automatically** on (the default), every later release that
passes every check is approved and signed by Slop straight away, with no wait for a person.
Combined with **Pick up new GitHub releases** (or Slop Build), publishing an update is just
publishing a release on GitHub. Players' apps show a notice and update when the player chooses.
A person looks again if a release fails a check, or if one of your releases was withdrawn or
rejected since the last one a person approved. Slop can withdraw a release that turns out to be
harmful; it then stops launching for everyone.

**Where things stand:** **Publish** lists each of your apps with the step it's waiting on or
stuck at. **Releases & progress** has the full checklist and every release's history.

### 5. Managing your app

Each app has a **Manage** page (Publish → **Manage**):

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

**Testing it.** Launch your game from the Slop desktop app while signed in, then watch your store
page's "playing now", or check by hand:

```sh
curl -i -X POST "$SLOP_PULSE_URL/start" -H 'Content-Type: application/json' -d "{\"token\":\"$SLOP_PULSE_TOKEN\"}"
```

## Checklist for agents

To prepare someone's project for Slop, in order. Steps marked **(person)** need the project owner.

1. Confirm the project is open source in a Git repository, and that it has a `LICENSE` file with an
   OSI-approved licence. If not, ask the owner which licence to use; don't choose one for them.
2. Make sure the project builds to a runnable program for each target platform, from a clean
   checkout, with a single command. Note the program's path relative to the build folder.
3. Add Slop Pulse to games (optional but recommended): pick the snippet above for the project's
   language and call it once at startup. Send nothing but the token.
4. Either publish a GitHub release with the files the store page's platforms need (Option A; archives need the
   program path, which the owner enters on Slop), or add `.github/workflows/slop.yml` (Option B) with `app` set to the store address the
   owner will use, `slop-url` set to the Slop site, and `project-type` set to `rust`, `go` or
   `custom` (with `build-command`, `output-dir` and `exec`). For private repositories without
   GitHub Enterprise, add `sign: false`. Off GitHub, or for full control, set up Option C. For
   Option A, name files so the version is the only part that changes between releases
   (e.g. `game-1.0.0-windows.zip`), so Slop can pick up later releases by itself.
5. **(person)** Sign in to Slop, go to **Publish → New app**, and create the store page with the
   same store address, the repository URL, licence, type, stage, age rating and tags.
6. Option A: **(person)** on **Releases & progress**, pick the release under **Submit a release**.
   Option B: push to the main branch (or a `v*` tag) and check the run in the Actions tab.
7. **(person)** Watch **Publish** (each app's current step) or **Releases & progress** until the
   first release is approved and the app is in the store. Later releases are picked up and
   approved automatically if they pass every check.
8. Optional but recommended, ideally before step 6: add `.slop/media.json` with a 920×430
   thumbnail and some screenshots (see **Pictures and videos**), push it, and have the owner press **Check my
   repository now** under **Edit store page**. An agent can capture screenshots from the
   running app if the owner wants. Leave the gallery order to the owner; they set it on the
   website.

Facts an agent may need:

- Store addresses: 3–60 lowercase letters, numbers and single dashes.
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
*(coming: optional donations.)*

**Does Slop host my builds?** No. They stay on your release page; Slop records each file's hash
and players' apps download from you and check it.

**What if I don't use GitHub?** Use Option B with your host's releases. Slop Build and maker
signatures are GitHub-only for now.

**Do I need Pulse?** No. It's what powers player counts, and players' playtime makes their votes
count more, so it's worth the few lines.
