# File verification

**Purpose prefix:** `slop/file-verification/v1` · **Schema:** [`schemas/file-verification.schema.json`](schemas/file-verification.schema.json) · **Example:** [`vectors/file-verification.json`](vectors/file-verification.json)

"Does SLAPorSLOP know this file?" for anyone, by the file's SHA-256: another store that wants to show
"verified by SLAPorSLOP", a launcher, or a player checking a download. No account is needed.

`GET /v1/verify/{sha256}` returns a signed answer in the
[signed envelope](README.md#the-signed-envelope), valid for 24 hours. The hash is 64 hex
characters (either case; the answer uses lowercase); anything else gets a 422 error.

## What it names

- Approved releases of apps that are **public and in the store**, as a visitor with no account
  would see them.
- **Revoked** releases of public apps, even once the app has left the store, since that's the
  answer that matters most.
- Never private apps. An unknown file and a private one get the same answer: no matches.
- Whole files only. A part of a file released in parts isn't looked up by its own hash.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `schema` | integer | `1` |
| `sha256` | string | The file asked about (lowercase hex) |
| `issued_at` | string | When it was signed (RFC 3339) |
| `expires_at` | string | When it stops being valid (RFC 3339); ask again after |
| `matches` | array | Releases with this file, newest first, at most 20; empty when SLAPorSLOP doesn't know it |

Each match:

| Field | Type | Meaning |
|---|---|---|
| `app_slug`, `app_title`, `maker` | strings | The app, as in its manifest |
| `release_id`, `version` | strings | The release |
| `platform` | string | Which of the release's files this is |
| `status` | string | `approved` or `revoked` |
| `approved_at` | string | When a person approved it (RFC 3339) |
| `revoked_at` | string | Optional. When it was revoked |
| `scan` | string | SLAPorSLOP's scan of the file: `passed`, `note` (passed, with something a person looked at), `on_hold` (a re-scan found something a person is checking), `rejected`, `blocked`, or `not_scanned` (this SLAPorSLOP doesn't scan files) |
| `defender` | string | Optional, for Windows files: Microsoft Defender's check, `clean` or `detections` (a reviewer judged them false positives before approving) |
| `page_url` | string | The app's store page |

A file can match more than one release, for example when a maker re-releases the same build.

## Showing it

See [Using "verified by SLAPorSLOP"](README.md#using-verified-by-slaporslop). Say "scanned" or
"verified by SLAPorSLOP", never "safe": a scan lowers the risk, it doesn't remove it.
