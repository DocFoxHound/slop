# Code signing policy

**Status:** not signed yet. The certificate will come from [SSL.com](https://www.ssl.com) (a
code-signing certificate with eSigner cloud signing, so the pipeline signs each approved release
without a key on anyone's computer). Until it's set up, the SLAPorSLOP app for Windows is published
unsigned. (SignPath Foundation declined the project in October 2026.)

## What is signed

Only the SLAPorSLOP desktop app for Windows, built from this repository: the program (`slop.exe`) and
its installer (`Slop_<version>_x64-setup.exe`), downloaded from
[slaporslop.games/download](https://slaporslop.games/download). Nothing else is signed with this
certificate: not third-party programs or libraries, and not the apps and games people publish on
SLAPorSLOP (their makers sign those themselves).

## How a release is built and signed

1. Changes are made on the `dev` branch and tried on SLAPorSLOP's test server. Every push to `dev`
   runs the [pipeline](../.github/workflows/pipeline.yml)'s checks (lint, the API tests, the
   website tests and the browser tests) on GitHub's own runners.
2. When they're ready, `main` moves forward to them, only to a commit that passed every check,
   and the pipeline builds the Windows app from that commit on GitHub's Windows runner.
3. **Nothing is released or signed until a release approver approves that run** (a GitHub
   environment with a required reviewer). Every release is approved by hand.
4. The approved build is published to slaporslop.games, and installed SLAPorSLOP apps update to it. The
   update is also signed with SLAPorSLOP's own updater key, which the app checks before installing.

The build scripts and pipeline are in this repository and are reviewed like any other change.

## The signing allowance

SSL.com's eSigner plan allows a set number of signings a month (Tier 1: 20), and each release
signs about three files (`slop.exe`, its uninstaller and the installer). `deploy/signing.conf` holds
the numbers; `deploy/signing-allowance.sh` counts the pipeline's runs on `main` since the allowance
last started again. `deploy/promote.sh` shows the count before every release and, once signing is
on (`SIGNING_ON=1`), refuses a release that would go over it. The pipeline's Windows build checks
again and builds a release past the allowance unsigned rather than signing it. Releases collect on
`dev` and go to production a few at a time, so the allowance lasts the month.

## Team roles

| Role | Who |
|---|---|
| Committers and reviewers (may change the source; review everyone else's changes) | [DocFoxHound](https://github.com/DocFoxHound) |
| Approvers (approve each signed release) | [DocFoxHound](https://github.com/DocFoxHound) |

Everyone in these roles uses multi-factor authentication for GitHub and SSL.com.

## Privacy

The SLAPorSLOP app connects to SLAPorSLOP's service at slaporslop.games, which is what it's for: signing in,
the store, your library, updates, ratings and chat. What SLAPorSLOP collects and why is in the
[Privacy Policy](https://slaporslop.games/legal/privacy).

Besides slaporslop.games, the app only contacts:

- **the release pages of the apps you install** (usually GitHub), to download them;
- **Microsoft's own download links**, when an app you're about to play needs a Windows runtime
  (such as the Visual C++ Redistributable) and you agree to install it;
- **sites you link to in chat**, when you send a link with a preview (fetched by your app, and
  you can turn previews off).

It sends nothing to any other networked system unless you ask it to.
