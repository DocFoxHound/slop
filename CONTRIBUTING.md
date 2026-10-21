# Contributing to Slop

Thanks for helping. Before you open a pull request:

1. **Read [`AGENTS.md`](AGENTS.md).** It holds the project's rules (plain wording, 404 not 403,
   bound SQL, accessibility, and how personal data is recorded). They apply to people too.
2. **Run the checks** the deploy runs: `cargo fmt --check`, `cargo clippy --all-targets -- -D
   warnings`, `cargo test --workspace` (with a test database, see the README), and in `web/`:
   `npm run typecheck`, `npm test`, `npm run build` and `npm run e2e`.
3. **Sign off every commit** (below).

## Licence of your contribution

Your change is licensed the way the part it changes is licensed (the README's
[License](README.md#license) table): the server, website and desktop app under the AGPL-3.0 or
later; everything else under Apache-2.0 or MIT. You keep the copyright in your work.

## Sign-off (Developer Certificate of Origin)

Each commit needs a line confirming you wrote it or otherwise have the right to submit it under
the project's licences, as set out in the [Developer Certificate of Origin 1.1](https://developercertificate.org/):

```
Signed-off-by: Your Name <you@example.com>
```

`git commit -s` adds it from your Git name and email. Use your real name or the name you're known
by; no anonymous contributions.

## Security problems

Don't open a public issue. Use the contact in `/.well-known/security.txt` on the live site.
