// SPDX-License-Identifier: AGPL-3.0-or-later
// Slop's own pages: terms and policies, the transparency report, checking a file, the source
// code offer and downloading the desktop app.
import { expect, ids, signIn, test } from "./fixtures";
import { call, failApi, path } from "./store-helpers";

test("terms and policies: the list, a document, and what's accepted versus only read", async ({ browser }) => {
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto("/legal");
  await expect(pat.getByRole("heading", { name: "Terms and policies", level: 1 })).toBeVisible();
  const terms = pat.locator(".legal-index li").filter({ has: pat.getByRole("link", { name: "Terms of Service" }) });
  await expect(terms.getByText(/^Version [\w-]+, in effect since .+ · you accept this one$/)).toBeVisible();
  const privacy = pat.locator(".legal-index li").filter({ has: pat.getByRole("link", { name: "Privacy Policy" }) });
  await expect(privacy.getByText(/· for reading$/)).toBeVisible();
  await expect(pat.getByRole("link", { name: "Transparency report" })).toHaveAttribute("href", "/transparency");

  await terms.getByRole("link", { name: "Terms of Service" }).click();
  await expect(pat).toHaveURL(/\/legal\/terms$/);
  await expect(pat.getByRole("heading", { name: "Terms of Service", level: 1 })).toBeVisible();
  await expect(pat.getByText(/^Version [\w-]+ · in effect since /)).toBeVisible();
  await expect(pat.getByText(/^SHA-256 of this text:/)).toBeVisible();
  // One version so far: no version list, and nothing waiting to be accepted.
  await expect(pat.getByRole("heading", { name: "All versions" })).toHaveCount(0);
  await expect(pat.getByRole("button", { name: /Review and accept/ })).toHaveCount(0);

  await pat.goto("/legal/no-such-document");
  await expect(pat.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});

test("a document's earlier versions can be read, and an announced change is shown and accepted", async ({ page }) => {
  const current = await call<{ version: string; summary: string; effective_at: string; published_at: string; body_md: string }>(null, "GET", "/v1/legal/terms");
  const later = new Date(Date.now() + 15 * 864e5).toISOString();
  const next = { version: "2099-01-01", summary: "Clearer words about refunds.", published_at: current.published_at, effective_at: later };
  const versions = [next, { version: current.version, summary: current.summary, published_at: current.published_at, effective_at: current.effective_at }];
  // As legal.rs answers once staff publish a change, 15 days ahead.
  await page.route(path("/v1/legal/terms"), async (route) => {
    const asked = new URL(route.request().url()).searchParams.get("version");
    const res = await route.fetch({ url: `${new URL(route.request().url()).origin}/v1/legal/terms` });
    const doc = await res.json();
    const which = asked === next.version ? { ...doc, ...next, in_effect: false, body_md: "# Terms of Service\n\nThe new words." } : doc;
    await route.fulfill({ json: { ...which, versions } });
  });
  let accepted: unknown;
  await page.route(path("/v1/me/legal"), (route) =>
    route.fulfill({
      json: accepted
        ? []
        : [{ kind: "terms", title: "Terms of Service", version: next.version, summary: next.summary, effective_at: later, in_effect: false, first: false, action: "accept" }],
    }),
  );
  await page.route(path("/v1/me/legal/accept"), (route) => {
    accepted = route.request().postDataJSON();
    return route.fulfill({ status: 204 });
  });
  await signIn(page, "player");
  await page.goto("/legal/terms");
  await expect(page.getByText(/^Version [\w-]+ · in effect since .+ · First version\.$/)).toBeVisible();
  const list = page.locator(".legal-versions");
  await expect(list.getByRole("button", { name: current.version })).toHaveAttribute("aria-current", "page");
  await list.getByRole("button", { name: next.version }).click();
  await expect(page).toHaveURL(/\/legal\/terms\?version=2099-01-01$/);
  await expect(page.getByText(/^Version 2099-01-01 · takes effect on /)).toBeVisible();
  await expect(page.locator("article").getByText("The new words.")).toBeVisible();
  await expect(list.getByRole("button", { name: next.version })).toHaveAttribute("aria-current", "page");

  // The change to accept.
  const notice = page.locator(".notice").filter({ hasText: "The Terms of Service changes on" });
  await expect(notice).toContainText("Clearer words about refunds.");
  await notice.getByRole("button", { name: "Review and accept version 2099-01-01" }).click();
  const dialog = page.getByRole("dialog", { name: "Terms of Service" });
  await expect(dialog.getByRole("heading", { name: "Before you continue: the Terms of Service" })).toBeVisible();
  await dialog.getByRole("button", { name: "Not now" }).click();
  await expect(dialog).toBeHidden();
  await notice.getByRole("button", { name: "Review and accept version 2099-01-01" }).click();
  await expect(dialog.getByRole("button", { name: "Accept and continue" })).toBeDisabled();
  await dialog.getByRole("checkbox", { name: "I've read and accept the Terms of Service." }).check();
  await dialog.getByRole("button", { name: "Accept and continue" }).click();
  await expect(dialog).toBeHidden();
  expect(accepted).toEqual({ kind: "terms", version: "2099-01-01" });
  await expect(notice).toHaveCount(0);

  // The list says what's coming.
  await page.route(path("/v1/legal"), async (route) => {
    const res = await route.fetch();
    const list = (await res.json()) as Array<{ kind: string }>;
    await route.fulfill({ json: list.map((d) => (d.kind === "terms" ? { ...d, upcoming: next } : d)) });
  });
  await page.goto("/legal");
  await expect(page.locator(".legal-index li").filter({ hasText: "Terms of Service" }).getByText(/^Changes on .+: Clearer words about refunds\.$/)).toBeVisible();
});

test("the terms list offers Try again when it can't load", async ({ page }) => {
  let fail = true;
  await page.route(path("/v1/legal"), (route) => (fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy. Try again soon." } } }) : route.fallback()));
  await page.goto("/legal");
  await expect(page.getByRole("alert")).toContainText("Slop is busy. Try again soon.");
  fail = false;
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("link", { name: "Terms of Service" })).toBeVisible();
});

test("the transparency report: a year's counts, and switching years", async ({ page }) => {
  // Early 2027: two years to choose from, the newest first. (The server still lives in 2026, so
  // 2027's numbers are this year's, as it would answer them then.)
  await page.clock.setFixedTime(new Date("2027-02-01T12:00:00Z"));
  await page.route(path("/v1/transparency/2027"), async (route) => {
    const res = await route.fetch({ url: route.request().url().replace("/2027", "/2026") });
    await route.fulfill({ json: { ...(await res.json()), year: 2027, complete: false } });
  });
  const asked: string[] = [];
  page.on("request", (r) => {
    const m = new URL(r.url()).pathname.match(/^\/v1\/transparency\/(\d+)$/);
    if (m) asked.push(m[1]);
  });
  await page.goto("/transparency");
  await expect(page.getByRole("heading", { name: "Transparency report", level: 1 })).toBeVisible();
  const year = page.getByRole("combobox", { name: "Year" });
  await expect(year.locator("option")).toHaveText(["2027", "2026"]);
  await expect(year).toHaveValue("2027");
  await expect(page.getByText("2027 isn't over yet: these are the numbers so far.")).toBeVisible();

  await year.selectOption("2026");
  await expect.poll(() => asked).toEqual(["2027", "2026"]);
  // The seed's decisions: Delisted Dash taken out of the store, and Sid suspended.
  const decisions = page.getByRole("table", { name: "Decisions by action" });
  await expect(decisions.getByRole("row", { name: /App taken out of the store \d+/ })).toBeVisible();
  await expect(decisions.getByRole("row", { name: /Account suspended \d+/ })).toBeVisible();
  await expect(page.getByRole("table", { name: "Appeals by outcome" }).getByRole("row", { name: /Waiting \d+/ })).toBeVisible();
  await expect(page.getByRole("table", { name: "By country" }).getByRole("cell", { name: "None" })).toBeVisible();
  await expect(page.getByText(/^Decisions made by automated tools alone: 0\./)).toBeVisible();

  // A finished year has no "so far" note.
  await page.route(path("/v1/transparency/2026"), async (route) => {
    const res = await route.fetch();
    await route.fulfill({ json: { ...(await res.json()), complete: true } });
  });
  await year.selectOption("2027");
  await year.selectOption("2026");
  await expect(decisions).toBeVisible();
  await expect(page.getByText(/isn't over yet/)).toHaveCount(0);
});

test("checking a file: approved, withdrawn, a bad hash, and a server error", async ({ page }) => {
  const releases = await call<Array<{ files: Array<{ sha256: string }> }>>(null, "GET", `/v1/apps/${ids.listedApp}/releases`);
  const sha = releases[0].files[0].sha256;
  await page.goto("/verify");
  const hash = page.getByRole("textbox", { name: "Or paste its SHA-256" });
  const check = page.getByRole("button", { name: "Check" });
  // Only a full SHA-256 can be checked.
  await hash.fill("abc123");
  await expect(check).toBeDisabled();
  await hash.fill("z".repeat(64));
  await expect(check).toBeDisabled();
  await hash.fill(sha.toUpperCase());
  await check.click();
  const result = page.getByRole("status");
  const lantern = result.locator("li").filter({ has: page.getByRole("link", { name: "Lantern Keep", exact: true }) });
  await expect(lantern).toContainText("Approved on Slop: Lantern Keep 1.0.0 by Mia Maker, for windows.");
  await expect(lantern.getByRole("link", { name: "Lantern Keep", exact: true })).toHaveAttribute("href", `/app/${ids.listedApp}`);

  // A build Slop withdrew, as the server signs it.
  const withdrawn = {
    schema: 1,
    sha256: "f".repeat(64),
    issued_at: new Date().toISOString(),
    expires_at: new Date().toISOString(),
    matches: [
      {
        app_slug: "castle-rock",
        app_title: "Castle Rock",
        maker: "Sam Seed",
        release_id: "00000000-0000-0000-0000-000000000002",
        version: "0.9.0",
        platform: "windows",
        status: "revoked",
        approved_at: "2026-09-01T00:00:00Z",
        revoked_at: "2026-09-02T00:00:00Z",
        scan: "passed",
        page_url: "/app/castle-rock",
      },
    ],
  };
  await page.route(path(`/v1/verify/${"f".repeat(64)}`), (route) =>
    route.fulfill({ json: { payload: JSON.stringify(withdrawn), signature: "", key_id: "" } }),
  );
  await hash.fill("f".repeat(64));
  await check.click();
  await expect(result).toContainText("Withdrawn by Slop: Castle Rock 0.9.0 by Sam Seed, for windows.");
  await expect(result).toContainText("don't run it.");

  await failApi(page, path(`/v1/verify/${"e".repeat(64)}`));
  await hash.fill("e".repeat(64));
  await check.click();
  await expect(result).toContainText("Something broke on Slop's side.");
});

test("a file over 1 GB isn't read in the browser: it asks for the hash instead", async ({ page }) => {
  let asked = false;
  page.on("request", (r) => {
    if (new URL(r.url()).pathname.startsWith("/v1/verify/")) asked = true;
  });
  // A small file that says it's 2 GB (the page only looks at the size first).
  await page.addInitScript(() => Object.defineProperty(File.prototype, "size", { get: () => 2 * 1024 ** 3 }));
  await page.goto("/verify");
  await page.getByLabel("Choose the file").setInputFiles({ name: "huge.iso", mimeType: "application/octet-stream", buffer: Buffer.from("tiny") });
  await expect(page.getByRole("status")).toContainText("That file is over 1 GB, too big to check in the browser. Paste its SHA-256 instead");
  await expect(page.getByRole("textbox", { name: "Or paste its SHA-256" })).toHaveValue("");
  expect(asked).toBe(false);
});

test("the source code page offers Try again when it can't load", async ({ page }) => {
  let fail = true;
  await page.route(path("/v1/source"), (route) => (fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy. Try again soon." } } }) : route.fallback()));
  await page.goto("/source");
  await expect(page.getByRole("alert")).toContainText("Slop is busy. Try again soon.");
  fail = false;
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByText(/GNU Affero General Public License/)).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("the download page: a card per system, yours first in colour, and missing builds tolerated", async ({ page }) => {
  // This server has no desktop builds yet.
  await page.goto("/download");
  await expect(page.getByRole("heading", { name: "Get the Slop app", level: 1 })).toBeVisible();
  const cards = page.locator(".download-card");
  await expect(cards).toHaveCount(3);
  await expect(cards.getByText("Not available right now.")).toHaveCount(3);
  await expect(page.getByText("You're already using the Slop app.")).toHaveCount(0);

  // With builds for Windows and RHEL, not Debian. The browser says Windows: that card is marked.
  await page.route(
    (u) => u.pathname.startsWith("/v1/desktop/latest/"),
    (route) => {
      const platform = new URL(route.request().url()).pathname.split("/").pop();
      if (platform === "linux-deb") return route.fulfill({ status: 404, json: { error: { code: "not_found", message: "not found" } } });
      return route.fulfill({ json: { version: "1.4.2", url: `https://downloads.example.com/slop-1.4.2-${platform}` } });
    },
  );
  await page.reload();
  const windows = cards.filter({ has: page.getByRole("heading", { name: "Windows" }) });
  await expect(windows.getByText("YOUR SYSTEM")).toBeVisible();
  await expect(windows).toHaveClass(/yours/);
  await expect(windows.getByRole("link", { name: "Download" })).toHaveAttribute("href", "https://downloads.example.com/slop-1.4.2-windows");
  await expect(windows.getByText("Version 1.4.2")).toBeVisible();
  const debian = cards.filter({ has: page.getByRole("heading", { name: "Ubuntu / Debian" }) });
  await expect(debian.getByText("Not available right now.")).toBeVisible();
  await expect(debian.getByText("YOUR SYSTEM")).toHaveCount(0);
  const rhel = cards.filter({ has: page.getByRole("heading", { name: "RHEL / Rocky / Alma 10" }) });
  await expect(rhel.getByRole("link", { name: "Download" })).toHaveAttribute("href", "https://downloads.example.com/slop-1.4.2-linux-rpm");
  await expect(rhel.getByText("sudo dnf install epel-release")).toBeVisible();
});
