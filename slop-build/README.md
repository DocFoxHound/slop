# Slop Build

The shared build recipe makers use to ship on Slop (`docs/ARCHITECTURE.md` §6.2). A maker adds
one small workflow file to their repository (Slop's **Add Slop Build** button fills it in; see
`examples/slop.yml`), and each push to their main branch or `v*` tag:

1. builds the app for Windows and Linux on GitHub-hosted runners,
2. packs each build (Windows: a `.zip`; Linux: a `.tar.gz` plus `.deb` and `.rpm` packages that
   install to `/opt/<app>` with `/usr/bin/<app>`) and signs every file with a GitHub artifact
   attestation (Sigstore), which names the repository, the commit and this recipe ("Built by Slop
   Build from source"),
   On Windows it first signs the program with the maker's free SignPath Foundation certificate
   (optional), then scans the build with Microsoft Defender and the day's definitions and records
   the result and the signer. A separate job runs gitleaks (secrets) and OSV-Scanner (vulnerable
   dependencies) on the source; the results go to Slop for the maker and the reviewer.
3. attaches the files to a release on the maker's repository (Slop stores no builds),
4. sends the release to Slop with the run's GitHub OIDC token. Slop checks the token was issued
   to this recipe, in the app's repository, at this commit; no secret is shared.

Slop then fetches and hashes the files, verifies the attestations, and the release goes to review.

## Project types

| `project-type` | What it does |
|---|---|
| `rust` | `cargo build --release --locked`; ships the first binary target |
| `go` | `go build -trimpath`; ships a binary named after the repository |
| `dotnet` | `dotnet publish -c Release --self-contained` of the program's `.csproj` (`dotnet-project` if there are several) |
| `godot` | Installs Godot and export templates (`godot-version`), imports, exports the `Windows Desktop` / `Linux` presets; `godot-dotnet: true` uses Godot .NET for C# projects |
| `unity` | GameCI's `unity-builder` on Linux runners (StandaloneWindows64 / StandaloneLinux64); needs the maker's `UNITY_LICENSE` or `UNITY_SERIAL`, `UNITY_EMAIL` and `UNITY_PASSWORD` secrets |
| `python` | PyInstaller `--onedir` of `python-entry`, after `requirements.txt` |
| `electron` | `npm ci`, electron-builder `--dir`; ships the unpacked folder |
| `tauri` | `npm ci` (if any), `tauri build --no-bundle`; ships the program |
| `web` | Builds the site, then wraps `output-dir` in a minimal Tauri shell generated in the run |
| `custom` | Runs `build-command`, ships `output-dir`, starts `exec` (`{ext}` becomes `.exe` on Windows) |

An optional `slop.toml` (`[launch]` and `[launch.<platform>]`, with `exec` and `args`) sets the
program and arguments, and a top-level `runtimes = [...]` names the shared runtimes the Windows
build needs (Slop's `GET /v1/runtimes`). `publish: false` builds and checks without a release, which is how the
types are tried on GitHub's runners (all built on Windows and Linux, 2026-10-20; Unity up to
licence activation, as there's no test licence).

GitHub release files are limited to 2 GiB: a bigger `.zip` or `.tar.gz` is split into
`<name>.001`, `<name>.002`… (up to 64), each attested; the desktop app checks and joins them.
`.deb`/`.rpm` packages over the limit stop the build.

## Inputs

| Input | Meaning |
|---|---|
| `app` | The app's store address on Slop (required) |
| `slop-url` | The Slop site (required) |
| `project-type` | `rust`, `go`, `dotnet`, `godot`, `unity`, `python`, `electron`, `tauri`, `web` or `custom` (required) |
| `platforms` | `windows,linux` by default |
| `linux-packages` | `deb,rpm` by default; Slop keeps the ones the store page lists |
| `build-command`, `output-dir`, `exec` | `custom` only |
| `sign` | `true` by default; `false` for private repositories without GitHub Enterprise |
| `project-path` | Where the project is (default `.`) |
| `godot-version`, `godot-preset-windows`, `godot-preset-linux` | `godot` |
| `godot-dotnet` | `godot`: the project uses C# |
| `dotnet-version`, `dotnet-project` | `dotnet` and C# Godot (`8.0.x` by default) |
| `python-version`, `python-entry`, `python-windowed` | `python` |
| `node-version` | `electron`, `tauri`, `web` |
| `linux-runtime` | `steam-sniper`: `rust`, `go` and `custom` compile Linux inside Valve's Steam Runtime 3 (sniper) SDK container, so the program needs only glibc 2.31 (tried with podman, 2026-10-21). The image is pinned to version 3.0.20260928.262393 by digest, so a change at Valve can't alter builds; moving it is a recipe change. Godot and Unity ship their engine's own prebuilt player (Godot 4.4 needs glibc 2.28), so they don't need it: the Pong example's 16 tests pass inside sniper |
| `publish` | `false`: build and check only, no release, nothing sent to Slop |
| `itch-target` | Optional: also push each build to itch.io (`user/game`) with butler 15.32.0, checked by hash; needs the `BUTLER_API_KEY` secret |
| `flatpak-app-id` | Optional: attach a draft Flatpak manifest for the Linux build to each release (`io.github.you.Game`) |
| `signpath-organization-id` | Optional: sign the Windows program with SignPath (needs the `SIGNPATH_API_TOKEN` secret) |
| `signpath-project-slug` | SignPath project; defaults to `app` |
| `signpath-signing-policy-slug` | `release-signing` by default |

## Slop Check

`.github/workflows/check.yml` is for makers who build with their own CI (`examples/check.yml`).
When they publish a release, it downloads the release's Windows files (`.zip`, `.exe`, `.msi`,
`.msix`) on a Windows runner, scans them with Microsoft Defender, records each file's SHA-256 and
signer, and posts the result to `/v1/slop-build/checks` with the run's OIDC token. Slop accepts it
only from this workflow (or the build recipe), in the app's own repository.

## Where this lives

This folder is the recipe's source, published as its own public repository
(`DocFoxHound/slop-build`, at `.github/workflows/build.yml`) with:

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
actionlint slop-build/.github/workflows/build.yml slop-build/.github/workflows/check.yml
```

The first real run needs a sample repository on GitHub.

## Licence

The recipes are under the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option, so using them never changes your app's licence.
