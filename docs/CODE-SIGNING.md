# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

**Status:** applied for. Until SignPath Foundation approves the project, the Slop app for Windows
is published unsigned.

## What is signed

Only the Slop desktop app for Windows, built from this repository: the program (`slop.exe`) and
its installer (`Slop_<version>_x64-setup.exe`), downloaded from
[slopai.games/download](https://slopai.games/download). Nothing else is signed with this
certificate: not third-party programs or libraries, and not the apps and games people publish on
Slop (their makers sign those themselves).

## How a release is built and signed

1. Changes are made on the `dev` branch and tried on Slop's test server. Every push to `dev`
   runs the [pipeline](../.github/workflows/pipeline.yml)'s checks (lint, the API tests, the
   website tests and the browser tests) on GitHub's own runners.
2. When they're ready, `main` moves forward to them, only to a commit that passed every check,
   and the pipeline builds the Windows app from that commit on GitHub's Windows runner.
3. **Nothing is released or signed until a release approver approves that run** (a GitHub
   environment with a required reviewer). Every release is approved by hand.
4. The approved build is published to slopai.games, and installed Slop apps update to it. The
   update is also signed with Slop's own updater key, which the app checks before installing.

The build scripts and pipeline are in this repository and are reviewed like any other change.

## Team roles

| Role | Who |
|---|---|
| Committers and reviewers (may change the source; review everyone else's changes) | [DocFoxHound](https://github.com/DocFoxHound) |
| Approvers (approve each signed release) | [DocFoxHound](https://github.com/DocFoxHound) |

Everyone in these roles uses multi-factor authentication for GitHub and SignPath.

## Privacy

The Slop app connects to Slop's service at slopai.games, which is what it's for: signing in,
the store, your library, updates, ratings and chat. What Slop collects and why is in the
[Privacy Policy](https://slopai.games/legal/privacy).

Besides slopai.games, the app only contacts:

- **the release pages of the apps you install** (usually GitHub), to download them;
- **Microsoft's own download links**, when an app you're about to play needs a Windows runtime
  (such as the Visual C++ Redistributable) and you agree to install it;
- **sites you link to in chat**, when you send a link with a preview (fetched by your app, and
  you can turn previews off).

It sends nothing to any other networked system unless you ask it to.
