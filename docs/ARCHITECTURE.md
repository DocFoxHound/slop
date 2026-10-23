# Slop — Technical Architecture

**Status:** Built and running as a test deployment (§0); the cloud setup below is still the plan · companion to [`DESIGN.md`](DESIGN.md)
**Last updated:** 2026-10-21
**Related:** [`PAYMENTS.md`](PAYMENTS.md) · [`SECURITY.md`](SECURITY.md) · [`METRICS.md`](METRICS.md) · [`LEGAL.md`](LEGAL.md)

---

`DESIGN.md` says *what* Slop does. This document sketches *how* a real version would be built. Section numbers like §10 refer to `DESIGN.md` unless marked otherwise.

**Decided 2026-10-17:**
- **One cloud provider** and a **small budget**, so managed services are used wherever possible.
- **PostgreSQL for everything.**
- **Slop Build:** Slop writes the build pipeline, and it runs on the maker's Git host. Slop doesn't compile on its own servers.
- **Builds live on makers' release pages.** Slop stores no builds or source code.
- **The maker signs each upload, then Slop signs it after scanning.**
- **Tauri on every platform.**
- **No staged rollouts.**

**Decided 2026-10-18:** the **API server is written in Rust** (axum, sqlx), matching the Tauri core.

## 0. What runs today (2026-10-21)

The diagram in §2 is the target. Today everything runs on **one server** (details and setup in
`deploy/README.md`):

| Part | Today | Target |
|---|---|---|
| API and website | One Rust program (`crates/slop-api`) behind Caddy, deployed by a systemd timer from `main` after the tests pass | The same program on a cloud container service |
| Database | Postgres in podman on the same server, nightly backups with a weekly restore check | Managed Postgres with point-in-time recovery |
| Job queue, search | In Postgres (as planned) | The same |
| Files | None stored: builds stay on makers' release pages; store-page pictures and videos are read from the repository (`GITHUB_RAW_BASE`) | Object storage + CDN for manifests and reports |
| Scanning | A locked-down container per file on the same server (ClamAV, YARA, capa), plus Microsoft Defender results from makers' GitHub runs (§6.7) | A separate cloud account, signed verdicts |
| Desktop app | Windows (built on GitHub, signed updates) and Linux (`.deb`, `.rpm`, built on the server) | Plus macOS, Android, and iOS if it goes ahead |
| Slop Build | GitHub Actions on makers' repositories (§6.2) | Plus Slop-run builders for other hosts |

The `sslip.io` server is the test deployment, which every commit on `main` reaches first;
production (`slopai.games`, its own server set up the same way) runs only commits already live
there, when the `production` tag is moved to them (`deploy/README.md`, "Production").

## 1. Goals and constraints

| Goal | Consequence for the architecture |
|---|---|
| Nothing reaches a player without being scanned | Every build passes through one pipeline. The CDN only serves files listed in a manifest Slop has signed. |
| A stolen Slop account can't push malware | Builds must carry the **pipeline's or maker's** signature as well as Slop's (§6.3). |
| Slop never holds money | Payments run on makers' own Stripe accounts. Slop only records them (`PAYMENTS.md`). |
| Small budget | Builds are made on GitHub's free runners (§6.2) and stay on the **maker's own release pages**. Slop stores only small signed manifests (§6.4). A single modular application, managed Postgres and a container service with autoscaling. No Kafka, no self-run Kubernetes, no own servers. |
| Five platforms, one product | One shared UI. Only installing and launching differs per platform. |

## 2. System overview

```
             ┌──────────────────────────── Clients ─────────────────────────────┐
             │  Slop app (Tauri): Windows · macOS · Linux · Android · iOS (TBD)  │
             │  Website (same UI) · Pulse SDK in games · Slop Build recipe       │
             └───────────────┬────────────────────────────────┬──────────────────┘
                             │ HTTPS (API)                    │ HTTPS (downloads)
                     ┌───────▼────────┐                ┌──────▼───────┐
                     │ Load balancer  │                │     CDN      │◄── signed manifests +
                     └───────┬────────┘                └──────▲───────┘    content-addressed chunks
       ┌─────────────────────▼─────────────────────────────────┼──────────────────────┐
       │  Slop API: one modular application on the cloud container service, autoscaled  │
       │  Identity · Catalog · Publishing · Distribution · Library · Ratings · Pulse    │
       │  Payment records · People · Teams · Chat · Moderation · Notify                 │
       └──────┬───────────────────┬──────────────────────┬──────────────────┬──────────┘
              │                   │                      │                  │
       ┌──────▼──────┐    ┌───────▼───────┐       ┌──────▼──────┐    ┌──────▼──────┐
       │ PostgreSQL  │    │ Object storage│       │ Job queue   │    │   Stripe    │
       │ (managed)   │    │ (uploads,     │       │ (in Postgres│    │ (external)  │
       │ + replica   │    │  chunks)      │       │  at first)  │    └─────────────┘
       └─────────────┘    └───────────────┘       └──────┬──────┘
                                       ┌─────────────────▼──────────────────────┐
                                       │ Isolated scan environment (separate    │
                                       │ cloud account, no route to core):      │
                                       │ malware engines · sandbox VMs · checks │
                                       │ → signed verdicts                      │
                                       └────────────────────────────────────────┘
```

## 3. Clients: Tauri everywhere

- **One Tauri 2 app** for Windows, macOS, Linux and Android, and iOS if it goes ahead. It uses the same TypeScript UI as the website.
- **Why:**
  - A small download, so it's cheap to distribute.
  - A Rust core with an explicit list of commands the UI may call, which is easy to lock down.
  - One codebase for a small team.
- **Fallback:** if the mobile builds run into trouble, keep Tauri on desktop and wrap the same UI in a thin native shell on phones.
- **The native core does:**
  - downloading, verifying and installing (§6.3);
  - launching apps with Pulse tokens;
  - keeping the local library;
  - applying revocations (`SECURITY.md` §5.5).
- **Phones:**
  - **Android** installs APKs, using the system's install-from-this-app permission.
  - **iOS: undecided** (`DESIGN.md` §17).

## 4. Modules

| Module | Owns |
|---|---|
| **Identity** | Accounts (16+), country detection, the UK under-18 view, third-party age checks to unlock 18+ content in the UK, sign-in through providers, sessions, devices, passkeys and maker signing identities |
| **Catalog** | Apps, store pages, tags, stages (Coming soon → Beta → Released), delisting, dead-project status, branches |
| **Publishing** | Repo links (GitHub, GitLab, Bitbucket, self-hosted), build uploads, release candidates |
| **Scan orchestrator** | Sends jobs to the isolated scan environment, stores verdicts and public reports |
| **Distribution** | Signed manifests, chunk storage, the revocation list |
| **Library** | Library, Interested, Private Slop sharing |
| **Ratings** | Votes, weights, reviews, rollups (`METRICS.md`) |
| **Pulse** | Launch tokens, heartbeats, live and peak counts (Postgres counters at first) |
| **Payment records** | Mirror of Stripe charges, server donations, People payments (`PAYMENTS.md` §8) |
| **People** | Profiles, quotes, Service Agreements, jobs, completion |
| **Teams** | Player teams, and official teams verified through GitHub organizations (`DESIGN.md` §14) |
| **Chat** | Friends and blocks, and the delivery service for end-to-end encrypted chats: devices and key packages, ordering, long-polled sync, encrypted attachments and backups (`DESIGN.md` §19, `SECURITY.md` §8.5). The encryption runs in `slop-client`. People job conversations stay in People |
| **Moderation** | Reports, notice-and-action, statements of reasons, audit log (`LEGAL.md` §5) |
| **Notify** | Email, push, in-app notifications |

## 5. Data stores
- **PostgreSQL (managed):** everything, including payment records and vote history. Point-in-time recovery, and a read replica when needed (`METRICS.md` §4.2).
- **Object storage + CDN:** signed manifests, scan reports, screenshots, store-page content and portfolios. **No builds and no source code** (§6.4).
- **Job queue:** in Postgres (`SELECT … FOR UPDATE SKIP LOCKED`).
- **Search:** Postgres full-text search.

## 6. From source code to launched app

### 6.1 How Steam does it (for reference)
- **Developers upload files, not installers.** They use Steam's upload tools to upload the game's folder as **depots** (sets of files per platform or language). Steam splits the files into chunks, removes duplicates, and stores them.
- **The Steam client downloads the chunks** into its own library folder (`steamapps/common/<game>`). It verifies them and launches the game using **launch options** set by the developer (which executable to start, with which arguments).
- **Shared requirements** such as DirectX or the Visual C++ runtime are installed from a set of **common redistributables**. A small install script lists them, and Steam runs them with admin rights through its own service on Windows.
- **Updates** download only the chunks that changed.
- **Games aren't sandboxed.** They run with the user's normal rights. On Linux, Steam runs games inside a "Steam Runtime" container so every game gets the same libraries.

### 6.2 Slop Build: how builds get made (proposed)

**Goal:** a maker only has to **verify their account and link their repo**. They shouldn't need to understand CI pipelines.

**How it works:** Slop doesn't compile on its own servers by default. It **writes the build pipeline for the maker**, and the pipeline runs on the free build machines of the maker's own Git host.
1. The maker signs in, verifies their account (passkey), and links their repo by installing the Slop GitHub App (read-only).
2. Slop looks at the repo and detects the project type: Godot, web/HTML5, Electron, Tauri, Python, Rust, Go, .NET, Unity…
3. Slop shows a **one-click "Add Slop Build"** button. It opens GitHub's own "create file" page, pre-filled with a short file (`.github/workflows/slop.yml`, about 5 lines) that calls **Slop's shared build recipe**. The maker clicks **Commit**. That's their only step.
   - Because the maker commits the file themselves, the Slop GitHub App can stay **read-only**.
4. From then on, whenever the maker pushes to the tracked branch (or presses "Build now" on Slop), the recipe runs on **GitHub's free runners**. It:
   - builds for Windows, macOS and Linux (plus Android where the project supports it);
   - **signs** the builds (§6.3);
   - attaches them to a **release on the maker's repo**, using the pipeline's own permission to create releases;
   - tells Slop.
5. **Slop updates the recipe centrally** (it's a versioned shared workflow), so makers never maintain it.

**Built (updated 2026-10-20):** the recipe (`slop-build/`) builds Windows and Linux (not macOS yet) for `rust`, `go`, `dotnet` (self-contained `dotnet publish`), `godot` (with C# when the project has a .csproj), `unity` (GameCI on Linux runners; the maker's own Unity licence in their secrets), `python` (PyInstaller), `electron` (electron-builder, unpacked), `tauri` (no bundle), `web` (the built site in a minimal Tauri shell generated in the run) and `custom`, each tested on GitHub's runners (Unity up to activation, as there's no test licence). Slop's setup panel detects the type (package.json decides Electron or web). `slop.toml` sets the program and arguments; `publish: false` builds and checks without a release. "Build now" opens the workflow on GitHub, where Run workflow starts it; starting it from Slop needs the GitHub App. Archives over 2 GiB go up in parts (§6.4). `slop.toml` also names shared runtimes (§6.3), and `linux-runtime: steam-sniper` compiles `rust`, `go` and `custom` Linux builds inside Valve's Steam Runtime 3 (sniper) SDK, so they run on any Linux with glibc 2.31 (built 2026-10-21). Still to do: macOS, Android, Unreal, and Slop-run builders for other hosts.

**Why this costs Slop almost nothing:** GitHub's standard runners are **free for public repos**, including macOS. Private repos get a monthly allowance on GitHub's free plan. Slop pays for no build machines and no storage.

**GitLab, Bitbucket and self-hosted Git:** the same idea using their CI (`.gitlab-ci.yml` and so on). Their free build minutes are much smaller than GitHub's for public projects, so there are two options:
- **(a)** Makers use their own free minutes. Fine for small apps.
- **(b)** **Slop-run build machines** that attach to the maker's project as a "project runner". The job still runs in the maker's pipeline and uploads to the maker's release page, but on Slop's computers.
  - Rough cost for Linux builds: a few cents per build on short-lived cloud machines. Windows costs a little more.
  - **macOS is expensive:** cloud Macs are rented as whole machines for at least 24 hours, so roughly $15–30 a day each.
  - Option (b) runs makers' code on Slop's machines, so it must use the **isolated scan environment**: one-use VMs with no credentials (`SECURITY.md` §5.2).
- **Decided:** at launch, **Slop Build on GitHub only**. GitLab/Bitbucket/self-hosted makers use option (a). Add (b) later, Linux and Windows first.

**Game engines and licences (why "GitHub doesn't need one" isn't quite right):**
- The **engine licence belongs to the maker**, not to GitHub or Slop. Whoever runs the build needs the engine installed and, for some engines, **activated with the maker's licence**.
- **Unity:** even the free Personal licence has to be activated. In CI, that means the maker stores their Unity licence as a secret in their own repo (the community GameCI project does this). With Slop Build on GitHub, the secret stays in the maker's repo. If Slop ran the build on its own machines, the maker would have to **hand Slop their Unity licence**, which isn't a good idea for either side.
- **Unreal:** free to use, but you have to accept Epic's EULA and link an Epic account to get the engine. It's heavy (tens of GB) and slow to build in CI. Support it later.
- **Godot, web/HTML5, Electron, Tauri, Python, Rust, Go and similar:** **no licence needed**. Most AI-made apps and games use these, so they're the launch targets.
- **macOS builds need macOS** (GitHub provides it free; Slop would have to rent Macs). Some tools (Godot, and Apple-signing tools that work on Linux) can produce Mac builds from Linux.

**Makers who build by hand:** they can still attach files to their own release page and sign them with a registered key (§6.3). Labelled "**Uploaded by maker**". There's **no upload to Slop**: every build lives on the maker's release page, and **every public app needs public source**.

**How a run reaches Slop (built 2026-10-18):** the recipe runs on the maker's pushes to their
main branch and `v*` tags, signs with `actions/attest-build-provenance`, uploads to a release on
the maker's repo, and submits to `POST /v1/slop-build/releases` with the run's GitHub OIDC token:
Slop accepts it only from the recipe (`job_workflow_ref`), in the app's repository, at the commit
being released. The **Add Slop Build** button opens GitHub's create-file page with the workflow
filled in. The Slop GitHub App (`github_app.rs`: release webhooks, private repositories) is built
but switched off until its GitHub registration is set up; without it, detection uses GitHub's
public API.

### 6.3 From build to launched app

```
 Maker's repo ─► Slop Build (on GitHub's runners) ─► signed release on the maker's repo ─► Slop is notified
                                                                                                 │
          Slop scan environment ◄── downloads each file once, scans it, then deletes it ◄────────┘
                   │
                   └─► Slop signs a small manifest: { file URL, SHA-256, size, maker/recipe signature, verdict }
                                   │
 Player's Slop app ─► reads the manifest ─► downloads from the maker's release page ─► checks hash + both signatures ─► installs ─► launches
```

1. **What a build contains:**
   - **Preferred: the app's files** (a zip per platform), like Steam's depots.
   - **Allowed: an installer** (`.exe`/`.msi`/`.pkg`/`.deb`) when the app really needs one. It's labelled "**Runs its own installer**" and gets stricter sandbox checks.
   - **Android: an APK.**
   - **A launch config:** the executable and arguments per platform (from `slop.toml` in the repository, or the release form), and the shared runtimes the Windows build needs from Slop's approved list (`slop-manifest/src/runtimes.rs`; built 2026-10-21). Network use, the mods folder and sandbox readiness are set on the store page and signed into the manifest.
2. **Signing** (details in `SECURITY.md` §5.3):
   - The build is signed **inside the pipeline**, with Sigstore "keyless" signing tied to the repo and workflow. The certificate even records **which recipe** built it, so Slop can show "**Built by Slop Build from source**".
   - Hand-built uploads are signed with a key registered to the maker's account.
   - **A stolen Slop session alone can't publish anything.**
3. **Slop verifies** the signature, that the commit exists in the linked repo, and the recipe identity.
4. **Scans** run in the isolated environment (DESIGN §10, `SECURITY.md` §5.2). Files are deleted after scanning.
5. **Slop signs** the release manifest (`SECURITY.md` §5.3) and publishes it, along with the transparency log entry.
6. **The client:**
   - downloads each file from the maker's release page;
   - checks **both** signatures and the SHA-256 before writing anything;
   - installs into **Slop's library folder in the user's profile**, so normal apps need no admin rights;
   - installs a missing shared runtime only after asking the player, from the vendor's own link, and runs it only when Windows says it's validly signed by that vendor (the vendors' links always serve the newest version, so a fixed hash can't be used);
   - runs a maker's installer only after asking the user, and with the OS's normal admin prompt.
   - adds a **mod** (DESIGN §9) only to the game its signed manifest names, and only once that game is installed. A mod is unpacked into its own folder (`mods/<slug>/files`); nothing in it is started. Uninstalling the game removes its mods.
   - installs a **community build** beside its original; its signed manifest names the original (`build_of`), so the original's Play button can offer it. Each app keeps its own save folder (`saves/<slug>`).
7. **Launch:** the client sweeps the app's save folder, then starts the executable from the launch config, passing a one-time **Pulse launch token**, its save folder in `SLOP_SAVE_DIR`, and in `SLOP_MODS` the folders of the game's mods (separated like `PATH`), leaving out any mod Slop has pulled. If the manifest names the game's own mods folder, the client first makes it hold exactly those mods (anything else goes to quarantine). It also passes `SLOP_LINK`, a folder the game uses to ask for mods while it runs (`slop-client/src/link.rs`). In **sandbox mode** (`SECURITY.md` §6.2) it starts inside bubblewrap on Linux, and in an AppContainer on Windows (through the Slop app itself as launcher, `--slop-sandbox`).
8. **Updates:** a new release goes through the same steps. **No staged rollout:** once scanned, it's available to everyone. Updates are full downloads.

### 6.4 Where builds are stored: on the maker's release page (decided)

**Slop stores no builds and no source code** (decided 2026-10-17). Storing other people's software could be abused, and it costs money.
- Builds live on the **maker's own release page**. GitHub Releases are free, with no limit on total size or bandwidth, but each file must be under 2 GiB.
- **Files over 2 GiB** are split into parts by Slop Build. The client joins them back together and checks the hash of the whole.
  **Built (2026-10-20):** an archive over 2,000,000,000 bytes becomes `<name>.001`, `<name>.002`… (up to 64), each attested on its own. Slop fetches the parts in order, hashing each and the joined file; the signed manifest lists every part's address, size, SHA-256 and attestation beside the whole file's. The desktop app downloads them one after another into one file, refusing at the first part whose size or hash differs, then checks the whole. A private part comes through `…/download/{platform}?part=N`. `.deb`/`.rpm` packages can't be split (a package manager installs them whole), so the recipe stops with an error if one is over the limit.
- **What Slop stores:** signed manifests, scan reports and hashes (a few KB per release), plus screenshots, store-page content and People portfolios.
- **Integrity doesn't depend on where the file lives.** The client only installs a file whose hash matches the manifest Slop signed. If a release file is changed or replaced after scanning, installs **fail safely** and the app is flagged ("file changed after scan").
- **Trade-offs:**
  - A maker can break their own app by deleting the release. The hash check keeps it safe, not available.
  - Downloads depend on GitHub's/GitLab's uptime.
  - GitHub can limit accounts with "excessive" bandwidth under its Acceptable Use Policies. If a very popular app hits that, Slop could consider a mirror later. That would mean storing files, so it would need a new decision.

### 6.5 Private Slop downloads without storing anything (proposed)
A private repo's release files can't be downloaded by people it's shared with. Slop solves this **without storing or relaying the files**:
1. A friend the app is shared with taps Install. Slop checks they're allowed (shared with them or their player team).
2. Slop asks GitHub for the file, using the read-only GitHub App the owner installed. **GitHub replies with a short-lived download link** (a signed URL that expires within minutes) rather than the file.
3. Slop passes **only that link** to the friend's Slop app, which downloads directly from GitHub and checks the hash as usual.

Slop never holds the file, and the link is useless after a few minutes.
- **GitLab and others:** check whether they give out similar short-lived links. If not, Private Slop on those hosts waits, or the friend needs their own read access to the repo.
- ⚖️ Check that GitHub's terms allow distributing a private repo's releases to non-collaborators through an app the owner installed.

### 6.6 Launch, Pulse and voting
1. The client asks for a launch token: a short-lived signed token tied to the account, app and session.
2. The Pulse SDK in the game sends `start` and then heartbeats.
3. The first valid `start` marks the account as **launched**, which allows voting. Playtime from the heartbeats feeds the vote weighting (`METRICS.md` §2).

### 6.7 The scan step today (2026-10-21)
Each release file gets a result on DESIGN §10's scale (passed, note, on hold, rejected, blocked) from
`crates/slop-api/src/scan.rs`. With `SLOP_SCANNER=local` (the test deployment), the API starts a
locked-down container per file (`deploy/scanner`: ClamAV, YARA with YARA-Forge rules, capa; no
network, read-only, nothing executed). Windows files also need a clean Microsoft Defender result
from Slop Build or Slop Check on GitHub's Windows runners. Source checks (`source_scan.rs`: secrets,
vulnerable or payment packages, licences, save folder, mods and Pulse use) run on the maker's
runner and come back with the release. With `SLOP_SCANNER=off`, reviewers, makers and the store
see "not scanned", and nothing claims a file was scanned.

Still to do (SECURITY §5.2): moving the scanner to its own cloud account, signing its verdicts, and
running builds in a sandbox VM to watch what they do.

## 7. Running it
- **Separate environments:** production, staging and development, with separate keys. Staging uses Stripe's test mode.
- **Infrastructure as code:** Terraform or OpenTofu for the cloud resources. Container images built in CI. Deploys go through staging first. No hand-configured servers, so **no Ansible needed**.
- **Isolated scan environment:** a **separate cloud account**, with no network route to the core. Results come back only through a queue the core pulls from.
- **Monitoring:** the cloud provider's logs and metrics (or a hosted Grafana). Alerts on:
  - scan backlog;
  - webhook failures;
  - payment-record mismatches;
  - unusual Pulse activity;
  - revocations.
- **Backups:** Postgres point-in-time recovery, with restores tested every quarter. Object storage versioned.
- **Budget levers:**
  - Build machines and download traffic would be the big costs. Slop Build on GitHub's runners and builds on makers' release pages remove both (§6.2, §6.4).
  - Malware-engine licences.
  - Sandbox VM time. Run sandboxes on short-lived/spot instances.

## 8. Rough build order
1. Identity, Catalog, **Slop Build recipes** (Godot, web, Electron/Tauri first) with pipeline signing, Distribution with Slop signing, and client install/launch. Scanning starts semi-automatic, with invited makers only.
2. The automated scan pipeline and public reports. Ratings and Pulse.
3. Donations and server donations (`PAYMENTS.md`).
4. Branches and community builds. Teams. Private Slop with private repos.
5. People and People payments. Chat.

**Status 2026-10-21:** a first version of all five steps is built and running on the test
deployment. What's left is in §0 and §6.7.

## 9. Open questions
1. **iOS:** whether to support it, and how (`DESIGN.md` §17).
2. **Linux runtime container:** decided 2026-10-21, offered as an option (`linux-runtime: steam-sniper`, §6.2). Godot and Unity already ship engine players built for old systems.
3. **Which cloud provider?** The design only needs managed Postgres, a container service, small object storage, and a second account for scanning.
4. **Slop Build (§6.2):** GitHub only at launch (decided). When to add Slop-run build machines for GitLab/Bitbucket?
5. **Private Slop downloads (§6.5):** confirm GitHub allows short-lived links for this. What about other hosts?
6. **Engines:** decided. Godot, web/HTML5, Electron/Tauri, Python, Rust, Go, .NET and Unity are built (§6.2). Unreal later.
