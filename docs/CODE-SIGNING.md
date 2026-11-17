# Code signing policy

**Status:** signed with **Azure Artifact Signing** from 2026-11-17 (account and certificate profile
`SLAPorSLOP`, identity validated as an individual: the publisher Windows shows is **Brian Martin**).
The pipeline signs each approved release on GitHub's Windows runner, with no key on anyone's
computer: it signs in to Azure with GitHub's OIDC token, which Azure accepts only from this
repository's `production` environment. (SignPath Foundation declined the project in October 2026;
SSL.com's eSigner was planned before Azure.)

## What is signed

Only the SLAPorSLOP desktop app for Windows, built from this repository: the program and
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
   environment with a required reviewer). Every release is approved by hand, twice: first the
   Windows build, which signs (Azure trusts only that environment), then production's deploy.
   Tauri signs the program, its uninstaller and the installer as it builds them (`signtool` with
   Microsoft's Artifact Signing client, `microsoft.artifactsigning.client`, pinned by version and
   SHA-256), each with Microsoft's timestamp so the signature outlasts the 3-day certificate, and
   the pipeline checks the signatures before uploading.
4. The approved build is published to slaporslop.games, and installed SLAPorSLOP apps update to it. The
   update is also signed with SLAPorSLOP's own updater key, which the app checks before installing.

The build scripts and pipeline are in this repository and are reviewed like any other change.

## The signing allowance

Azure Artifact Signing's plan allows a set number of signatures a month (Basic: 5,000), and each
release signs about three files (the program, its uninstaller and the installer). `deploy/signing.conf` holds
the numbers; `deploy/signing-allowance.sh` counts the pipeline's runs on `main` since the allowance
last started again. `deploy/promote.sh` shows the count before every release and, once signing is
on (`SIGNING_ON=1`), refuses a release that would go over it. The pipeline's Windows build checks
again and builds a release past the allowance unsigned rather than signing it. At 5,000 a month the
allowance is a safety net rather than a limit.

## Team roles

| Role | Who |
|---|---|
| Committers and reviewers (may change the source; review everyone else's changes) | [DocFoxHound](https://github.com/DocFoxHound) |
| Approvers (approve each signed release) | [DocFoxHound](https://github.com/DocFoxHound) |

Everyone in these roles uses multi-factor authentication for GitHub and Azure.

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
