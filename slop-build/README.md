# Slop Build

The shared build recipe makers use to ship on Slop (`docs/ARCHITECTURE.md` §6.2). A maker adds
one small workflow file to their repository (Slop's **Add Slop Build** button fills it in; see
`examples/slop.yml`), and each push to their main branch or `v*` tag:

1. builds the app for Windows, macOS and Linux on GitHub-hosted runners,
2. zips each build and signs it with a GitHub artifact attestation (Sigstore), which names the
   repository, the commit and this recipe ("Built by Slop Build from source"),
3. attaches the zips to a release on the maker's repository (Slop stores no builds),
4. sends the release to Slop with the run's GitHub OIDC token. Slop checks the token was issued
   to this recipe, in the app's repository, at this commit; no secret is shared.

Slop then fetches and hashes the files, verifies the attestations, and the release goes to review.

## Project types

| `project-type` | What it does |
|---|---|
| `rust` | `cargo build --release --locked`; ships the first binary target |
| `go` | `go build -trimpath`; ships a binary named after the repository |
| `custom` | Runs `build-command`, ships `output-dir`, starts `exec` (`{ext}` becomes `.exe` on Windows) |

Godot, web/HTML5, Electron/Tauri and Python projects use `custom` for now; built-in support comes
next (`docs/ARCHITECTURE.md` §6.2).

## Where this lives

This folder is the recipe's source. Before makers rely on it, it moves to its own public
repository (`DocFoxHound/slop-build`, at `.github/workflows/build.yml`), with:

- releases tagged and makers pinning a tag (`@v1`);
- two-person review for every change (it's trusted by every app built with it, SECURITY §5.1);
- the API configured to match: `SLOP_BUILD_WORKFLOW=DocFoxHound/slop-build/.github/workflows/build.yml`
  and `SLOP_BUILD_VERSION=v1`.

## Checking it

GitHub Actions can't run locally. The recipe and the workflow Slop generates for makers are linted
with [actionlint](https://github.com/rhysd/actionlint), which also checks the inputs makers pass
against the recipe's:

```sh
cargo test -p slop-api print_sample_workflows -- --ignored --nocapture   # sample maker workflows
actionlint slop-build/.github/workflows/build.yml
```

The first real run needs a sample repository on GitHub.
