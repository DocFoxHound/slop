# Slop — Security Design

**Status:** Draft · companion to [`DESIGN.md`](DESIGN.md) §10–11, §16
**Last updated:** 2026-10-17
**Related:** [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`PAYMENTS.md`](PAYMENTS.md) · [`METRICS.md`](METRICS.md) · [`LEGAL.md`](LEGAL.md)

---

Slop's whole promise is "AI-made software you can trust." That makes it a **software distribution platform**, one of the most valuable targets there is: whoever controls what Slop installs controls every player's computer. This document covers how Slop protects players, makers and itself.

## 1. What we protect (assets)

| Asset | Why it matters |
|---|---|
| **Players' devices** | Slop installs and launches executables on them |
| **The release signing keys** | They decide what every client will trust and install |
| **Publisher and creative accounts** | They can push updates and have donations and payments sent to them |
| **Money** | Donations and People payments. They go straight to makers' own Stripe accounts, so the risk is someone **redirecting** them, not stealing a Slop balance (`PAYMENTS.md`). |
| **Personal data** | Names, emails, linked devices, tax details, messages |
| **Integrity of ratings and Pulse** | They decide what people find and trust |
| **Private Slop** | Users expect it to stay private |

## 2. Who might attack (threat actors)

| Actor | Goal | Typical method |
|---|---|---|
| **Malware author** posing as a maker | Infect players | Upload an app that hides its behaviour from the scanners, or turns malicious after an update |
| **Account / repo hijacker** | Push a malicious update to a popular app | Phishing the maker, stolen GitHub tokens, compromised dependencies |
| **Fraudster** | Cash out stolen cards, scam players | Card testing, laundering through donations, fake People jobs (`PAYMENTS.md` §9) |
| **Rating manipulator** | Boost their app or bury a rival | Bot accounts, faked launches, review bombing |
| **Curious or malicious user** | Read someone else's Private Slop, data or messages | Guessing IDs, broken permission checks |
| **Insider / compromised admin** | Anything above, at scale | Misused admin tools or credentials |
| **Attacker on the build/scan infrastructure** | Escape the sandbox, steal keys, fake a "passed" verdict | Uploaded code built to exploit the scanners |

## 3. Security principles

1. **Never trust uploaded code.** Every build and scan step treats source and binaries as hostile, even from trusted publishers.
2. **Defence in depth.** Scanning won't catch everything. That's why there are also two signatures (the maker's and Slop's), a transparency log, revocation, reports and fast response.
3. **Small blast radius.** No single key, account or service can push software to every player at once.
4. **Least privilege.** For services, staff, GitHub access and database roles.
5. **Be transparent.** Public security reports per app, a public transparency log of releases, published policies.
6. **Collect less.** Data we don't have can't leak (Pulse carries no device IDs; card data stays with the processor).

## 4. Accounts and authentication

### 4.1 Sign-in
- **OpenID Connect / OAuth 2.0 only** (Google, Apple, Microsoft, GitHub, Discord). Slop stores no passwords.
- Use the **authorization code flow with PKCE**, with `state` and `nonce` checked, and exact redirect URIs.
- **Desktop and phone apps** sign in through the **system browser** (not an embedded webview), returning through a loopback address or app link (RFC 8252). The app never sees the provider password.
- **Account linking:** a new provider is linked only from a signed-in session. **Never link or merge accounts automatically just because the email matches.** Some providers don't verify email, which would let an attacker take over an account.
  - **Built 2026-10-20** (`auth/mod.rs`, `auth/oauth.rs`, `auth/merge.rs`): linking needs a sign-in from the last 10 minutes (else the browser signs in again first and comes back), and the OAuth flow is bound to the account and session that started it; finishing it in another session fails. If the linked sign-in has its own Slop account, the two are combined only after a confirmation in the same session; suspended, closed and staff accounts never are. Unlinking needs a recent sign-in and never removes the last sign-in.

### 4.2 Sessions
- Short-lived access tokens (≈15 min) and **rotating refresh tokens**. Reusing an old refresh token revokes the whole token family (a sign of theft).
- Tokens are kept in the OS secure store (Keychain, Windows Credential Manager/DPAPI, Secret Service/libsecret, Android Keystore, iOS Keychain), never in plain files.
- **Website:** `HttpOnly`, `Secure`, `SameSite=Lax` cookies. State-changing requests from other sites are refused using the browser's `Sec-Fetch-Site` and `Origin` headers (the Go 1.25 / `tower-http` scheme), instead of per-request CSRF tokens (changed 2026-10-18).
- **A devices page** lists every session and linked device, and can revoke each one.

### 4.3 Step-up authentication (for sensitive actions)
Since there are no passwords, high-risk actions require a **passkey (WebAuthn)** registered with Slop, or a fresh provider re-login if no passkey is set up:
- Required for **anyone who publishes or receives money:** a passkey for publishers, branch authors with builds, creatives and team admins. Their provider account must also have 2FA (checked where the provider reports it, otherwise required by policy).
- Step-up is needed for:
  - Changing the tracked branch.
  - Registering or changing a maker signing identity (§5.3).
  - Connecting, disconnecting or switching the Stripe account that receives donations.
  - Adding team admins.
  - Transferring an app.
  - Deleting the account.
- **Built so far (2026-10-20):** the fresh re-login, for connecting, switching or disconnecting the Stripe account that receives donations: the session must come from a sign-in in the last 10 minutes (`auth::session::require_recent_sign_in`), else the page asks the user to sign in again. Passkeys and the other actions above come later.

### 4.4 Linking a phone
The computer shows a QR code holding a short-lived (2 min), single-use pairing code. The phone, already signed in to the same account, confirms it. The link is listed on the devices page and can be revoked.

## 5. Software supply chain (the core risk)

### 5.1 From commit to upload
How builds get from source to the player's computer is described in `ARCHITECTURE.md` §6.2–6.5. In short:
- **Slop Build:** Slop writes the build pipeline (a shared, versioned recipe). It runs on the **maker's Git host** (GitHub's free runners at launch) and puts the builds on the **maker's release page**. Slop doesn't compile on its own servers, and **stores no builds or source code**.
- Every public app has **public source**. There's no "upload a build without a repo" option.
- Slop gets **read-only** access to the repo, so it can check the commit and run source scans:
  - **GitHub:** a GitHub App limited to the selected repos.
  - **GitLab / Bitbucket / self-hosted:** read-only deploy tokens.
- **Only the repository's owner can link it** (`crates/slop-api/src/repo_owner.rs`): the publisher's GitHub account ID must be the repository owner's, or they must be an owner of the organization that owns it. Collaborators can't, and a repository can back one app only, so nobody can list (and take donations for) someone else's work.
- Every upload names a **commit SHA**. Slop checks that the commit exists in the linked repo. Tags and branch names aren't trusted, because they can be moved.
- **Build attestations** (a signed CI record of "built from this commit by this workflow"):
  - Built by Slop Build's recipe: **"Built by Slop Build from source"**.
  - Built by the maker's own pipeline, with an attestation: **"Built from source"**.
  - Files the maker attached by hand: **"Uploaded by maker"**, with stricter sandbox checks.
- **The shared recipe is a high-value target.** Every app built with it trusts it. So:
  - Changes need two-person review.
  - Releases of the recipe are signed and pinned by version.
  - Makers' workflow files reference an exact version.
  - The recipe never handles Slop secrets.
- **Files are preferred over installers.** An app that needs its own installer gets a **"Runs its own installer"** label, the strictest sandbox checks, and an extra confirmation before it runs.
- Every file is referenced by its **SHA-256 hash** in a manifest Slop signs. Files live on the **maker's release page**, never with Slop (`ARCHITECTURE.md` §6.4). If one changes after scanning, it no longer matches its hash: installs stop and the app is flagged.
- The scan environment downloads each file once and **deletes it after scanning**.

### 5.2 Scanning
The checks are listed in DESIGN §10. The engineering rules:
- Scanning runs in a **separate cloud account and network**. Analysis workers and sandbox VMs are **ephemeral** (destroyed after each job), have **no credentials**, and only reach the internet through a monitoring proxy (so callbacks can be seen).
- Use **microVM/sandbox isolation** (e.g. Firecracker or gVisor) for static analysis workers, and full VMs for running builds. Keep the hypervisor and images patched.
- The scanner's output is a **verdict signed by the scan service key**. Distribution only accepts signed verdicts, so a compromised core service can't fake one, and a compromised scanner can't publish (§5.3).
- **Built so far (2026-10-20):** a container per file (`deploy/scanner`), started by the API with `--network=none`, `--read-only`, `--cap-drop=ALL`, `no-new-privileges`, memory and process limits, and the file mounted read-only. Static analysis only (ClamAV, YARA, capa); nothing is executed. Its verdict isn't signed yet, and it runs on the same host as the API: separating it is still to do. Signatures update every six hours.
- **Defender checks from makers' CI** are accepted only with a GitHub OIDC token whose repository is the app's repository and whose `job_workflow_ref` is Slop's `check.yml` or the Slop Build recipe, and only for files whose SHA-256 matches. A maker can't send a clean result for a file that wasn't scanned, short of running a modified runner image, which the human review of first releases and Slop's own scan still catch.
- **Known limits**, stated openly: sandbox evasion (code that waits, or detects VMs) and logic bombs can get past. That's why §5.3–5.6 exist.

### 5.3 Signing: the maker first, then Slop (decided)
Every release carries **two signatures**, and the client requires both:

1. **The maker's signature** on the upload. It proves the build came from the maker, not from someone who only stole a Slop session.
   - **Default:** Sigstore "keyless" signing in the maker's CI:
     1. The CI job asks GitHub/GitLab for a short-lived identity token that says "this is workflow Y in repo X, running commit Z".
     2. Sigstore swaps that for a 10-minute certificate, the build's hash is signed with it, and the signature is recorded in Sigstore's public log.
     3. There's no long-lived key for the maker to lose. Slop checks that the certificate's repo and workflow match the app's linked repo. GitHub's `attest-build-provenance` action does all of this.
   - **With Slop Build**, the certificate also records **which shared recipe** ran (its exact version). That proves Slop's recipe built it, from that commit, in that repo.
   - **Files attached by hand:** a signing key registered to the maker's account. Registering or changing it needs step-up auth (§4.3), a 72-hour delay and an email to the maker.
2. **Slop's signature** on the release manifest, added only after the maker's signature is verified and every scan has passed.

**Built (2026-10-18):** GitHub artifact attestations are verified with `sigstore-verify` against Sigstore's public-good root (certificate chain, transparency log, signature, issuer `token.actions.githubusercontent.com`), then checked to name the file's SHA-256 and the app's repository and the release's commit (`crates/slop-provenance`). The API refuses approval without one; the attestation travels in the signed manifest and the client re-verifies it, and refuses updates from a different repository than the installed app's. Not built yet: the maker-registered key for hand-attached files, refreshing the Sigstore root over TUF, and private repositories (GitHub's own Sigstore instance).

Slop's own keys use a **TUF-style** scheme (The Update Framework), so that no single key compromise is catastrophic:

| Role | Signs | Key storage |
|---|---|---|
| **Root** | Which keys are trusted for the other roles | Offline, hardware tokens, 3-of-5 people needed to sign |
| **Targets** | Release manifests (app, version, platform, hashes, verdict reference, the maker's signature) | HSM/KMS, used only by the Distribution service after checking a signed verdict |
| **Snapshot / Timestamp** | The current set of manifests, with expiry | Online KMS, short expiry (stops replay of old or rolled-back manifests) |

- Clients ship with the root keys and **verify both signatures and every hash before installing**. A failed check stops the install, with no option to override.
- Releases go into a public **append-only transparency log**. Makers are **emailed about every release** of their app, so one they didn't make stands out at once.
- **What Slop's signature means:** "this is the file the maker signed, and it passed our checks". It **doesn't** mean "safe" (`LEGAL.md` §5.2, §10).
- **OS code signing** (Windows Authenticode, Apple Developer ID) stays the **maker's** choice and responsibility. Slop doesn't sign other people's apps with its own OS certificate.
  - When apps are installed and launched through the Slop client, Slop's verification takes the place of the OS download checks, the way Steam's does. On Apple Silicon Macs, apps still need at least an ad-hoc signature to run, so Slop Build adds one if it's missing.

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
- Dependency advisories that hit a live release notify the publisher, with a deadline to fix based on severity.

## 6. Runtime protection on the player's device

Apps from Slop run as normal programs with the user's permissions, as on Steam. Slop's protections are:
- the scans;
- the two signatures;
- per-user installs (no admin rights unless the app runs its own installer);
- shared components only from Slop's own verified copies;
- revocation.

Running apps in an OS container (Flatpak or a Slop runtime on Linux, MSIX on Windows) is **optional** and may come later (`ARCHITECTURE.md` §9).
- Store pages show a **"What this app can access"** section: what the publisher declared, compared with what the sandbox scan actually saw. A mismatch fails the scan.
- **Android:** the OS's permission list is shown before install.
- **The Slop Pulse SDK** is small, open source, and pinned by hash. The scan checks that games include the real SDK, not a modified copy.

## 7. The Slop client itself

The client installs software, so it's a prime target:
- **Self-updates are signed and verified** with the same TUF chain as apps.
- **Tauri:** only the Rust commands that are actually needed are exposed, with a strict Content Security Policy, no remote code in the UI, and every IPC argument validated.
- **The `slop://` link handler** treats every link as untrusted input. Links can only open pages or show confirmation dialogs; they can never install or launch anything without a user click.
- Installs run with the **least privilege** needed. On Windows and macOS, per-user installs where possible (no admin prompt).
- Local files (library database, cached manifests) are integrity-checked, so tampering with them can't bypass verification.

## 8. Platform services

### 8.1 Web and API
- Follow the **OWASP ASVS** (level 2 overall, level 3 for payments and publishing).
- **Authorization on every request, on the server.** This is the biggest web risk for Private Slop, chat and People messages, and payment records: every object read checks the caller's rights. Use non-guessable IDs (UUIDv7/ULID) as well, but never *instead of* checks. Postgres row-level security acts as a backstop.
- Rate limits per account, IP and endpoint. Stricter for sign-in, voting, donations, reports and messages. *Built: per IP for every request (`ratelimit.rs`), and 20 reports per account per day (`moderation.rs`).*
- Suspended accounts (`users.suspended_at`) can only read, appeal, accept terms, download their data, sign out or close the account: every other write is refused in the session extractor (`auth/session.rs`). Closed accounts can't sign in at all, and their sessions are revoked.
  - **Built (2026-10-18):** per-IP token buckets for sign-in (20/min), writes (60/min) and reads (600/min), in memory per API instance (`crates/slop-api/src/ratelimit.rs`). Still to do: per-account limits for voting, donations, reports and messages as those features land, and a shared limiter once Slop runs several instances.
- Input validation and output encoding. Store pages and reviews are **Markdown rendered with a strict allowlist** (no raw HTML).
- **Security headers:** a CSP with nonces, HSTS (preload), `frame-ancestors 'none'`, Referrer-Policy, Permissions-Policy.
- Uploads (portfolio images, screenshots) are re-encoded, stripped of metadata and served from a **separate domain** (stops script injection through uploaded files).
  - **Built (2026-10-19), store-page pictures** (`crates/slop-api/src/media.rs`): Slop doesn't host them. Makers list them in `.slop/media.json` in their GitHub repository; Slop reads it at one commit, downloads each picture once to check its format (PNG, JPEG, WebP, GIF, read from the header), size and dimensions, and records its SHA-256. Pages link to `raw.githubusercontent.com` **at that commit**, which can't change, so what's shown is what was reviewed; the page CSP allows only that image host. Every new set waits for review (people or AI reviewers) before it shows. Videos are YouTube links; YouTube's privacy-enhanced player (the one allowed `frame-src`) loads only when someone presses play.

- **AI reviewers (built 2026-10-19, `crates/slop-api/src/review.rs`):** admins make API tokens (`slopadm_…`, stored hashed) that act for them on the admin API only. `read` tokens see the queue and leave notes; `review` tokens also approve and reject. Tokens expire (up to 365 days), can be revoked, stop working when their admin loses admin rights, can't make tokens, and are named in the audit log. The reviewer guide tells them to ignore instructions found inside makers' content (prompt injection) and to leave anything uncertain to a person.

### 8.2 Webhooks (GitHub, payment processor)
- Verify signatures (HMAC / the processor's signing secret) and timestamps, and reject replays.
- Process each event idempotently, by event ID.
- **Don't trust the event body for money.** Fetch the current state from Stripe's API before recording a payment.

### 8.3 Secrets and keys
- No secrets in code or images. Use a secrets manager/KMS with automatic rotation.
- Signing keys are in an HSM/KMS (§5.3), and their use is logged and alerted on.
- Separate keys per environment. Staging can never sign production releases.
- **Makers' repository tokens** (built 2026-10-20, `repo_tokens.rs`): read-only tokens for private repositories outside GitHub, checked before they're saved, encrypted with ChaCha20-Poly1305 under `SLOP_REPO_TOKEN_KEY` with the app's ID as associated data, never returned by the API, and only sent to the repository's own host (checked against private networks) in the `Authorization` header, which the HTTP client drops on a redirect to another host. Saving one is audit-logged. Today the key sits in the same env file as the others; it moves to the KMS with them.

### 8.4 Staff and admin access
- SSO with **hardware security keys** for all staff, and short-lived just-in-time production access.
- Admin tools have **roles** (moderator, support, finance, security). Sensitive actions need two people: revocations, transferring a disputed app to another account, disabling donations on an app, and account removals.
- Every admin action is written to an **append-only audit log**, visible to the security team.

### 8.5 Chat: end-to-end encryption (decided)
Slop **can't** read chat messages (`LEGAL.md` §4.6).
- **Protocol:** **MLS** (Messaging Layer Security, RFC 9420), which handles one-to-one and group chats (player teams) with many devices.
  - Use **OpenMLS**, an MIT-licensed Rust library that fits the Tauri app's Rust core.
  - Signal's library is an alternative, but it's AGPL-licensed, which would affect Slop's own app licence.
- **Keys:** each device creates its own keys and keeps the private ones in the OS secure store (§4.2). Slop's server only stores public key packages, plus **encrypted** messages waiting to be delivered.
- **New devices:**
  - A new device is added by an existing one (QR pairing, §4.4). The server can't add a device on its own, so a stolen Slop session can't quietly read chats.
  - **Encrypted history backup is on by default** (decided; users can turn it off). Old messages reach a new device or a reinstall through it.
    - The backup is encrypted **on the device** with a key **only the user holds**. Slop stores only the encrypted backup and **never** has the key, so backups stay end-to-end.
    - **The key:** derived from the user's **passkey** where the platform supports it (WebAuthn's PRF extension). Otherwise it's a **recovery code** shown once at set-up, which the user must save.
    - If the user loses both the passkey and the recovery code, the history is gone. Slop can't recover it, and the app says so plainly.
    - Turning backup off deletes the stored copy.
- **Verifying people:** each chat has a safety code or QR code users can compare. Everyone in a chat is warned when someone's keys change.
- **Reports:** the reporter's app sends the reported messages, decrypted, with the reporter's consent.
  - Add **message franking**: a cryptographic tag that lets Slop check a reported message is genuine without being able to read any others. That way reports can't be faked.
- **Attachments:** encrypted on the device before upload. Slop stores only the encrypted file, with a size limit and automatic deletion after 30 days. Slop can't see what's in them.
- **What Slop still sees (metadata):** who is in a chat, when messages are sent, and their sizes. Keep the minimum and delete it quickly (e.g. delivery records after 30 days).
- **Website:** browser encryption is weaker, because the page's code is delivered fresh by the server each time. **Chat runs in the Slop app only** (desktop and phone). The website shows "open the Slop app to chat".
- **Built 2026-10-20** (`crates/slop-client/src/chat/`, `crates/slop-api/src/chat.rs`):
  - OpenMLS 0.9, ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`. Each device's credential is `slop:{user}:{device}`; its private keys and MLS state live in a local file encrypted (XChaCha20-Poly1305) with a key from the OS secure store. Without a secure store, chat refuses to start rather than write keys to a plain file.
  - **Devices:** registering needs a desktop sign-in (never the website). The first device stands alone; later ones are approved by an existing device signing `slop-chat-device-approval:v1:{user}:{device}:{key}` after the user types the 9-character code the new device shows. Slop checks the signature, and every app checks the whole chain back to the user's first device before adding a device to a chat. Trust on first use: if a person's first device changes ("start over", which needs a fresh sign-in), each chat with them says their security code changed. Security codes (eight groups of five digits) can be compared per person.
  - **Ordering:** a commit must name the epoch Slop last saw and moves it on; a message must use the current epoch; losers fetch and retry. Any member's app keeps a chat's devices in step with its members (adding new and approved devices, removing revoked ones and people who left), the group's first device at once and others after 45 seconds, so commits rarely race.
  - **Delivery:** long-polling (`/chat/sync`, 25 s), woken in-process. Messages, Welcomes and attachments are deleted after 30 days; key packages of removed devices and unapproved devices after a day.
  - **Attachments** are encrypted on the device with a fresh key each (in the message), checked by SHA-256 after decrypting, 15 MB at most. **Link previews and linked media** are fetched by the sender's app only, from public addresses (no private networks), with size and time limits.
  - **Franking:** each message carries a random key; Slop stores HMAC-SHA256(key, plaintext) with the message. A report sends that one plaintext and key, and Slop checks the tag.
  - **History:** a newly approved device gets a copy of the history from the approving one through the user's own "self" chat. The backup (on by default) is encrypted with a key derived by Argon2id from a 24-character recovery code, salted with the user ID; turning it off deletes Slop's copy. The key reaches the user's other devices through the self chat; the code itself is shown once.
  - **Typing indicators** are the one unencrypted signal: Slop holds "this person is typing in this chat" in memory for 6 seconds and passes it to the chat's members; nothing is written down.
  - **Notifications:** desktop notifications are made on the device from the decrypted message. Web push (`push.rs`) sends empty pushes signed with Slop's VAPID key, only to the known push services (so a subscription can't point Slop's server elsewhere); the phone fetches the notice itself, so push services learn only timing. Server notices about chats never contain message text.
  - **Overlay:** the desktop app registers Shift+Tab system-wide only while the window in front belongs to a process Slop launched (or its children) and fills its screen, and releases it otherwise. It reads only the foreground window's process, size and fullscreen state; nothing is injected into games.
  - Not yet: passkey-derived backup keys (PRF), message expiry on the device, and a phone app.
- **People job conversations** (decided 2026-10-19, `DESIGN.md` §15.5) aren't end-to-end encrypted: they're recorded, which both sides are told. Slop holds the text only while open and 30 days after closing, then keeps SHA-256 fingerprints that prove each side's downloaded copy is unchanged. Staff read one only from a copy a side submits for a problem or review dispute, and every mediator look at a job record is audit-logged.

## 9. Payment security
More detail in `PAYMENTS.md`.
- **Slop never holds money** (`LEGAL.md` §3.1). Payments are direct charges on makers' and creatives' own Stripe accounts, so there are no Slop balances to steal.
- **PCI DSS:** card data is entered only into Stripe's hosted fields or page, which keeps Slop on the smallest self-assessment (SAQ A). The pages that host checkout still need a strict CSP and monitoring for script changes (PCI DSS v4 requirements 6.4.3 and 11.6.1).
- **Bank details live in Stripe, not Slop.** Changing them happens in the maker's Stripe dashboard, protected by Stripe's own security.
- **The main risk is a hijacked account pointing donations at a different Stripe account.** Connecting, disconnecting or switching the Stripe account requires step-up auth, a 72-hour delay, and a notice to the account's email and devices. (Built 2026-10-20 for donations; the delay applies to switching and disconnecting, while a first connection needs only the fresh sign-in. People's Stripe account comes next.)
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
  - UK users are found from several signals (IP, billing, app-store and stated country). Slop stores only the resulting country. No VPN detection.
  - UK accounts start in an **under-18 view**: 18+ content hidden, messages from connections only.
  - A third-party age check unlocks 18+ content. Slop stores only "passed", the date and the method, never ID or face images (`LEGAL.md` §6).
  - Content ratings on store pages (`LEGAL.md` §5.7).
- **Deleting an account:** personal data is removed or anonymized. Votes become anonymous (not deleted, to keep rating history honest). Payment records stay for the legal retention period, with the personal link removed where the law allows.

## 11. Trust & safety (abuse of the platform)

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

## 12. Private Slop

- **Access control:** a private app's metadata, builds and security report are visible only to the owner and the users/player teams it's shared with. This is checked on every request. Shared content is never sent to official teams (DESIGN §13), and this is enforced on the server, not just in the UI.
- **Private repos are allowed for Private Slop.** Public apps still need a public, open-source repo (DESIGN §8). The owner gives Slop permission to read the code, so it can be checked and scanned like any other app.
- **Builds stay on the owner's private release page.** People it's shared with download through **short-lived links** that Slop gets from GitHub. Slop never stores or relays the files (`ARCHITECTURE.md` §6.5).

### 12.1 Granting Slop access (all at launch)
- **GitHub:**
  1. The owner installs the **Slop GitHub App** and picks **"Only select repositories"**, choosing just this repo.
  2. The app asks for the minimum: **Contents: read** and **Metadata: read**. No write access, no access to issues, secrets or other repos.
  3. Slop uses short-lived (1-hour) installation tokens:
     - the **isolated scan environment** uses them to read code and builds;
     - the download service uses them only to request short-lived download links for people the app is shared with.
- **GitLab / Bitbucket / self-hosted Git:** a **read-only deploy token** limited to that one repo (e.g. GitLab `read_repository`). It's stored in the secrets manager and used only by the scan environment. Sharing builds from these hosts depends on `ARCHITECTURE.md` §6.5.
- **There's no source-upload option.** Code always comes from a Git host.
- The **Publish** screen shows exactly what access Slop has, and a "Check access" button.
- **Scan findings** are shown to the owner. People the app is shared with see the verdict, but no code excerpts. **Leaked secrets** are reported by file and line, with the value hidden.

### 12.2 Slop keeps no copies (decided)
Slop **doesn't store makers' source code or builds**, public or private (decided 2026-10-17). It fetches them from the repo whenever it needs to scan, and deletes them afterwards.
- **If the owner removes Slop's access, or deletes the repo or release:**
  - existing builds can't be re-scanned, so they're **frozen** (no updates);
  - after a **14-day grace period**, they're removed from sharing (Private Slop) or delisted (public apps);
  - installed copies keep working unless they're revoked for a security reason (§5.5).
- **What Slop keeps:** hashes, scan reports and signed manifests. That's enough to prove what was scanned and when (`LEGAL.md` §13), without keeping the software.

**Built 2026-10-20** (`crates/slop-api/src/frozen.rs`): a daily check of every app's repository through GitHub's API (or, on other hosts, Git's anonymous discovery request for public repositories), plus "Check now" for the owner. A public app whose repository turns private counts as lost access. Frozen apps take no releases or approvals; public ones are delisted after 14 days and listed again when access returns; Private Slop stops being shared after 14 days.

### 12.3 Making a private app public
The repo has to become public under an OSI-approved licence, followed by a fresh full scan.
**Built 2026-10-20:** the owner asks from the Manage page; Slop checks the repository is readable without signing in; it waits in the admin queue, where listing it checks again and makes it public. Until the scan pipeline exists, the "fresh full scan" is the reviewer's.

## 13. Detection, response and disclosure

- **Monitoring:** alerts on Slop signing-key use, maker signing-identity changes, revocations, admin actions, Stripe account changes, scan pipeline failures, spikes in reports, and payment-record mismatches.
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

## 14. What the mockup simulates

The mockup's scan results, security reports, sign-in, Pulse handshakes and reports are all faked in the browser, using invented data. None of these controls exist in the mockup.

## 15. Decisions (2026-10-17) and open questions

**Decided:**
- **Slop Build:** Slop writes the pipeline, and it runs on the maker's Git host. The pipeline (or the maker) signs, then Slop signs after scanning.
- **Slop stores no builds or source code.** Builds live on makers' release pages. Every public app has public source.
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
