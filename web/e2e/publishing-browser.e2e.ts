// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing a game that plays in the browser (DESIGN A9): the new app form's Runs on choice and
// play address, the first address sent with the page and checked on Releases, an address that
// doesn't go through waiting there to be fixed, and Sam's seeded Orbit Tap's Releases tab and
// Manage page. Mia's games are her own, made here; what Sam sends to Orbit Tap is withdrawn after,
// so the address Play opens stays the seeded one.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { expect, ids, server, signIn, test } from "./fixtures";
import { call, fakeRepo, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

/** No serious or critical problem axe can find (WCAG 2.2 AA). */
async function axe(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

interface ReleaseRow {
  id: string;
  version: string;
  status: string;
  play_address: string | null;
}

/** Withdraws `who`'s releases of `slug` still waiting, so the review queue is as it was. */
async function withdrawWaiting(who: "maker" | "sam", slug: string) {
  const releases = await call<ReleaseRow[]>(who, "GET", `/v1/apps/${slug}/releases`);
  for (const r of releases.filter((x) => x.status === "fetching" || x.status === "awaiting_review")) {
    await call(who, "POST", `/v1/apps/${slug}/releases/${r.id}/withdraw`);
  }
}

/** The page on the fake GitHub a play address can lead to (e2e_server/main.rs serves it). */
const playPage = (repo: string) => `${server.fake}e2e/${repo}/play/`;

const lamp = uniq("pub-web");
const lampTitle = `Lantern Run ${lamp.slice(-4)}`;

test("a new game that plays in the browser: Runs on is one choice, and its play address goes to Releases", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill(lampTitle);
  await page.getByRole("textbox", { name: /^Store address/ }).fill(lamp);
  await page.getByLabel("One-line summary").fill("Run the lanterns down the hill.");
  // On GitHub, so SLAPorSLOP can take the newest commit for the address (the fake GitHub's).
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`https://github.com/e2e/${lamp}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");

  const runsOn = page.getByRole("group", { name: "Runs on" });
  const download = runsOn.getByRole("radio", { name: /^Download/ });
  const inBrowser = runsOn.getByRole("radio", { name: /^In the browser/ });
  // A download to start with: its builds are ticked below it.
  await expect(download).toBeChecked();
  await expect(runsOn.getByRole("checkbox", { name: /Windows/ })).toBeVisible();
  await runsOn.getByRole("checkbox", { name: /Windows/ }).check();

  // Only games play in the browser.
  await page.getByLabel("Type").selectOption("app");
  await expect(inBrowser).toBeDisabled();
  await expect(runsOn.getByText("Only games can play in the browser for now.")).toBeVisible();
  await page.getByLabel("Type").selectOption("game");

  // In the browser stands alone: no builds, no sandbox mode or mods folder, and never private.
  await inBrowser.check();
  await expect(download).not.toBeChecked();
  await expect(runsOn.getByRole("checkbox")).toHaveCount(0);
  await expect(runsOn.getByText(/SLAPorSLOP checks the address, not what the site serves/)).toBeVisible();
  await expect(page.getByRole("checkbox", { name: /Works in sandbox mode/ })).toHaveCount(0);
  await expect(page.getByRole("group", { name: "Mods" })).toHaveCount(0);
  await expect(page.getByRole("radio", { name: /^Private/ })).toBeDisabled();
  await expect(page.getByLabel("Stage").locator("option", { hasText: "Coming soon (not playable yet)" })).toHaveCount(1);
  // Back to a download, the builds come back (none ticked: browser took their place).
  await download.check();
  await expect(runsOn.getByRole("checkbox", { name: /Windows/ })).not.toBeChecked();
  await expect(page.getByRole("checkbox", { name: /Works in sandbox mode/ })).toBeVisible();
  await inBrowser.check();

  // A Coming soon page may leave the address for later; Beta needs it.
  const address = page.getByRole("textbox", { name: /^Play address/ });
  await expect(page.getByText("(optional for Coming soon)")).toBeVisible();
  await page.getByLabel("Stage").selectOption("beta");
  await expect(address).toHaveAttribute("required", "");
  await address.fill(playPage(lamp));
  await axe(page);

  await page.getByRole("button", { name: "Create draft" }).click();
  // Made, and the address sent as its first release: Releases shows it being checked.
  await expect(page).toHaveURL(new RegExp(`/publish/${lamp}/releases$`));
  await expect(page.getByText("Your game runs on your own website. Here a release is its play address")).toBeVisible();
  await expect(page.locator(".maker-head").getByText("PLAYS IN THE BROWSER")).toBeVisible();
  const release = page.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
  await expect(release.getByText("IN REVIEW", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(release.getByText("It passed the checks. Waiting for a person at SLAPorSLOP, who opens it and plays it.")).toBeVisible();
  await expect(release.getByRole("link", { name: playPage(lamp) })).toHaveAttribute("rel", "noopener noreferrer");
  // Each check in words: this test server allows plain http and doesn't look up where a site is,
  // so those aren't claimed; its bad-link list is the seed's one site.
  const checks = release.getByRole("list", { name: "SLAPorSLOP's checks of the address" });
  await expect(checks.getByRole("listitem")).toHaveText([
    /https: No: plain http/,
    /A public site: Not checked/,
    /Bad-link lists: Not on them/,
    /The page loads: Yes \(HTTP 200\)/,
  ]);
  // The commit was the repository's newest (the fake GitHub's).
  const [saved] = await call<Array<ReleaseRow & { commit_sha: string; files: unknown[] }>>("maker", "GET", `/v1/apps/${lamp}/releases`);
  expect(saved).toMatchObject({ version: "1.0.0", play_address: playPage(lamp), files: [] });
  expect(saved.commit_sha).toMatch(/^[0-9a-f]{40}$/);

  // Nothing about downloads on its Releases tab.
  for (const gone of ["New release", "Submit a release", "Let Slop Build do it"]) {
    await expect(page.getByRole("heading", { name: gone })).toHaveCount(0);
  }
  await expect(page.getByText("Or build and sign it yourself")).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: /Pick up new GitHub releases/ })).toHaveCount(0);
  await expect(page.locator(".updates-panel")).toContainText("A new address always waits for a person.");

  // Its progress: the address and its checks, what SLAPorSLOP can't check, then a person.
  const progress = page.getByRole("region", { name: "Publishing progress" });
  await expect(progress.locator("ol.steps > li > div > b")).toHaveText([
    "Store page",
    "Play address",
    "Address checked",
    "What SLAPorSLOP can't check",
    "Source check",
    "Review by a person",
    "In the store",
  ]);
  await expect(progress).toContainText("SLAPorSLOP checked the address, not what the site serves");
  await expect(progress).not.toContainText("Maker signature");
  // The stage bar waits for the address.
  await expect(page.getByRole("region", { name: "Stage" })).toContainText("Release it once a play address is approved.");

  // The form starts from the address sent, with the next version suggested.
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "Your play address" }) });
  await expect(form.getByRole("textbox", { name: /^Play address/ })).toHaveValue(playPage(lamp));
  await expect(form.getByRole("textbox", { name: /^Version/ })).toHaveValue("1.0.1");
  await expect(form.getByRole("textbox", { name: /^Commit/ })).not.toHaveAttribute("required", "");
  await axe(page);

  // The dashboard says where it is (this test server may still be reading the fake repository's
  // source, which comes before the review).
  await page.goto("/publish");
  await expect(page.locator("li.status-row").filter({ hasText: lampTitle })).toContainText(
    /v1\.0\.0Waiting on: (Source check|Review by a person)/,
  );
});

test("its store page and Manage: where Play goes, and the choice kept once it has releases", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(`/publish/${lamp}`);
  const runsOn = page.getByRole("group", { name: "Runs on" });
  await expect(runsOn.getByRole("radio", { name: /^In the browser/ })).toBeChecked();
  await expect(runsOn.getByRole("radio", { name: /^Download/ })).toBeDisabled();
  await expect(runsOn.getByText(/It has releases, so it can't switch between a download and a browser game/)).toBeVisible();
  // The address isn't edited here.
  await expect(page.getByRole("textbox", { name: /^Play address/ })).toHaveCount(0);
  await expect(page.locator(".play-address-line")).toHaveText("Play address: none approved yet · change it on Releases");
  await expect(page.locator(".play-address-line").getByRole("link", { name: "change it on Releases" })).toHaveAttribute(
    "href",
    `/publish/${lamp}/releases`,
  );
  // Saving sends no platforms (they didn't change), and nothing a browser game doesn't have.
  await page.getByLabel("One-line summary").fill("Run every lantern down the hill.");
  const saving = page.waitForRequest((r) => r.method() === "PATCH" && r.url().endsWith(`/v1/apps/${lamp}`));
  await page.getByRole("button", { name: "Save changes" }).click();
  const body = (await saving).postDataJSON() as Record<string, unknown>;
  expect(body.summary).toBe("Run every lantern down the hill.");
  for (const key of ["platforms", "sandbox_ready", "mods_folder"]) expect(body).not.toHaveProperty(key);
  await expect(page).toHaveURL(new RegExp(`/app/${lamp}$`));

  await page.goto(`/publish/${lamp}/manage`);
  const bar = page.getByRole("region", { name: "Stage" });
  await expect(bar).toContainText("Release it once a play address is approved.");
  await expect(bar.getByRole("link", { name: "Add the play address" })).toHaveAttribute("href", `/publish/${lamp}/releases`);
  const where = page.getByRole("region", { name: "Where Play goes" });
  await expect(where).toContainText("Nothing yet: no play address has been approved. Add it on Releases.");
  await expect(page.getByRole("heading", { name: "What players get" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: /Browser demo/ })).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: /Pick up new GitHub releases/ })).toHaveCount(0);
  await expect(page.getByText(/v1\.0\.0\s+Waiting for review, submitted from your account/)).toBeVisible();
  await axe(page);

  await withdrawWaiting("maker", lamp);
  await readNotices("maker", lamp);
});

test("an address that doesn't go through with the new page waits on Releases, filled in, to fix", async ({ page }) => {
  const slug = uniq("pub-web2");
  await signIn(page, "maker");
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill(`Kite Hill ${slug.slice(-4)}`);
  await page.getByRole("textbox", { name: /^Store address/ }).fill(slug);
  await page.getByLabel("One-line summary").fill("Fly the kite over the hill.");
  // Not on GitHub: SLAPorSLOP can't look up the newest commit, so the first address is refused.
  await page.getByRole("textbox", { name: /Source repository/ }).fill(fakeRepo("mia", slug));
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("group", { name: "Runs on" }).getByRole("radio", { name: /^In the browser/ }).check();
  await page.getByLabel("Stage").selectOption("beta");
  await page.getByRole("textbox", { name: /^Play address/ }).fill(playPage(slug));
  await page.getByRole("button", { name: "Create draft" }).click();

  await expect(page).toHaveURL(new RegExp(`/publish/${slug}/releases$`));
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "Your play address" }) });
  await expect(form.getByText("Your store page is made, but its play address didn't go through.")).toBeVisible();
  await expect(form.getByRole("textbox", { name: /^Play address/ })).toHaveValue(playPage(slug));
  const commit = form.locator("label").filter({ hasText: "Commit" });
  await expect(commit.getByRole("alert")).toContainText("give the commit your site is built from");
  await expect(form.getByRole("textbox", { name: /^Commit/ })).toHaveAttribute("required", "");
  await expect(page.getByText("No releases yet.")).toBeVisible();

  // An address on a bad-link list is refused, by its field.
  await form.getByRole("textbox", { name: /^Commit/ }).fill("c".repeat(40));
  await form.getByRole("textbox", { name: /^Play address/ }).fill("https://bad-games.example/play");
  await form.getByRole("button", { name: "Send the address" }).click();
  await expect(form.locator("label").filter({ hasText: "Play address" }).getByRole("alert")).toHaveText(
    "this address is on the e2e test list, a list of known-bad links",
  );
  await form.getByRole("textbox", { name: /^Play address/ }).fill(playPage(slug));
  await form.getByRole("button", { name: "Send the address" }).click();
  await expect(form.getByRole("status")).toContainText("Version 1.0.0 sent. SLAPorSLOP is checking the address");
  await expect(form.getByText("didn't go through")).toHaveCount(0);
  const release = page.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
  await expect(release.getByText("IN REVIEW", { exact: true })).toBeVisible({ timeout: 20_000 });
  // A reload doesn't bring the old problem back.
  await page.reload();
  await expect(page.getByText("didn't go through")).toHaveCount(0);

  await withdrawWaiting("maker", slug);
  await readNotices("maker", slug);
  // Withdrawn, it has no address again: the dashboard asks for one.
  await page.goto("/publish");
  await expect(page.locator("li.status-row").filter({ hasText: slug.slice(-4) })).toContainText("Next: add the play address");
});

test("Orbit Tap's Releases tab: the address Play opens, and a new one waits for a person", async ({ page }) => {
  await signIn(page, "sam");
  try {
    await page.goto(`/publish/${ids.browserApp}/manage`);
    const where = page.getByRole("region", { name: "Where Play goes" });
    await expect(where.getByRole("link", { name: ids.browserPlayAddress })).toHaveAttribute("href", ids.browserPlayAddress);
    await expect(where).toContainText("(v1.0.0)");
    await expect(where).toContainText("Always the newest approved address.");
    // No release to hold players at.
    await expect(where.getByRole("radio")).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Stage" })).toContainText("Released: the last stage.");

    await page.goto(`/publish/${ids.browserApp}/releases`);
    const approved = page.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
    await expect(approved.getByText("APPROVED", { exact: true })).toBeVisible();
    await expect(approved).toContainText("Approved: Play opens this address.");
    await expect(approved.getByRole("link", { name: ids.browserPlayAddress })).toBeVisible();
    await expect(approved).not.toContainText(/SHA-256|NOT SCANNED|SIGNED/);
    const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "A new play address or version" }) });
    await expect(form.getByRole("textbox", { name: /^Play address/ })).toHaveValue(ids.browserPlayAddress);
    const version = form.getByRole("textbox", { name: /^Version/ });
    await expect(version).toHaveValue(/^1\.0\.[1-9]\d*$/);
    const sent = await version.inputValue();
    await expect(page.getByRole("heading", { name: "Let Slop Build do it" })).toHaveCount(0);
    await expect(page.getByRole("checkbox", { name: /Pick up new GitHub releases/ })).toHaveCount(0);

    // A new site: checked, then it waits for a person; Play keeps opening the approved one.
    const next = playPage("orbit-tap-next");
    await form.getByRole("textbox", { name: /^Play address/ }).fill(next);
    await form.getByLabel("What's new, for players (optional)").fill("Moons now wobble.");
    await form.getByRole("button", { name: "Send the address" }).click();
    await expect(form.getByRole("status")).toContainText(`Version ${sent} sent.`);
    const waiting = page.locator("ul.release-list > li").filter({ hasText: sent });
    await expect(waiting.getByText("IN REVIEW", { exact: true })).toBeVisible({ timeout: 20_000 });
    await expect(waiting.getByRole("link", { name: next })).toBeVisible();
    await expect(waiting.getByRole("list", { name: "SLAPorSLOP's checks of the address" })).toContainText("The page loads: Yes (HTTP 200)");
    await expect(waiting.locator(".release-notes")).toHaveText("Moons now wobble.");
    const [newest] = await call<ReleaseRow[]>("sam", "GET", `/v1/apps/${ids.browserApp}/releases`);
    // Waiting for a person, however Sam's updates are approved: a new address always is.
    expect(newest).toMatchObject({ version: sent, status: "awaiting_review", play_address: next });
    const store = await call<{ play_address: string }>(null, "GET", `/v1/apps/${ids.browserApp}`);
    expect(store.play_address).toBe(ids.browserPlayAddress);
    await axe(page);
  } finally {
    await withdrawWaiting("sam", ids.browserApp);
    await readNotices("sam", ids.browserApp);
  }
  const after = await call<{ play_address: string }>(null, "GET", `/v1/apps/${ids.browserApp}`);
  expect(after.play_address).toBe(ids.browserPlayAddress);
});
