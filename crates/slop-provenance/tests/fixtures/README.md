Test fixtures copied from the `sigstore-verify` 0.15.0 crate's `test_data` (Apache-2.0,
https://github.com/sigstore/sigstore-rust), which took them from sigstore-go:

- `github-actions-provenance.sigstore.json`: the SLSA provenance attestation GitHub Actions made
  for `sigstore@2.0.0` on npm, logged in Sigstore's public-good Rekor. Built from
  `https://github.com/sigstore/sigstore-js` at commit `f0b49a04e5a62250e0f60fb128004a73110fe311`.
- `dsse.sigstore.json`: a DSSE bundle that isn't from GitHub Actions.
- `github-private-attestation.sigstore.json`: a release attestation from a private GitHub
  repository, signed by GitHub's own Sigstore instance (timestamped, no public log entry). Used to
  check verification against GitHub's root.
