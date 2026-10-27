# The SLAPorSLOP open standard

**Version 1 · 2026-10-21 · Apache-2.0 OR MIT**

How SLAPorSLOP says, in a way anyone can check, which files it scanned and approved, who built them,
and which it has withdrawn. Another store, a launcher, a package manager or a curious player can
use these documents without trusting a SLAPorSLOP server: everything is signed, and the signatures are
checked against keys you pin yourself.

| Document | Says | Where | Purpose prefix | Spec |
|---|---|---|---|---|
| Release manifest | Every file of an approved release: where it is, its hash and size, how to start it, and the maker's own signature | `GET /v1/apps/{slug}/manifest` | `slop/release-manifest/v1` | [release-manifest.md](release-manifest.md) |
| Revocation list | Releases that must no longer be installed or started | `GET /v1/revocations` | `slop/revocation-list/v1` | [revocation-list.md](revocation-list.md) |
| File verification | What SLAPorSLOP knows about one file, by its SHA-256: its releases, scan results and whether they were revoked | `GET /v1/verify/{sha256}` | `slop/file-verification/v1` | [file-verification.md](file-verification.md) |

Paths are relative to a SLAPorSLOP site, such as `https://slop.example`. Each document has a JSON Schema
in [`schemas/`](schemas/), and [`vectors/`](vectors/) holds examples signed with a published test
key.

## Tools

- **`slop-verify`** (`crates/slop-verify`): a command-line checker for all three documents,
  including the downloaded files and the makers' signatures. `cargo install --path
  crates/slop-verify`, then `slop-verify` prints how to use it.
- **`slop-manifest`** (`crates/slop-manifest`): the Rust types, signing and checking.
- **`slop-provenance`** (`crates/slop-provenance`): checks the makers' Sigstore attestations.

All three are under Apache-2.0 OR MIT, so they can go into any store or launcher, open or closed.

## The signed envelope

Every document travels in the same envelope:

```json
{
  "payload": "{\"schema\":1,...}",
  "signature": "base64 Ed25519 signature (64 bytes)",
  "key_id": "16 hex characters"
}
```

- `payload` is the document as **JSON text**, exactly the bytes that were signed. Check the
  signature over these bytes, then parse them. Never re-serialise before checking.
- The signed message is the **purpose prefix**, a newline, then the payload's UTF-8 bytes, e.g.
  `slop/release-manifest/v1\n{"schema":1,...}`. The prefix stops a document signed for one
  purpose being passed off as another.
- `signature` is a standard base64 (with padding) Ed25519 signature
  ([RFC 8032](https://www.rfc-editor.org/rfc/rfc8032)). Check it strictly: reject non-canonical
  signatures and weak keys (`verify_strict` in `ed25519-dalek`).
- `key_id` is the first 16 hex characters of the SHA-256 of the 32-byte public key. It only picks
  which of your trusted keys to try; it proves nothing by itself.

The envelope's JSON Schema is [`schemas/signed.schema.json`](schemas/signed.schema.json).

## Keys

`GET /v1/keys` lists the site's public keys as `[{"key_id": "...", "public_key": "base64 32
bytes"}]`. Fetching keys from the site you're checking only proves the site said so; **pin the
keys** you trust in your own code or configuration, as the SLAPorSLOP desktop app does at build time.
Keys may be added before an old one is retired, so accept any key on your pinned list.

## Checking a release before installing it

1. Check the manifest's signature with a pinned key, then parse it.
2. Fetch the revocation list, check its signature and that it hasn't expired, and refuse a
   release it names.
3. Download each file from its `url`. Before using it, its size and SHA-256 must match the
   manifest. For a file in parts, check each part's size and hash, join them in order, then check
   the whole file.
4. Check the maker's signature: the file's `attestation` (a Sigstore bundle naming the file's
   SHA-256, built from the manifest's `repo_url` at its `commit`), or its `maker_signature` (an
   OpenSSH signature). A release with neither isn't from its maker.
5. Install only then. Start only the `launch.exec` path inside the installed files.

`slop-verify manifest <file> --key <key> --file <download> --attestations` does steps 1, 3 and 4;
`slop-verify revocations` does step 2.

## Versions

Each document carries `"schema": 1`. Within a schema version SLAPorSLOP only **adds optional fields**
(left out when empty). A reader that ignores unknown fields keeps working; a reader that rejects
them (as SLAPorSLOP's own app does, to be strict) needs updating to read new fields. A change that
would break a reader gets a new schema number and a new purpose prefix.

## Using "verified by SLAPorSLOP"

A store or launcher may say a file is **verified by SLAPorSLOP** when a fresh, correctly signed file
verification lists it with status `approved`. Say **revoked** when it's listed as `revoked`, and
nothing at all when it isn't listed. Link to the match's `page_url`. Use SLAPorSLOP's name only for what
the document says; don't suggest SLAPorSLOP endorses your store.
