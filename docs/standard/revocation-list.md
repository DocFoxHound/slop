# Revocation list

**Purpose prefix:** `slop/revocation-list/v1` · **Schema:** [`schemas/revocation-list.schema.json`](schemas/revocation-list.schema.json) · **Example:** [`vectors/revocation-list.json`](vectors/revocation-list.json)

The kill switch: releases Slop withdrew after approving them, such as malware found by a re-scan.
A conforming installer or launcher refuses to install or start a release on the list.

`GET /v1/revocations` returns a freshly signed list in the
[signed envelope](README.md#the-signed-envelope). It's valid for six hours; fetch it at least that
often, and treat an expired list as no list at all. The expiry stops an attacker replaying an old
list from before a release was revoked.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `schema` | integer | `1` |
| `issued_at` | string | When it was signed (RFC 3339) |
| `expires_at` | string | When it stops being valid (RFC 3339) |
| `revoked` | array | Every revoked release, oldest first |

Each entry:

| Field | Type | Meaning |
|---|---|---|
| `release_id` | string | The release, as in its manifest's `release_id` |
| `app_slug` | string | The app's address |
| `reason` | string | Why, in plain words |
| `revoked_at` | string | When (RFC 3339) |

Releases of private apps are listed too, by ID and address only, so a player who installed one
is protected.
