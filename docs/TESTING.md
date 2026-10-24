# Testing Slop privately

Everything stays private: the `slop` repository, the Slop Build recipe and your test apps.

## 1. Sign-in (needed before anyone can sign in)

Create a GitHub OAuth app (GitHub → Settings → Developer settings → OAuth Apps → New):

- **Homepage URL:** `https://slop.188-68-41-241.sslip.io`
- **Authorization callback URL:** `https://slop.188-68-41-241.sslip.io/v1/auth/github/callback`

Put its client ID and a client secret in `~/.config/slop/slop.env` on the server
(`GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`), then `systemctl --user restart slop-api`.

## 2. The desktop app on Windows

Open the site and click **Download** at the top left, then **Windows** (it appears once the first
build has been published). The test server's app is **Slop Dev**, built on the server after each
deploy that changed it; it installs beside the real Slop and keeps its own sign-in and settings
(`deploy/README.md`, "The desktop app"). Windows warns that the publisher is unknown, because the
installer isn't code-signed: choose **More info → Run anyway**. From then on the app offers each
new version itself.

## 3. Private repositories

- **Release files:** give the server a fine-grained GitHub token with read-only **Contents** and
  **Metadata** on your test repositories (`GITHUB_TOKEN` in `slop.env`). Slop uses it to check
  commits, find attestations, and hand players' apps short-lived download links.
- **Maker signatures:** GitHub only makes artifact attestations in private repositories on
  GitHub Enterprise Cloud. The deployment therefore runs with `SLOP_REQUIRE_MAKER_SIGNATURE=false`,
  desktop builds are made with `SLOP_ALLOW_UNSIGNED=true`, and Slop Build is used with
  `sign: false`. Builds show as **NOT SIGNED**; everything else is checked as usual.

## 4. Slop Build in a private repository

1. Create a private repository `DocFoxHound/slop-build` and copy this repo's `slop-build/` folder
   into its root, so the recipe is at `.github/workflows/build.yml`. Tag it `v1`.
2. In that repository: **Settings → Actions → General → Access →** "Accessible from repositories
   owned by the user 'DocFoxHound'".
3. On Slop: **Publish → New app** (repository URL of your private test app), then **Releases →
   Add to my repository on GitHub**, and add `sign: false` under `with:` before committing.
4. Push to the test app's main branch. The run builds, attaches the zips to a release, and sends
   it to Slop. Review it on the server:

   ```sh
   set -a; . ~/.config/slop/slop.env; set +a
   ~/slop-deploy/current/slop-api review pending
   ~/slop-deploy/current/slop-api review approve <id> --reviewer you
   ~/slop-deploy/current/slop-api review list <slug> --reviewer you
   ```

5. Install it from the desktop app.
