# Release manifest

**Purpose prefix:** `slop/release-manifest/v1` · **Schema:** [`schemas/release-manifest.schema.json`](schemas/release-manifest.schema.json) · **Example:** [`vectors/release-manifest.json`](vectors/release-manifest.json)

SLAPorSLOP signs a release manifest when a person approves a release. It lists every file of the
release. The files themselves stay on the maker's release page; the manifest is what makes them
safe to install from there, since a file whose hash doesn't match is refused.

`GET /v1/apps/{slug}/manifest` returns the manifest of the release SLAPorSLOP currently serves for an
app, in the [signed envelope](README.md#the-signed-envelope). It answers 404 for an app the asker
can't see.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `schema` | integer | `1` |
| `app_id` | string | The app's ID (a UUID) |
| `app_slug` | string | Its address on SLAPorSLOP: the store page is `/app/{app_slug}` |
| `app_title` | string | Its name |
| `maker` | string | The maker's public name |
| `repo_url` | string | The app's source repository |
| `release_id` | string | The release's ID (a UUID), as named by revocation lists |
| `version` | string | The release's version |
| `commit` | string | The 40-character commit the release was built from |
| `files` | array of [files](#a-file) | One per platform |
| `approved_at` | string | When a person approved it (RFC 3339) |
| `mod_for` | [app](#an-app) | Optional. A mod: the game it's for. Install it only into that game |
| `build_of` | [app](#an-app) | Optional. A community build: the original it's a build of |
| `mods_folder` | [mods folder](#a-mods-folder) | Optional. Where the app reads mods from, when the maker named a folder |
| `runtimes` | array of strings | Optional. Shared runtimes the Windows build needs, by ID from SLAPorSLOP's approved list (`GET /v1/runtimes`) |

### A file

| Field | Type | Meaning |
|---|---|---|
| `platform` | string | `windows`, `mac`, `linux`, `android`, `linux-deb`, `linux-rpm`, or `any` (a mod, the same for every platform) |
| `kind` | string | `archive` (a `.zip` or `.tar.gz` of the app's files, installed into a folder), `installer` (the maker's own installer, run only after the player agrees), or `apk` |
| `url` | string | Where to download it: the maker's release page. For a file in parts, the joined file's name, not downloaded itself |
| `file_name` | string | Its file name |
| `size` | integer | Its size in bytes |
| `sha256` | string | Lowercase hex SHA-256 of the whole file |
| `launch` | object or null | For archives: `exec`, the program's path inside the archive (`/` separators, no `..`, no leading `/`), and `args`, its arguments |
| `attestation` | string or null | The maker's signature: a [Sigstore bundle](https://docs.sigstore.dev/about/bundle/) (JSON text) for a SLSA provenance statement naming this file's SHA-256 |
| `maker_signature` | object | Optional, when there's no attestation: `public_key` (`ssh-ed25519 …`, registered on the maker's SLAPorSLOP account) and `signature` (an armored OpenSSH signature) |
| `parts` | array | Optional. For files over 2 GiB: each part's `url`, `size`, `sha256` and `attestation`, joined in order |

### An app

`app_id`, `app_slug` and `app_title`, as above.

### A mods folder

| Field | Type | Meaning |
|---|---|---|
| `base` | string | `game` (inside the installed files) or `saves` (inside the app's save folder) |
| `path` | string | Relative to the base: `/` separators, no `..`, no leading `/`, 1–200 characters |
| `layout` | string | `folder` (each mod in a folder named after its SLAPorSLOP address) or `flat` (every mod's files directly in the folder) |

## The maker's signature

A release carries its maker's signature as well as SLAPorSLOP's, so a stolen SLAPorSLOP account or a
compromised SLAPorSLOP server can't publish a build the maker didn't make.

**Attestation** (builds made in CI, including Slop Build): a GitHub artifact attestation. Check
the bundle against Sigstore's public-good root (or GitHub's own Sigstore instance, for private
repositories), with the issuer `https://token.actions.githubusercontent.com`. Then check the
statement: `predicateType` is `https://slsa.dev/provenance/v1`, a subject's `sha256` digest is the
file's (or the part's), and the signing certificate names the manifest's `repo_url` and `commit`.
When the signing workflow is Slop Build's recipe, SLAPorSLOP shows "Built by Slop Build from source".

**SSH signature** (files a maker built and attached themselves): an OpenSSH signature
(`ssh-keygen -Y sign -n slop-release -f key file`) in the
[PROTOCOL.sshsig](https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.sshsig) format.
Only `ssh-ed25519` keys and the `sha512` or `sha256` message hashes are valid, and the namespace
must be `slop-release`, so a signature for a login or a git commit can't be reused.

## Installing

See [Checking a release before installing it](README.md#checking-a-release-before-installing-it).
An installer must also refuse an `exec`, mods-folder `path` or archive entry that would land
outside the install folder.
