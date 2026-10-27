# SLAPorSLOP — Security Design

**Status:** Draft · companion to [`DESIGN.md`](DESIGN.md) §10–11, §16
**Last updated:** 2026-10-27
**Related:** [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`PAYMENTS.md`](PAYMENTS.md) · [`METRICS.md`](METRICS.md) · `LEGAL.md` (private)

---

SLAPorSLOP's whole promise is "AI-made software you can trust." That makes it a **software distribution platform**, one of the most valuable targets there is: whoever controls what SLAPorSLOP installs controls every player's computer. This document covers how SLAPorSLOP protects players, makers and itself.

## 1. What we protect (assets)

| Asset | Why it matters |
|---|---|
| **Players' devices** | SLAPorSLOP installs and launches executables on them |
| **The release signing keys** | They decide what every client will trust and install |
| **Publisher and creative accounts** | They can push updates and have donations and payments sent to them |
| **Money** | Donations and People payments. They go straight to makers' own Stripe accounts, so the risk is someone **redirecting** them, not stealing a SLAPorSLOP balance (`PAYMENTS.md`). |
| **Personal data** | Names, emails, linked devices, tax details, messages |
| **Integrity of ratings and Pulse** | They decide what people find and trust |
| **Private apps** | Users expect them to stay private |

## 2. Who might attack (threat actors)

| Actor | Goal | Typical method |
|---|---|---|
| **Malware author** posing as a maker | Infect players | Upload an app that hides its behaviour from the scanners, or turns malicious after an update |
| **Account / repo hijacker** | Push a malicious update to a popular app | Phishing the maker, stolen GitHub tokens, compromised dependencies |
| **Fraudster** | Cash out stolen cards, scam players | Card testing, laundering through donations, fake People jobs (`PAYMENTS.md` §9) |
| **Rating manipulator** | Boost their app or bury a rival | Bot accounts, faked launches, review bombing |
| **Curious or malicious user** | Read someone else's private apps, data or messages | Guessing IDs, broken permission checks |
| **Insider / compromised admin** | Anything above, at scale | Misused admin tools or credentials |
| **Attacker on the build/scan infrastructure** | Escape the sandbox, steal keys, fake a "passed" verdict | Uploaded code built to exploit the scanners |

## 3. Security principles

1. **Never trust uploaded code.** Every build and scan step treats source and binaries as hostile, even from trusted publishers.
2. **Defence in depth.** Scanning won't catch everything. That's why there are also two signatures (the maker's and SLAPorSLOP's), a transparency log, revocation, reports and fast response.
3. **Small blast radius.** No single key, account or service can push software to every player at once.
4. **Least privilege.** For services, staff, GitHub access and database roles.
5. **Be transparent.** Public security reports per app, a public transparency log of releases, published policies.
6. **Collect less.** Data we don't have can't leak (Pulse carries no device IDs; card data stays with the processor).

## 4. Accounts and authentication

### 4.1 Sign-in
- **OpenID Connect / OAuth 2.0 only** (Google, Apple, Microsoft, GitHub, Discord). SLAPorSLOP stores no passwords.
- Use the **authorization code flow with PKCE**, with `state` and `nonce` checked, and exact redirect URIs.
- **Desktop and phone apps** sign in through the **system browser** (not an embedded webview), returning through a loopback address or app link (RFC 8252). The app never sees the provider password.
- **Account linking:** a new provider is linked only from a signed-in session. **Never link or merge accounts automatically just because the email matches.** Some providers don't verify email, which would let an attacker take over an account.
  - **Built 2026-10-20** (`auth/mod.rs`, `auth/oauth.rs`, `auth/merge.rs`): linking works from any signed-in session (a signed-out browser signs in first and comes back; no recent sign-in needed since 2026-10-25, §4.3), and the OAuth flow is bound to the account and session that started it; finishing it in another session fails. If the linked sign-in has its own SLAPorSLOP account, the two are combined only after a confirmation in the same session; suspended, closed and staff accounts never are. Unlinking never removes the last sign-in.

### 4.2 Sessions
- Short-lived access tokens (≈15 min) and **rotating refresh tokens**. Reusing an old refresh token revokes the whole token family (a sign of theft).
- Tokens are kept in the OS secure store (Keychain, Windows Credential Manager/DPAPI, Secret Service/libsecret, Android Keystore, iOS Keychain), never in plain files.
- **Website:** `HttpOnly`, `Secure`, `SameSite=Lax` cookies. State-changing requests from other sites are refused using the browser's `Sec-Fetch-Site` and `Origin` headers (the Go 1.25 / `tower-http` scheme), instead of per-request CSRF tokens (changed 2026-10-18).
- **A devices page** lists every session and linked device, and can revoke each one.

### 4.3 Step-up authentication: dropped (2026-10-25)
**Decided 2026-10-25 by the owner:** a signed-in session is trusted for sensitive actions. SLAPorSLOP no longer asks "confirm it's you" (a sign-in from the last few minutes, or a passkey confirmation) before them, and no longer requires a passkey for publishing or payouts. Asking to sign in again to enable donations, link a sign-in or approve the desktop app was too much of a hassle for what it protected, and the owner accepts the trade-off: **whoever holds a live session (a stolen cookie, an unlocked computer) can do everything the account can**, money changes included.

What changed, all on the same day:
- **No recent sign-in needed** for: connecting, switching or disconnecting the Stripe account that receives donations (`donations.rs`), a creative's Stripe account (`people/stripe.rs`), linking and unlinking sign-ins (`auth/mod.rs`), combining accounts (`auth/merge.rs`), adding and removing passkeys (`passkeys.rs`), approving a desktop sign-in (`auth/desktop.rs`), adding a maker signing key (`maker_keys.rs`), closing the account (`account.rs`), starting chat over on a new device (`chat.rs`), and staff approving sensitive actions or changing roles (`staff.rs`). `auth::session::require_recent_sign_in` is gone.
- **Passkeys are optional.** The rule from 2026-10-21 (30 days after first publishing or setting up as a creative, publishing, release settings, repository changes and payout changes needed a passkey used in the last 12 hours) is gone, with its reminders, its notice, the "passkey required" dialog, and staff "passkey grace". An account with passkeys no longer needs one of them to add a passkey, link or unlink a sign-in or be combined into another account. Passkeys still work for signing in, without a username; the account page suggests one, without a deadline.
- **What still protects money and accounts:** switching or disconnecting the Stripe account still waits **72 hours**, announced at once in SLAPorSLOP and by email, and can be cancelled meanwhile (§9); a new signing key still works only after 72 hours, with a notice (§5.3); a new passkey sends a "security" notice; closing the account needs the typed confirmation; linking still needs signing in to the account being linked (§4.1); sessions time out when idle (§4.2) and can be signed out from the devices page; bank details live in Stripe.
- **No step-up is left.** Staff making an admin API token (`review.rs`) and staff changing roles or approving sensitive actions (`staff.rs`) are trusted on a signed-in session like everyone else; the `confirm_recent` check, `/v1/me/step-up/*`, `ApiError::StepUpRequired` and the website's confirm-with-a-passkey prompt are deleted. The `sessions.stepped_up_at` and `sessions.passkey_at` columns stay (old data), but nothing writes them.

**Bringing step-up back for money changes** would mean: calling a recent-sign-in check (the deleted `require_recent_sign_in`, 10 minutes, `sessions.created_at` or `stepped_up_at`) at the top of `donations.rs` `onboard` (first connection) and `request_change`, and `people/stripe.rs` `onboard`; answering a `step_up_required` error (`ApiError::StepUpRequired`, deleted); and showing a prompt (`StepUpPrompt` in `web/src/components/PasskeyStepUp.tsx`, deleted) where those calls fail in `AccountSections.tsx` and `DonationsManage.tsx`. The git history of 2026-10-25 has the removed code and its tests.

**Before 2026-10-25,** for the record: sensitive actions accepted a sign-in or passkey confirmation from the last ten minutes (built 2026-10-20); publishers and creatives needed a passkey used in the last 12 hours from 30 days after first publishing (built 2026-10-21, migration 0051); and the 2026-10-22 security review made accounts with passkeys need one of them for changing what protects the account, and made approving a desktop sign-in need a fresh sign-in. Closing the account needed a fresh sign-in too. Team admins and a provider-2FA check were planned and are not.

### 4.4 Linking a phone
The computer shows a QR code holding a short-lived (2 min), single-use pairing code. The phone, already signed in to the same account, confirms it. The link is listed on the devices page and can be revoked.

## 5. Software supply chain (the core risk)

### 5.1 From commit to upload
How builds get from source to the player's computer is described in `ARCHITECTURE.md` §6.2–6.5. In short:
- **Slop Build:** SLAPorSLOP writes the build pipeline (a shared, versioned recipe). It runs on the **maker's Git host** (GitHub's free runners at launch) and puts the builds on the **maker's release page**. SLAPorSLOP doesn't compile on its own servers, and **stores no builds or source code**.
- Every public app has **public source**. There's no "upload a build without a repo" option.
- SLAPorSLOP gets **read-only** access to the repo, so it can check the commit and run source scans:
  - **GitHub:** a GitHub App limited to the selected repos.
  - **GitLab / Bitbucket / self-hosted:** read-only deploy tokens.
- **Only the repository's owner can link it** (`crates/slop-api/src/repo_owner.rs`): the publisher's GitHub account ID must be the repository owner's, or they must be an owner of the organization that owns it. Collaborators can't, and a repository can back one app only, so nobody can list (and take donations for) someone else's work.
- Every upload names a **commit SHA**. SLAPorSLOP checks that the commit exists in the linked repo. Tags and branch names aren't trusted, because they can be moved.
- **Build attestations** (a signed CI record of "built from this commit by this workflow"):
  - Built by Slop Build's recipe: **"Built by Slop Build from source"**.
  - Built by the maker's own pipeline, with an attestation: **"Built from source"**.
  - Files the maker attached by hand: **"Uploaded by maker"**, with stricter sandbox checks.
- **The shared recipe is a high-value target.** Every app built with it trusts it. So:
  - Changes need two-person review.
  - Releases of the recipe are signed and pinned by version.
  - Makers' workflow files reference an exact version.
  - The recipe never handles SLAPorSLOP secrets.
- **Files are preferred over installers.** An app that needs its own installer gets a **"Runs its own installer"** label, the strictest sandbox checks, and an extra confirmation before it runs.
- Every file is referenced by its **SHA-256 hash** in a manifest SLAPorSLOP signs. Files live on the **maker's release page**, never with SLAPorSLOP (`ARCHITECTURE.md` §6.4). If one changes after scanning, it no longer matches its hash: installs stop and the app is flagged.
- The scan environment downloads each file once and **deletes it after scanning**.

**Repository identity (built 2026-10-22, `repo_identity.rs`).** An app's repository is recorded by GitHub's numeric repository and owner IDs as well as its URL. A name can be taken over (a renamed account, a deleted repository registered again: "repojacking"); the IDs can't. Slop Build's token and every file's build attestation (certificate extensions 1.3.6.1.4.1.57264.1.15 and .17) must name the recorded IDs, so a repository registered again under the old name can't publish to the app. Apps linked earlier get their IDs filled in from GitHub in the background; moving an app's repository records the new one's on first use.

### 5.2 Scanning
The checks are listed in DESIGN §10. The engineering rules:
- Scanning runs in a **separate cloud account and network**. Analysis workers and sandbox VMs are **ephemeral** (destroyed after each job), have **no credentials**, and only reach the internet through a monitoring proxy (so callbacks can be seen).
- Use **microVM/sandbox isolation** (e.g. Firecracker or gVisor) for static analysis workers, and full VMs for running builds. Keep the hypervisor and images patched.
- The scanner's output is a **verdict signed by the scan service key**. Distribution only accepts signed verdicts, so a compromised core service can't fake one, and a compromised scanner can't publish (§5.3).
- **Built so far (2026-10-20):** a container per file (`deploy/scanner`), started by the API with `--network=none`, `--read-only`, `--cap-drop=ALL`, `no-new-privileges`, memory and process limits, and the file mounted read-only. Static analysis only (ClamAV, YARA, capa); nothing is executed. Its verdict isn't signed yet, and it runs on the same host as the API: separating it is still to do. Signatures update every six hours.
- **Defender checks from makers' CI** are accepted only with a GitHub OIDC token whose repository is the app's repository (by GitHub's numeric repository and owner IDs, not only its name) and whose `job_workflow_ref` is a *tagged* version of SLAPorSLOP's `check.yml` or the Slop Build recipe, and only for files whose SHA-256 matches. **Fixed 2026-10-22:** Slop Build ran the maker's build in the same job that held the OIDC token, so build code could mint a token and report "clean" for anything. Now the maker's code runs in a `build` job with no `id-token` permission and read-only contents; signing, Defender, attestation and the report to SLAPorSLOP run in a separate `release` job on a fresh runner, which takes over only the build's archive (paths and file types checked) and a few validated settings. Slop Check passes its inputs through the environment, never into scripts. A report that found nothing never replaces one that found something: only a reviewer clears a detection.
- **Scans that didn't finish don't pass (fixed 2026-10-22).** An archive that won't unpack or is past the size limits, an engine error or timeout, or missing signatures holds the file for a person, the same as a detection. The scanner gets the file on stdin as an unprivileged user (an unmapped ID outside the container), and podman stops the container itself after 25 minutes.
- **Known limits**, stated openly: sandbox evasion (code that waits, or detects VMs) and logic bombs can get past. That's why §5.3–5.6 exist.

### 5.3 Signing: the maker first, then SLAPorSLOP (decided)
Every release carries **two signatures**, and the client requires both:

1. **The maker's signature** on the upload. It proves the build came from the maker, not from someone who only stole a SLAPorSLOP session.
   - **Default:** Sigstore "keyless" signing in the maker's CI:
     1. The CI job asks GitHub/GitLab for a short-lived identity token that says "this is workflow Y in repo X, running commit Z".
     2. Sigstore swaps that for a 10-minute certificate, the build's hash is signed with it, and the signature is recorded in Sigstore's public log.
     3. There's no long-lived key for the maker to lose. SLAPorSLOP checks that the certificate's repo and workflow match the app's linked repo. GitHub's `attest-build-provenance` action does all of this.
   - **With Slop Build**, the certificate also records **which shared recipe** ran (its exact version). That proves SLAPorSLOP's recipe built it, from that commit, in that repo.
   - **Files attached by hand:** a signing key registered to the maker's account. Registering or changing it needs a 72-hour delay and an email to the maker (no step-up since 2026-10-25, §4.3).
2. **SLAPorSLOP's signature** on the release manifest, added only after the maker's signature is verified and every scan has passed.

**Built (2026-10-18):** GitHub artifact attestations are verified with `sigstore-verify` against Sigstore's public-good root (certificate chain, transparency log, signature, issuer `token.actions.githubusercontent.com`), then checked to name the file's SHA-256 and the app's repository and the release's commit (`crates/slop-provenance`). The API refuses approval without one; the attestation travels in the signed manifest and the client re-verifies it, and refuses updates from a different repository than the installed app's. **Built 2026-10-20 (`maker_keys.rs`, `slop-manifest/src/sshsig.rs`):** the maker-registered key for hand-attached files: an Ed25519 SSH key, added with step-up, counting 72 hours later, with a notice when it's added. Files are signed with `ssh-keygen -Y sign -n slop-release` (OpenSSH's sshsig, verified against the file's digest, Ed25519 only) and the `.sig` attached beside them. SLAPorSLOP verifies it when fetching; the signature and key travel in the signed manifest and the client verifies it again. A file in parts (over 2 GiB, ARCHITECTURE §6.4) carries an attestation for every part; SLAPorSLOP and the client verify each against that part's SHA-256, and the client then checks the joined file against the whole's hash. Not built yet: refreshing the Sigstore root over TUF, and private repositories (GitHub's own Sigstore instance).

SLAPorSLOP's own keys use a **TUF-style** scheme (The Update Framework), so that no single key compromise is catastrophic:

| Role | Signs | Key storage |
|---|---|---|
| **Root** | Which keys are trusted for the other roles | Offline, hardware tokens, 3-of-5 people needed to sign |
| **Targets** | Release manifests (app, version, platform, hashes, verdict reference, the maker's signature) | HSM/KMS, used only by the Distribution service after checking a signed verdict |
| **Snapshot / Timestamp** | The current set of manifests, with expiry | Online KMS, short expiry (stops replay of old or rolled-back manifests) |

- Clients ship with the root keys and **verify both signatures and every hash before installing**. A failed check stops the install, with no option to override.
- Releases go into a public **append-only transparency log**. Makers are **emailed about every release** of their app, so one they didn't make stands out at once.
- **What SLAPorSLOP's signature means:** "this is the file the maker signed, and it passed our checks". It **doesn't** mean "safe" (`LEGAL.md` §5.2, §10).
- **OS code signing** (Windows Authenticode, Apple Developer ID) stays the **maker's** choice and responsibility. SLAPorSLOP doesn't sign other people's apps with its own OS certificate.
  - When apps are installed and launched through the SLAPorSLOP client, SLAPorSLOP's verification takes the place of the OS download checks, the way Steam's does. On Apple Silicon Macs, apps still need at least an ad-hoc signature to run, so Slop Build adds one if it's missing.

### 5.4 No staged rollouts (decided)
Once an update passes scanning, it's available to everyone at once. What replaces the gradual rollout:
- the maker's signature (§5.3);
- release emails to the maker;
- the transparency log;
- re-scans (§5.6);
- the kill switch (§5.5), which can stop a bad release on every client within hours.

### 5.5 Revocation (kill switch)
- Clients check a **signed revocation list** at least every few hours and when launching an app.
- A revoked release is **blocked from launching**, and the user is told why and offered the previous good version (or uninstall).
- Revocation needs **two people** (the on-call security engineer and a second approver), except for confirmed malware in an emergency, where one person may act and the second confirms within 1 hour.
- Every revocation gets a public notice on the app's security report.

### 5.6 Re-scanning
- Every live release is **re-scanned when scanner signatures update** (at least weekly) and immediately on credible reports (DESIGN §16).
  - **Built 2026-10-20 (`rescan.rs`):** the release players get for each app is downloaded again, checked against its approved hash and scanned with the current signatures (refreshed every six hours) at least weekly, five files every 15 minutes; a malware or "doesn't match its source" report re-scans it at once. A new finding, or a file that changed on the maker's page, puts it in the admin queue and alerts the admins (ops.rs); a reviewer withdraws it or keeps it with a logged reason, and a kept finding isn't raised again. Nothing is taken down automatically.
- Dependency advisories that hit a live release notify the publisher, with a deadline to fix based on severity.
  - **Built 2026-10-20 (`dep_watch.rs`):** the packages in each release's lockfiles (from the source check, `source_scan.rs`) are checked against OSV once a day, for the release players get and for releases waiting for review. Only package names and versions go to OSV. New advisories on a live release notify the maker, with the fixed version where OSV has one; ones Slop Build's check already showed at build time aren't repeated. A malicious package (OSV's `MAL-` entries) raises an abuse signal for the admins. Not built yet: a deadline by severity (OSV's severity data is uneven across ecosystems), and lockfiles SLAPorSLOP doesn't read yet (pnpm, Gradle, Godot and Unity package lists).

## 6. Runtime protection on the player's device

Apps from SLAPorSLOP run as normal programs with the user's permissions, as on Steam. SLAPorSLOP's protections are:
- the scans;
- the two signatures;
- per-user installs (no admin rights unless the app runs its own installer);
- shared components only from SLAPorSLOP's own verified copies;
- revocation.

Players can also run an app in **sandbox mode** (§6.2).
- Store pages show a **"What this app can access"** section: what the publisher declared, compared with what the sandbox scan actually saw. A mismatch fails the scan.
- **Android:** the OS's permission list is shown before install.
- **The Slop Pulse SDK** is small, open source, and pinned by hash. The scan checks that games include the real SDK, not a modified copy.

### 6.1 Servers SLAPorSLOP can't check (online play and mods)
SLAPorSLOP checks an app's code, never the servers it talks to: those belong to the maker or to other players, and can change at any moment. A game that runs whatever a server sends turns every server into a way onto players' computers. What SLAPorSLOP does about it:
- **The rule** (Content Policy, "Code from outside SLAPorSLOP"): an app only runs code that came in its release or in a mod approved on SLAPorSLOP. No code, scripts, packs or mods from servers, and no self-updaters. Data the app's own code reads (maps, levels, settings) may come from anywhere. The publishing guide's "Online play and mods" section is written for makers' AI agents to copy into their projects.
  - **Built 2026-10-21 (`source_scan.rs`):** the source check notes code loading (assemblies, Java classes, native libraries, `eval`/`exec`/`loadstring`, Godot packs and scripts, Unity asset bundles), self-updaters (in code and lockfiles) and data formats that can rebuild any object (`BinaryFormatter`, `pickle`, `bytes_to_var_with_objects`, `ObjectInputStream`…). Notes go to the reviewer; an update with a new one waits for a person. Reports for "Runs code or mods from outside SLAPorSLOP" are reviewed within 24 hours.
- **Telling players** (built 2026-10-21, migration 0045): makers declare under **Online** what an app connects to (their servers or database, servers players host, direct connections, other online services) and name their servers. The store page of an app that goes online says SLAPorSLOP can't check its servers and suggests sandbox mode. The source check notes networking code (engines' multiplayer, sockets, HTTP/WebSocket clients, online-service SDKs; files with the Slop Pulse snippet don't count), and a release whose app doesn't declare online play gets a note that holds an update for a person.
- **What's left over:** a server sending deliberately broken data to exploit a bug in how the game reads it. No check of the source can rule that out; sandbox mode (§6.2) limits what such an exploit reaches.

### 6.2 Sandbox mode and save folders (built 2026-10-21)
**Sandbox mode** is optional, per app, and the player's choice (decided 2026-10-21). SLAPorSLOP suggests it the first time a player starts an app that goes online (declared, or networking code the store page doesn't declare); "Run every app in sandbox mode" turns it on for everything. It limits what a compromised app can reach; it isn't a guarantee, and SLAPorSLOP never calls it "safe".
- **Linux (`slop-client/src/sandbox.rs`):** bubblewrap, as in Flatpak. The app gets new namespaces and no capabilities; the system's `/usr` and `/etc` read-only; an empty home and `/tmp`; its installed files and mods read-only; its save and cache folders writable; the graphics, sound and controller devices (their normal permissions still apply); the Wayland, X11 and PulseAudio sockets (not PipeWire's own socket, which could see and control every other app's streams; sound goes through its Pulse service); and the network unless the player blocked it (offered only for apps that don't declare going online). No session bus, SSH or keyring agents. The Linux packages depend on `bubblewrap`; where it can't make a sandbox (some systems restrict user namespaces), the app says why.
- **Known gaps:** on a desktop that runs X11 itself, any window can read the keyboard and send keystrokes to other windows (so a sandboxed app could type into a terminal), inside the box or not; the Library page tells players on such desktops (`sandbox::limits`). Under Wayland, X11 apps run in XWayland and only share with each other. The PulseAudio socket lets an app record the microphone; kernel anti-cheat and apps that run their own installer can't be sandboxed.
- **Windows (built 2026-10-21, `slop-client/src/appcontainer.rs`):** an AppContainer per app (`slop.<address>`, removed on uninstall), the sandbox Store apps run in. The SLAPorSLOP app starts a copy of itself as the launcher (`--slop-sandbox`), which grants the container read access to the app's files and mods and modify (not delete-child) access to its save and cache folders, adds a deny-write entry on a mods folder inside the saves and on Slop Link's answers, then starts the app with those capabilities (internet, and client/server on public and private networks, unless the player blocked the network) in a job object that ends it, and anything it started, when the launcher goes, and keeps it from logging off or switching desktops. Not yet tried on a real PC: controllers using raw HID and games with anti-cheat drivers may not work in it (the player can run normally once or turn sandbox mode off).
- **If an app closes with an error within 15 seconds of starting sandboxed**, the SLAPorSLOP app offers "Run normally this time", turning it off for that app, and telling SLAPorSLOP. Makers can declare "Works in sandbox mode"; three or more players' reports show a note on the store page.

**Save folders** are what make switching seamless: each app gets `saves/<slug>/` in SLAPorSLOP's library, outside its installed files, and finds it in `SLOP_SAVE_DIR`, in both modes. On Linux the usual per-user folders (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`) point inside it, and on Windows `APPDATA` and `LOCALAPPDATA`, so engines that follow them need no change; the cache goes to a separate folder. Because the folder is the one bridge between a sandboxed session and a normal one, it's hardened:
- **Swept before every launch:** links, pipes and devices are removed; programs and libraries (ELF, PE, Mach-O, by their first bytes) are moved to a quarantine folder and the player is told; execute permission is taken off every file. A sandboxed session can only leave data behind, so it can't plant a library that a normal session would load (for example through a Vulkan layer or GStreamer plugin folder under `XDG_DATA_HOME`).
- **Copied before every sandboxed session** (the newest 3, up to 2 GB): the player can go back from the library, and a restore copies the current saves first.
- **Mods:** mods may carry code, reviewed like an app's (a mod with networking code is shown as "goes online" and suggests sandbox mode). Mod folders are read-only in the sandbox, including a mods folder the game names inside its saves (it isn't swept for code; the client keeps it to exactly the approved mods before each start). A game asks for a server's mods through `SLOP_LINK`, a folder per run: it can write only `requests/` (plain files of at most 16 KiB; SLAPorSLOP never follows links), and `answers/` is read-only to it in the sandbox; servers only name mods, the player confirms each request, and the files come from SLAPorSLOP under its signature.
- **Saves are data:** the publishing guide requires plain formats for saves (the same rule as for anything from a server), and the source check notes formats that can rebuild objects, code loading, and reports on every release where it saves (`SLOP_SAVE_DIR`, Godot's `user://`, fixed per-user folders, or none found). A store page that says "works in sandbox mode" stops saying so to players while the newest release saves elsewhere.
- What remains: a game that reads its own saves carelessly can still be exploited by a poisoned save, as on any platform.
- Saves stay when an app is uninstalled, unless the player ticks "Delete my saves too". Switching to another build offers to copy the saves across; each build has its own folder.

## 7. The SLAPorSLOP client itself

The client installs software, so it's a prime target:
- **Self-updates are signed and verified** with the same TUF chain as apps.
- **Tauri:** only the Rust commands that are actually needed are exposed, with a strict Content Security Policy, no remote code in the UI, and every IPC argument validated.
- **The `slop://` link handler** treats every link as untrusted input. Links can only open pages or show confirmation dialogs; they can never install or launch anything without a user click.
- Installs run with the **least privilege** needed. On Windows and macOS, per-user installs where possible (no admin prompt).
- Local files (library database, cached manifests) are integrity-checked, so tampering with them can't bypass verification.

## 8. Platform services

### 8.1 Web and API
- Follow the **OWASP ASVS** (level 2 overall, level 3 for payments and publishing).
- **Authorization on every request, on the server.** This is the biggest web risk for private apps, chat and People messages, and payment records: every object read checks the caller's rights. Use non-guessable IDs (UUIDv7/ULID) as well, but never *instead of* checks. Postgres row-level security acts as a backstop.
- Rate limits per account, IP and endpoint. Stricter for sign-in, voting, donations, reports and messages. *Built: per IP for every request (`ratelimit.rs`), and 20 reports per account per day (`moderation.rs`).*
- Suspended accounts (`users.suspended_at`) can only read, appeal, accept terms, download their data, sign out or close the account: every other write is refused in the session extractor (`auth/session.rs`). Closed accounts can't sign in at all, and their sessions are revoked.
  - **Built (2026-10-18):** per-IP token buckets for sign-in (20/min), writes (60/min) and reads (600/min), in memory per API instance (`crates/slop-api/src/ratelimit.rs`). **Per account (built 2026-10-20, `limits.rs`):** votes 60/hour, written reviews 10/hour, reports 10/hour, chat and job messages 60/minute, friend requests 40/day, donation checkouts 10/hour, shares 60/hour, Interested marks 120/hour, team invitations 50/day, new apps 10/day, releases 30/hour, hiring requests 20/day; 429 with `Retry-After`. Still to do: a shared limiter once SLAPorSLOP runs several instances.
- Input validation and output encoding. Store pages and reviews are **Markdown rendered with a strict allowlist** (no raw HTML).
- **Security headers:** a CSP with nonces, HSTS (preload), `frame-ancestors 'none'`, Referrer-Policy, Permissions-Policy.
- Uploads (portfolio images, screenshots) are re-encoded, stripped of metadata and served from a **separate domain** (stops script injection through uploaded files).
  - **Built (2026-10-19), store-page pictures** (`crates/slop-api/src/media.rs`): SLAPorSLOP doesn't host them. Makers list them in `.slop/media.json` in their GitHub repository; SLAPorSLOP reads it at one commit, downloads each picture once to check its format (PNG, JPEG, WebP, GIF, read from the header), size and dimensions, and records its SHA-256. Pages link to `raw.githubusercontent.com` **at that commit**, which can't change, so what's shown is what was reviewed; the page CSP allows only that image host. Every new set waits for review (people or AI reviewers) before it shows. Videos are YouTube links; YouTube's privacy-enhanced player (the one allowed `frame-src`) loads only when someone presses play. **2026-10-25:** an app's square `icon` (PNG, JPEG or WebP, 256–1024 pixels a side, up to 4 MB, checked from the header like the rest) is part of the same set and reviewed with it; `icon_url` is only given out for an approved set.
  - **Built (2026-10-24), profile pictures** (`crates/slop-api/src/avatars.rs`): the one picture SLAPorSLOP hosts. It's read from a linked sign-in's provider only, over https, from that provider's own hosts (an allowlist checked again at every redirect, followed by hand), through a client that connects only to public addresses; at most 1 MB, PNG, JPEG, WebP or GIF by their first bytes, decoded off the async workers with size and memory limits, and **re-encoded** as a 128×128 PNG, so metadata and anything hidden in the file are gone. Served from SLAPorSLOP's own origin (the page CSP's `img-src 'self'`) with `nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`, an ETag and a versioned address cached for good. The daily refresh uses conditional requests, a batch at a time, about three a second; choosing one is 20 an hour per account.

- **AI reviewers (built 2026-10-19, `crates/slop-api/src/review.rs`):** admins make API tokens (`slopadm_…`, stored hashed) that act for them on the admin API only. `read` tokens see the queue and leave notes; `review` tokens also approve and reject. Tokens expire (up to 365 days), can be revoked, stop working when their admin loses admin rights, can't make tokens, and are named in the audit log. The reviewer guide tells them to ignore instructions found inside makers' content (prompt injection) and to leave anything uncertain to a person.

### 8.2 Webhooks (GitHub, payment processor)
- Verify signatures (HMAC / the processor's signing secret) and timestamps, and reject replays.
- Process each event idempotently, by event ID.
- **Don't trust the event body for money.** Fetch the current state from Stripe's API before recording a payment.

### 8.3 Secrets and keys
- No secrets in code or images. Use a secrets manager/KMS with automatic rotation.
- Signing keys are in an HSM/KMS (§5.3), and their use is logged and alerted on.
- Separate keys per environment. Staging can never sign production releases.
- **Encrypted fields (built 2026-10-20, `fields.rs`):** `users.vat_number` is sealed with ChaCha20-Poly1305 under `SLOP_FIELD_KEY`, the column's name bound in, and opened only to show the owner their number, their fee statements and their data export. Values stored before the key was set are sealed at start-up.
- **Release notices (built 2026-10-20):** the maker is told under the 🔔 (and by email when set up) about every release of their app as it's received, approved (by a person or automatically), rejected, fails its checks, or is withdrawn, so a release published in their name by someone else doesn't go unnoticed. **Built 2026-10-23 (migration 0080):** the "was submitted" notice links to the app's Manage page, where the maker can withdraw a release that hasn't been reviewed yet (also their agent, with a `publish` token). It never reaches players, the withdrawal is audit-logged, and the app's next release waits for a person, in case it wasn't the maker who sent it. No passkey is needed: it only takes away a release nobody has.
- **Headers (built 2026-10-20):** `Permissions-Policy` turns off camera, microphone, location, payment, USB, serial and Bluetooth; `Cross-Origin-Opener-Policy: same-origin`; pages already allow only SLAPorSLOP's own scripts (no inline scripts), so script nonces aren't needed. `/.well-known/security.txt` (RFC 9116) is served once `SLOP_SECURITY_CONTACT` is set, or else with the `SLOP_CONTACT_EMAIL` address.
- **Deploy checks (built 2026-10-20, `deploy/update.sh`):** besides the tests, every deploy runs clippy with warnings as errors (a failure blocks it), writes the build's software bill of materials (CycloneDX, `deploy/sbom.py`, served at `/sbom.json` and linked from the download page), and audits `Cargo.lock` and `package-lock.json` with OSV-Scanner. The audit never blocks a deploy; findings not reviewed and accepted in `deploy/audit-accepted.json` (each with its reason) become an admin alert. GitHub CI isn't used: the repository is private and the server already runs all of this for free.
- **Makers' repository tokens** (built 2026-10-20, `repo_tokens.rs`): read-only tokens for private repositories outside GitHub, checked before they're saved, encrypted with ChaCha20-Poly1305 under `SLOP_REPO_TOKEN_KEY` with the app's ID as associated data, never returned by the API, and only sent to the repository's own host (checked against private networks) in the `Authorization` header, which the HTTP client drops on a redirect to another host. SLAPorSLOP uses them to fetch release files and, for the source check, the repository archive at the release commit. Saving one is audit-logged. Today the key sits in the same env file as the others; it moves to the KMS with them.

### 8.4 Staff and admin access
- SSO with **hardware security keys** for all staff, and short-lived just-in-time production access.
- Admin tools have **roles** (moderator, support, finance, security). Sensitive actions need two people: revocations, transferring a disputed app to another account, disabling donations on an app, and account removals.
- Every admin action is written to an **append-only audit log**, visible to the security team.
- **Built 2026-10-20 (`staff.rs`):** roles owner, moderator, reviewer, support, finance and security, each allowing a fixed set of permissions; staff without a role only look; agent tokens are capped by both their level and their admin's roles. Sensitive actions (withdrawing a release, delisting an app, suspending or reinstating an account, changing staff roles) are requests that need `SLOP_APPROVALS_REQUIRED` different people, the requester first; approving takes a person signed in themselves, never a token, so an agent can't be the second person. It's 1 while SLAPorSLOP has one person: the action happens at once but is recorded the same way. Requests lapse after three days; SLAPorSLOP always keeps one owner.
- **Running SLAPorSLOP from an agent (decided 2026-10-22, `agent.rs`):** the owner may be away from a computer, so every admin panel action has an API route and an MCP tool, with the same checks, approvals and audit log as the website. Two things stay with a person signed in: the payments record and making tokens. Staff changes (giving or taking a role, removing someone from staff, `staff.rs`) need a `moderate` token and an admin who may manage staff, and an agent's request always waits for a person to approve it on the website, which can be the same admin on their phone with a passkey, even while one approval is enough. Removing someone from staff ends every role and revokes their admin tokens; SLAPorSLOP always keeps one owner.
- **Agents (built 2026-10-20, `agent.rs`, `review.rs`):** an admin signed in themselves makes an expiring, revocable API token for an AI agent, at one of three levels: `read` (look, leave notes), `review` (also approve, reject, decide reports and appeals) and `moderate` (also suspend and reinstate accounts, withdraw releases, take apps out of the store). The token acts only on the admin API, stops working if its admin loses admin rights, and every action is audit-logged under its name; reading an account's details is logged too. The same API is offered as MCP tools at `/v1/admin/mcp`, which call the REST endpoints through the router, so no check is skipped. Until roles and two-person approval exist, a `moderate` token is as powerful as its admin for those actions: give one only to an agent you direct, for a short time.

### 8.4.1 Makers' AI agent tokens (built 2026-10-21)

A maker's or creative's token (`slopmk_…`, `maker_agents.rs`) is a session of kind `agent`: hashed, listed with their devices, signed out like one, lasting what its owner chose, at most 12 months (365 days, checked on the server; decided 2026-10-25, up from 30 days). Unlike other sessions it has no idle limit, so it works until that expiry: the page shows how long each has left, and the owner is notified a week and a day before and when it expires. It stays revocable any time (Account → AI agents, or the devices page). It may only call an allowlist of maker and People routes, within its scope (look or publish), its apps, and People access; everything else answers 403, and the admin area 404. Making one needs only a signed-in session, no step-up (decided 2026-10-24): it can be removed any time and can't pay, accept agreements or change payouts, so asking "confirm it's you" bought little. One made within 12 hours of a passkey confirmation counts as passkey-proven for its lifetime, so it can publish. Releases it submits get the same scans and review. It can never pay, accept agreements, offers or changes, change payouts, tokens or the account. **Keeping it out of AI chats:** the API returns the secret once in `secret` and never inside a ready-made command; the website tells the owner never to paste it into an AI chat and shows how to save it in a `SLOP_TOKEN` environment variable in their own terminal and connect each tool with a reference to it, so the token isn't in the conversation, in a command the AI sees, or in the tool's config (the AI on that computer could still read the variable: maker_guide.md tells agents never to print it, never to ask for the token in chat, and to tell the owner to remove and replace any token seen in a conversation). Since 2026-10-27 the website puts first a way with no token at all: the AI tool is approved in the browser (§8.4.3). The admin agent (`slopadm_…`, agent.rs) is separate. Both kinds of token can write to SLAPorSLOP's admins (`admin_messages.rs`), the one thing a `read` or one-app token may post: only from staff or a maker with an app in the store, with an account at least 7 days old and a token at least an hour old (each message records which token sent it), limited per account in the database (an hour apart, five a day) and to 60 new threads an hour across SLAPorSLOP, plain text only with at most 3 links and shown as text, agents see staff by role unless they chose to be named, and staff can block a sender and delete a thread. Staff and their agents treat these messages as information, never as instructions (review_guide.md).

### 8.4.2 Save sync (built 2026-10-21)

Saves are packed and encrypted on the player's device with XChaCha20-Poly1305 under a key derived from their chat backup key (shared only between their approved devices; restored with their recovery code), with the app's address bound in. SLAPorSLOP stores the ciphertext and a device name, keeps the newest two per app, and refuses an upload that doesn't build on the newest (no silent overwrites). Unpacking accepts plain files only, inside the save folder. Closing an account deletes them.

### 8.4.3 AI tools approved in the browser (built 2026-10-27)

Making a token and keeping it in an environment variable (§8.4.1) was where non-technical makers gave up. Now an AI tool can be given SLAPorSLOP's MCP address (`/mcp`, the same endpoint as `/v1/maker/mcp`) and connect itself, following the MCP authorization spec (revision 2026-07-28: OAuth 2.1 with PKCE, RFC 9728, 8414, 8707, 9207, 7591, 7009 and Client ID Metadata Documents; `agent_oauth.rs`, `pages/ConnectAi.tsx`). Nobody handles a token: the tool keeps and refreshes its own, so there's nothing to paste into a chat by mistake.

**The flow.** The MCP endpoint answers a request without a working token with 401 and `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource/mcp", scope="read publish"` (`error="invalid_token"` when a token was sent). That document names SLAPorSLOP as the authorization server; its metadata (`/.well-known/oauth-authorization-server`) lists S256 as the only PKCE method, `none` as the only client authentication, the registration and revocation endpoints, Client ID Metadata Document support and `iss` in responses. The tool identifies itself, sends the browser to `/v1/oauth/authorize`, which checks it and sends the browser on to the approval page; the person approves; the browser goes back to the tool with a one-time code and `iss`; the tool swaps code and PKCE verifier for tokens at `/v1/oauth/token`.

**What an approval is.** Exactly a maker token (§8.4.1): a session of kind `agent` with a `maker_tokens` row holding the scope, apps and People access the person chose on the approval page, lasting what they chose (a day to 12 months, `MAX_DAYS`), plus an `agent_grants` row naming the tool. So the allow-list of maker routes, the scope, app and People checks, the Publisher Agreement rule (`terms_required`), the ten-agent limit, the expiry warnings and listing on Account → AI agents and the devices page all apply unchanged, and removing it there ends it at once: every request, access token or refresh, checks the session.

**Tokens and lifetimes.** The access token (`slopmk_…`, so the API reads it as a maker token; kept hashed in `access_tokens`) lasts an hour and never past the session's end. Because removal is checked on every request, its lifetime only bounds a copied access token without its refresh token; an hour (not the desktop app's 15 minutes) keeps refreshes, and so the chance of two copies of a tool racing, rare. The refresh token (`slopmr_…`, hashed in `refresh_tokens`) lasts until the session's end, with no idle limit (an agent session has none, §8.4.1), and is replaced on every use (OAuth 2.1 §4.3.1). **Reuse detection:** a refresh token presented after it was used ends the connection; within 30 seconds of its first use it's only refused (two copies of the same tool refreshing together), and the second request still gets nothing. A refresh token works only for the tool it was issued to, and only at this endpoint (the desktop endpoint now requires a desktop session). An authorization code lasts 5 minutes after approval, works once, and a second use ends the session the first one made.

**Identifying the tool.** Two ways, both public clients (a tool on someone's computer can't keep a secret; PKCE protects the code):
- **Client ID Metadata Documents** (preferred by the spec; used by Claude, Claude Code, ChatGPT, Codex and VS Code): the `client_id` is an https address, which SLAPorSLOP reads with the profile-picture client (public addresses only, no redirects, 5 s to connect, 15 s in all), only an exact `https` address with a host name and a path (no IP address, user name, query, fragment or dot segments), a 200 answer of at most 5 KB whose `client_id` is exactly that address, no shared-secret method, and valid redirect URIs. It's kept as its cache headers say, between a minute and a day; a failed read is never kept, and stops the approval.
- **Dynamic Client Registration** (RFC 7591; deprecated by the spec, still what Cursor and Gemini CLI use): 20 registrations an hour per address (IPv6 per /64), and none while 2,000 registrations have never started an approval. A registration that never starts one is deleted after a day; any tool with nothing still approved, 30 days after its last use. Only the code and refresh grants and the code response type; a tool that asked for a secret is told it has none.

**Return addresses.** https anywhere, or http to this computer (127.0.0.1, [::1], localhost), as the spec requires; no fragments or user names. Matched exactly, except that a loopback address may use any port and any of the computer's names for itself (RFC 8252 §7.3; Claude Code has sent 127.0.0.1 for a registered localhost); path and query still match. Apps' own schemes (`cursor://`, `ms-…:`) aren't accepted: no tool checked uses them, and a browser handing a link to whatever app claims a scheme is a risk of its own. Until the tool and its return address check out, nothing is sent back to it: the browser goes to SLAPorSLOP's own explanation page instead (no open redirect). After that, errors (no S256 challenge, a wrong `resource`, an unsupported response type) go back to the tool with `state` and `iss`.

**Audience.** `resource` (RFC 8707) must be one of SLAPorSLOP's MCP addresses (or the site, which the root metadata names), checked at the authorization and the token requests and recorded on the connection; anything else is `invalid_target`. The tokens only ever work on SLAPorSLOP's own maker API, which the MCP tools call through the router; SLAPorSLOP never passes them to anyone else.

**The approval page.** Signed in (signing in or up first, and coming back); no "confirm it's you" (§4.3). It shows the tool's name, whether that's only its own word (registered) or comes from its description's host, and prominently where approving sends the browser: "on this computer" for loopback, or the website's host, with a warning when it isn't one SLAPorSLOP knows (Claude, ChatGPT, VS Code and Cursor's own). It warns, as the desktop approval does, to deny if you didn't just ask your tool to connect. The choices are the token page's; publishing needs the Publisher Agreement, shown in place; at the ten-agent limit one can be removed in place. Approve and Deny are cookie-authenticated POSTs, so the Sec-Fetch-Site/Origin check refuses them from other sites (CSRF, §4.2); pages can't be framed (`frame-ancestors 'none'`, against clickjacking); agent sessions can't use them. Deny sends `access_denied` back to the tool and forgets the request.

| Threat | Mitigation |
|---|---|
| Someone sends a maker an approval link for their own tool ("consent phishing") | The page names where the code goes and warns about unknown websites; the maker chooses what it may do (look only, some apps); it's listed and removable; it can't pay, accept agreements or change payouts or the account (§8.4.1); releases still go through scans and review |
| A local program pretending to be a known tool (any program can listen on localhost and use Claude Code's description) | It can only get a code sent to this computer, which needs the maker to approve it right now; the page says "on this computer" and to deny if they didn't just connect. A remote attacker never gets a loopback code |
| A stolen code | PKCE S256 (only the tool that started holds the verifier), 5 minutes, single use, bound to the tool and return address; a second use ends the connection |
| A stolen refresh token | Rotation with reuse detection ends the connection; bound to its tool; the owner can remove it any time |
| A stolen access token | An hour at most; removing the connection ends it at once |
| Open redirect through the authorization endpoint | Unknown tools and unregistered return addresses go to SLAPorSLOP's own page, never to the address given |
| Mix-up with another authorization server | `iss` in every response, advertised in the metadata |
| SSRF through a tool's description address | Public addresses only, checked when resolved (DNS rebinding) and no IP literals, no redirects, size and time limits, never anything else from the description (no logo fetched) |
| Registration flooding | Per-address limit, a ceiling on never-used registrations, deletion of unused ones; registered tools hold no personal data |
| CSRF or clickjacking on Approve | Sec-Fetch-Site/Origin check on cookie POSTs; pages can't be framed |

**Left over:** a maker who approves a convincing phishing request still gives that tool what they chose (the same as handing over a token). Tool names are self-chosen, so a registered "Claude Code" may not be; the page says the name is only the tool's word and shows where the code goes. The per-address rate limits (§8.1) see all of Claude's or ChatGPT's users as a few addresses: a busy day could make their connectors wait; a limiter by token is the fix if it happens. Browser-based tools on other origins can register and get tokens (CORS is open on those endpoints, which use no cookies), but the MCP endpoint itself doesn't send CORS headers yet.

### 8.5 Chat: end-to-end encryption (decided)
SLAPorSLOP **can't** read chat messages (`LEGAL.md` §4.6).
- **Protocol:** **MLS** (Messaging Layer Security, RFC 9420), which handles one-to-one and group chats (player teams) with many devices.
  - Use **OpenMLS**, an MIT-licensed Rust library that fits the Tauri app's Rust core.
  - Signal's library is an alternative, but it's AGPL-licensed, which would affect SLAPorSLOP's own app licence.
- **Keys:** each device creates its own keys and keeps the private ones in the OS secure store (§4.2). SLAPorSLOP's server only stores public key packages, plus **encrypted** messages waiting to be delivered.
- **New devices:**
  - A new device is added by an existing one (QR pairing, §4.4). The server can't add a device on its own, so a stolen SLAPorSLOP session can't quietly read chats.
  - **Encrypted history backup is on by default** (decided; users can turn it off). Old messages reach a new device or a reinstall through it.
    - The backup is encrypted **on the device** with a key **only the user holds**. SLAPorSLOP stores only the encrypted backup and **never** has the key, so backups stay end-to-end.
    - **The key:** derived from the user's **passkey** where the platform supports it (WebAuthn's PRF extension). Otherwise it's a **recovery code** shown once at set-up, which the user must save.
    - If the user loses both the passkey and the recovery code, the history is gone. SLAPorSLOP can't recover it, and the app says so plainly.
    - Turning backup off deletes the stored copy.
- **Verifying people:** each chat has a safety code or QR code users can compare. Everyone in a chat is warned when someone's keys change.
- **Reports:** the reporter's app sends the reported messages, decrypted, with the reporter's consent.
  - Add **message franking**: a cryptographic tag that lets SLAPorSLOP check a reported message is genuine without being able to read any others. That way reports can't be faked.
- **Attachments:** encrypted on the device before upload. SLAPorSLOP stores only the encrypted file, with a size limit and automatic deletion after 30 days. SLAPorSLOP can't see what's in them.
- **What SLAPorSLOP still sees (metadata):** who is in a chat, when messages are sent, and their sizes. Keep the minimum and delete it quickly (e.g. delivery records after 30 days).
- **Website:** browser encryption is weaker, because the page's code is delivered fresh by the server each time. **Chat runs in the SLAPorSLOP app only** (desktop and phone). The website shows "open the SLAPorSLOP app to chat".
- **Built 2026-10-20** (`crates/slop-client/src/chat/`, `crates/slop-api/src/chat.rs`):
  - OpenMLS 0.9, ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`. Each device's credential is `slop:{user}:{device}`; its private keys and MLS state live in a local file encrypted (XChaCha20-Poly1305) with a key from the OS secure store. Without a secure store, chat refuses to start rather than write keys to a plain file.
  - **Devices:** registering needs a desktop sign-in (never the website). The first device stands alone; later ones are approved by an existing device signing `slop-chat-device-approval:v1:{user}:{device}:{key}` after the user types the 9-character code the new device shows. SLAPorSLOP checks the signature, and every app checks the whole chain back to the user's first device before adding a device to a chat. Trust on first use: if a person's first device changes ("start over", which needs a fresh sign-in), each chat with them says their security code changed. Security codes (eight groups of five digits) can be compared per person.
  - **Ordering:** a commit must name the epoch SLAPorSLOP last saw and moves it on; a message must use the current epoch; losers fetch and retry. Any member's app keeps a chat's devices in step with its members (adding new and approved devices, removing revoked ones and people who left), the group's first device at once and others after 45 seconds, so commits rarely race.
  - **Delivery:** long-polling (`/chat/sync`, 25 s), woken in-process. Messages, Welcomes and attachments are deleted after 30 days; key packages of removed devices and unapproved devices after a day.
  - **Attachments** are encrypted on the device with a fresh key each (in the message), checked by SHA-256 after decrypting, 15 MB at most. **Link previews and linked media** are fetched by the sender's app only, from public addresses (no private networks), with size and time limits.
  - **Franking:** each message carries a random key; SLAPorSLOP stores HMAC-SHA256(key, plaintext) with the message. A report sends that one plaintext and key, and SLAPorSLOP checks the tag.
  - **History:** a newly approved device gets a copy of the history from the approving one through the user's own "self" chat. The backup (on by default) is encrypted with a key derived by Argon2id from a 24-character recovery code, salted with the user ID; turning it off deletes SLAPorSLOP's copy. The key reaches the user's other devices through the self chat; the code itself is shown once.
  - **Typing indicators** are the one unencrypted signal: SLAPorSLOP holds "this person is typing in this chat" in memory for 6 seconds and passes it to the chat's members; nothing is written down.
  - **Notifications:** desktop notifications are made on the device from the decrypted message. Web push (`push.rs`) sends empty pushes signed with SLAPorSLOP's VAPID key, only to the known push services (so a subscription can't point SLAPorSLOP's server elsewhere); the phone fetches the notice itself, so push services learn only timing. Server notices about chats never contain message text.
  - **Overlay:** the desktop app registers Shift+Tab system-wide only while the window in front belongs to a process SLAPorSLOP launched (or its children) and fills its screen, and releases it otherwise. It reads only the foreground window's process, size and fullscreen state; nothing is injected into games.
  - Not yet: passkey-derived backup keys (PRF), message expiry on the device, and a phone app.
- **People job conversations** (decided 2026-10-19, `DESIGN.md` §15.5) aren't end-to-end encrypted: they're recorded, which both sides are told. SLAPorSLOP holds the text only while open and 30 days after closing, then keeps SHA-256 fingerprints that prove each side's downloaded copy is unchanged. Staff read one only from a copy a side submits for a problem or review dispute, and every mediator look at a job record is audit-logged.

## 9. Payment security

**Built 2026-10-20 (migration 0040):** `payments` and `vote_events` refuse every update and delete at the database (a trigger, like `audit_log`'s), so corrections are new rows. The only exceptions: closing an account replaces its ID on `vote_events` and nothing else, and the simulation sandbox may wipe its data in a database named `*_sim`. **Built 2026-10-21 (migration 0050):** the API can run as its own role (`slop_app`, `deploy/db-roles.sh`) that doesn't own the tables: it can only add rows to `payments`, `vote_events` and `audit_log` (plus the account-closing ID swap), and can't disable their triggers. Migrations run as the owner (`SLOP_MIGRATE_DATABASE_URL`); the API re-applies the role's grants after each migration and warns at start while it still owns its tables.
More detail in `PAYMENTS.md`.
- **SLAPorSLOP never holds money** (`LEGAL.md` §3.1). Payments are direct charges on makers' and creatives' own Stripe accounts, so there are no SLAPorSLOP balances to steal.
- **PCI DSS:** card data is entered only into Stripe's hosted fields or page, which keeps SLAPorSLOP on the smallest self-assessment (SAQ A). The pages that host checkout still need a strict CSP and monitoring for script changes (PCI DSS v4 requirements 6.4.3 and 11.6.1).
- **Bank details live in Stripe, not SLAPorSLOP.** Changing them happens in the maker's Stripe dashboard, protected by Stripe's own security.
- **The main risk is a hijacked account pointing donations at a different Stripe account.** Disconnecting or switching the Stripe account requires a 72-hour delay and a notice to the account's email and devices. (Built 2026-10-20 for donations. A first connection, and the change itself, work from any signed-in session: step-up was dropped on 2026-10-25, §4.3, which also says what bringing it back would take. People's Stripe account comes next.)
- **Payment-record integrity:** the app's database role can only add payment records, never change or delete them. Daily reconciliation against Stripe.
- **Fraud signals** go from the payment flow to the trust & safety team (`PAYMENTS.md` §9).

## 10. Privacy and data protection
- **GDPR / UK GDPR / CCPA:**
  - A clear privacy policy and a record of processing.
  - Data export and deletion, self-service.
  - Data processing agreements with every subprocessor.
- **Data minimization:** Pulse has no device IDs or gameplay data. Analytics are aggregated and opt-in where the law requires.
- **Retention:**
  - Pulse heartbeats are aggregated within 24 hours and the raw events deleted.
  - Logs are kept 30–90 days.
  - Payment records are kept for the legal minimum (`METRICS.md` §6).
- **Encryption:** TLS 1.2+ everywhere, encryption at rest, and **field-level encryption** for tax IDs and People contact details.
- **Age:** accounts are **16+** everywhere (decided). Donations and People jobs are 18+.
  - UK users are found from several signals (IP, billing, app-store and stated country). SLAPorSLOP stores only the resulting country. No VPN detection.
  - UK accounts start in an **under-18 view**: 18+ content hidden, messages and team invites from friends only (since 2026-10-22), no team chats (since 2026-10-23), and open teams never list them to outsiders. Open teams have no team chat at all, since anyone can join one.
  - A third-party age check unlocks 18+ content. SLAPorSLOP stores only "passed", the date and the method, never ID or face images (`LEGAL.md` §6).
  - Content ratings on store pages (`LEGAL.md` §5.7).
- **Deleting an account:** personal data is removed or anonymized. Votes become anonymous (not deleted, to keep rating history honest). Payment records stay for the legal retention period, with the personal link removed where the law allows.

## 11. Trust & safety (abuse of the platform)

**Tightened 2026-10-22 (security review: a hostile client scripting many accounts):**
- **Storage:** each account may send 200 MB of chat a day (messages, key updates and attachments together, kept 30 days), and synced saves count every kept copy toward the 200 MB allowance. The admin panel already alerts on low disk space.
- **Playtime** is credited at most at the speed of the clock per player and app, however many play sessions run at once, so vote weight can't be inflated with parallel sessions. A weekly check flags apps with many votes from accounts that barely played them (`thin_votes`).
- **Spam and cost:** offers to other makers (5 a day, one collapsed notice per branch), repository and licence lookups and media checks (30 an hour per account, licences cached a day), a limit of four repository proof checks at once (no full clones, files over 1 MB skipped), store pictures decoded off the async workers with a memory limit, demos charged only for the bytes they send, signed-out reports about the same thing merged, sandbox trouble reports only from players who started the app, and a stronger proof of work before checkouts.
- **Fetching makers' addresses:** one HTTP client whose resolver returns only public addresses (IPv4-mapped, NAT64 and 6to4 forms included) and which refuses redirects to private ones, used for release files, repository checks and demos.

**Built 2026-10-20 (`abuse.rs`, `limits.rs`):** hourly checks for vote bursts from accounts made in the last week, review bombs (a day's 👎 at least 5 times the usual), coordinated vote swings, app names that look like another maker's listed app (case, spacing, punctuation and look-alike characters folded), and accounts repeatedly hitting the per-account rate limits. Each is raised once in the admin queue and as an alert; nothing changes automatically, and a reviewer closes it with a note. **Built 2026-10-21:** icon similarity (each store-page picture gets a perceptual hash; a new thumbnail within 10 of 64 bits of another maker's live one, or a new icon as close to another maker's live icon, is shown to the reviewer, `media.rs`), known-bad links (public malware and phishing lists, URLhaus by default, refreshed daily and checked against store pages, creative profiles and reviews; shared sites match only exact addresses, `bad_links.rs`), and days with unusual voting shaded on each app's votes-per-day chart unless staff dismissed the signal (`votes.rs`).

| Threat | Mitigations |
|---|---|
| **Fake votes / bots** | Voting only after a recorded launch (DESIGN §7). One account = one vote. Votes from new accounts weighted less or delayed. Checks for clusters (same devices/IPs/payment methods). |
| **Review bombing** | Detect sudden spikes not tied to an update. Mark the period on the rating chart (like Steam's "off-topic review activity") instead of deleting votes. |
| **Faked Pulse counts** | Signed single-use launch tokens, one count per account, rate-limited heartbeats, spike detection (DESIGN §11). Accept that Pulse is *approximate*. It's never used for money. |
| **Impersonation** (fake "official" apps, copied store pages) | Verification for official teams, checks for similar names and icons, licence/code matching in scans |
| **Phishing and scams in reviews, profiles, People and chat** | Check links in reviews and profiles against known-bad domains. **Chat is end-to-end encrypted** (§8.5): link warnings and "pay me off-platform" warnings run **in the recipient's app**, plus one-click reporting. |
| **Illegal content** | Hash matching against known abuse imagery (e.g. PhotoDNA-type services) on all uploads. Mandatory reporting to authorities where required (e.g. NCMEC in the US). Text, image and audio classifiers. Human review. ⚖️ |
| **Non-consensual intimate images and deepfakes** | A dedicated request form. Removal within **48 hours** of a valid request (US TAKE IT DOWN Act), plus steps to catch identical copies (hash matching of removed images). `LEGAL.md` §5.11. |
| **Sexually explicit content** (banned) | The content scan checks images. Matches are rejected, with human review on request. Repeat offenders lose publishing rights. |
| **Copyright** | DMCA / EU notice-and-action process, with counter-notices and repeat-infringer rules |
| **EU Digital Services Act** | Notice-and-action, reasons given for takedowns, appeals. Several duties don't apply to small enterprises (`LEGAL.md` §4.2). |

## 12. Private apps

- **Access control:** a private app's metadata, builds and security report are visible only to the owner and the users/player teams it's shared with. This is checked on every request. Shared content is never sent to official teams (DESIGN §13), and this is enforced on the server, not just in the UI.
- **Private repos are allowed for private apps.** Public apps still need a public, open-source repo (DESIGN §8). The owner gives SLAPorSLOP permission to read the code, so it can be checked and scanned like any other app.
- **Builds stay on the owner's private release page.** People it's shared with download through **short-lived links** that SLAPorSLOP gets from GitHub. SLAPorSLOP never stores or relays the files (`ARCHITECTURE.md` §6.5).

### 12.1 Granting SLAPorSLOP access (all at launch)
- **GitHub:**
  1. The owner installs the **SLAPorSLOP GitHub App** and picks **"Only select repositories"**, choosing just this repo.
  2. The app asks for the minimum: **Contents: read** and **Metadata: read**. No write access, no access to issues, secrets or other repos.
  3. SLAPorSLOP uses short-lived (1-hour) installation tokens:
     - the **isolated scan environment** uses them to read code and builds;
     - the download service uses them only to request short-lived download links for people the app is shared with.
- **GitLab / Bitbucket / self-hosted Git:** a **read-only deploy token** limited to that one repo (e.g. GitLab `read_repository`). It's stored in the secrets manager and used only by the scan environment. Sharing builds from these hosts depends on `ARCHITECTURE.md` §6.5.
- **There's no source-upload option.** Code always comes from a Git host.
- The **Publish** screen shows exactly what access SLAPorSLOP has, and a "Check access" button.
- **Scan findings** are shown to the owner. People the app is shared with see the verdict, but no code excerpts. **Leaked secrets** are reported by file and line, with the value hidden.

### 12.2 SLAPorSLOP keeps no copies (decided)
SLAPorSLOP **doesn't store makers' source code or builds**, public or private (decided 2026-10-17). It fetches them from the repo whenever it needs to scan, and deletes them afterwards.
- **If the owner removes SLAPorSLOP's access, or deletes the repo or release:**
  - existing builds can't be re-scanned, so they're **frozen** (no updates);
  - after a **14-day grace period**, they're removed from sharing (private apps) or delisted (public apps);
  - installed copies keep working unless they're revoked for a security reason (§5.5).
- **What SLAPorSLOP keeps:** hashes, scan reports and signed manifests. That's enough to prove what was scanned and when (`LEGAL.md` §13), without keeping the software. From the source check (`source_scan.rs`) it keeps the package names and versions in the lockfiles, which licence the licence files are, and the file paths and line numbers of any notes, never the code itself.

**Built 2026-10-20** (`crates/slop-api/src/frozen.rs`): a daily check of every app's repository through GitHub's API (or, on other hosts, Git's anonymous discovery request for public repositories), plus "Check now" for the owner. A public app whose repository turns private counts as lost access. Frozen apps take no releases or approvals; public ones are delisted after 14 days and listed again when access returns; Private stops being shared after 14 days.

### 12.3 Making a private app public
The repo has to become public under an OSI-approved licence, followed by a fresh full scan.
**Built 2026-10-20:** the owner asks from the Manage page; SLAPorSLOP checks the repository is readable without signing in; it waits in the admin queue, where listing it checks again and makes it public. Until the scan pipeline exists, the "fresh full scan" is the reviewer's.

## 13. Detection, response and disclosure

- **Monitoring:** alerts on SLAPorSLOP signing-key use, maker signing-identity changes, revocations, admin actions, Stripe account changes, scan pipeline failures, spikes in reports, and payment-record mismatches.
- **Incident response plan:** severity levels, on-call rota, playbooks for:
  - A malicious release.
  - A compromised publisher account.
  - A signing key compromise (root-key rotation through TUF).
  - A data breach.
  - Donation fraud (card testing, redirected Stripe accounts).
- **Notifying people:** players who installed a revoked release get an in-client notice and email. Data breaches are reported to regulators within legal deadlines (72 hours under GDPR).
- **Vulnerability disclosure:**
  - A `security.txt` and a public policy with safe harbour.
  - A **bug bounty** after launch, with top rewards for anything that lets someone install unscanned code, bypass either signature, or redirect donations.
- **Before launch:** an external penetration test of the client, API, signing/update chain and payment flows, plus a threat-model review whenever a major feature is added.

## 14. The mockup and the simulation

The original mockup (`mockup/`) fakes scan results, security reports, sign-in, Pulse handshakes and reports in the browser, with invented data; none of these controls exist in it. The real SLAPorSLOP runs them as each section here says (built parts are marked with their date). The simulation sandbox (`docs/SIMULATION.md`) uses invented data on its own `_sim` database, never the real one.

## 15. Decisions (2026-10-17) and open questions

**Decided:**
- **Slop Build:** SLAPorSLOP writes the pipeline, and it runs on the maker's Git host. The pipeline (or the maker) signs, then SLAPorSLOP signs after scanning.
- **SLAPorSLOP stores no builds or source code.** Builds live on makers' release pages. Every public app has public source.
- No staged rollouts.
- Running apps in an OS sandbox is optional, not required.
- Private repos are allowed, with GitHub, GitLab, Bitbucket and self-hosted Git supported at launch.
- **End-to-end encrypted chat**, in the app only (§8.5).
- A **24-hour** human review of serious reports.
- Accounts are 16+. UK accounts start in an under-18 view, and an age check unlocks 18+ content. No VPN detection.
- Frozen apps are delisted after a **14-day** grace period (§12.2).
- Encrypted chat backups are **on by default** (§8.5).

**Open:**
1. **The 24-hour review needs staffing:** who is on call at launch?
2. **Linux runtime container:** offer one or not (`ARCHITECTURE.md` §9)?
3. **UK age-check provider** (`LEGAL.md` §6): Yoti, Persona, Stripe Identity or another?
