// SPDX-License-Identifier: AGPL-3.0-or-later
// Games that play in the browser, for players (components/InstallControls.tsx BrowserControls,
// pages/AppPage.tsx, pages/Library.tsx): the store page says where it runs and what SLAPorSLOP
// checked, and Play opens the maker's website in a new tab (the desktop app: the system browser),
// never anything to install; SLAPorSLOP records the play, so the player can rate it and report a
// bug. The library has them in a group of their own.
//
// Sam's seeded Orbit Tap (ids.browserApp) stays unplayed and in nobody's library: where Pat
// presses Play on it, the launch is answered here (page.route), and he takes it out of his library
// again. A real recorded play is on Remy's Comet Hop, made for this file under a new address each
// run (a retry too), so Pat has never played it when the file starts, and delisted after.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { asDesktop, desktopCalls, openedUrls } from "./desktop-helpers";
import { expect, ids, signIn, test } from "./fixtures";
import { uniq } from "./publishing-helpers";
import { call, delist, listedApp, patchJson, path, playAddress, tryCall } from "./store-helpers";

const game = ids.browserApp;
const own = uniq("comet-hop");
const ownTitle = "Comet Hop";

test.beforeAll(async () => {
  await listedApp(own, ownTitle, { who: "pendingCreative", platforms: ["browser"], tags: ["arcade"] });
});

test.afterAll(async () => {
  await tryCall("player", "POST", `/v1/apps/${own}/vote`, { vote: null });
  for (const slug of [own, game]) await tryCall("player", "DELETE", `/v1/me/library/${slug}`);
  await delist(own);
});

async function axe(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

const head = (page: Page) => page.locator(".rating-head");
const box = (page: Page) => page.locator(".install-box");
const side = (page: Page) => page.getByRole("navigation", { name: "Your apps" });
const row = (page: Page, name: string) => side(page).getByRole("link", { name: new RegExp(`^${name}( – |$)`) });
const app = (page: Page, name: string) => page.getByRole("region", { name, exact: true });
const isLaunch = (slug: string) => (u: URL) => u.pathname === `/v1/apps/${slug}/launch`;

/** Answers Play's launch request here, so the seeded game stays unplayed; counts the requests. */
async function answerLaunches(page: Page, slug: string, address: string | null = ids.browserPlayAddress) {
  const sent: URL[] = [];
  await page.route(isLaunch(slug), (route) => {
    sent.push(new URL(route.request().url()));
    return route.fulfill({ json: { token: null, pulse_url: "/v1/pulse", heartbeat_seconds: 60, play_address: address } });
  });
  return sent;
}

/** Clicks (or presses Enter on) Play and returns the tab it opened, loaded. */
async function newTab(page: Page, open: () => Promise<void>) {
  const [tab] = await Promise.all([page.context().waitForEvent("page"), open()]);
  await tab.waitForLoadState();
  return tab;
}

test("a browser game's store page: plays in your browser, what SLAPorSLOP checked, and nothing to install", async ({ page }) => {
  const sent: string[] = [];
  page.on("request", (r) => {
    if (isLaunch(game)(new URL(r.url()))) sent.push(r.url());
  });
  await page.goto(`/app/${game}`);
  await expect(page.getByRole("heading", { name: "Orbit Tap", level: 1 })).toBeVisible();
  // By the title, in words with the icon beside them.
  const badge = page.locator(".title-row .badge").filter({ hasText: "PLAYS IN YOUR BROWSER" });
  await expect(badge).toBeVisible();
  await expect(badge.locator('svg[data-icon="browser"]')).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator("dl.kv dt", { hasText: "Runs on" }).locator("xpath=following-sibling::dd[1]")).toHaveText("Your browser");

  // What SLAPorSLOP checked: the address, not the site; no build, scan or sale claims.
  const trust = page.locator(".trust");
  await expect(trust.locator(".trust-row").first()).toHaveText("Runs on the maker's website. SLAPorSLOP checked the address, not what the site serves.");
  await expect(trust.getByText(/Built (by Slop Build )?from source|Scanned|Defender|signed by|Nothing for sale/)).toHaveCount(0);
  await expect(trust.getByText("Open source")).toBeVisible();
  // Nothing about installing, sandbox mode, servers SLAPorSLOP can't check, demos or mods.
  await expect(page.locator(".online-notice, .sandbox-note, .community-builds, .packages")).toHaveCount(0);
  await expect(page.getByText(/sandbox mode|SLAPorSLOP's signature|Get the SLAPorSLOP app|Play the demo/)).toHaveCount(0);

  // Play: a real link, opening a new tab with nothing of this page passed on.
  await expect(box(page).getByRole("heading", { name: "Play it", level: 3 })).toBeVisible();
  const play = box(page).getByRole("link", { name: "Play" });
  await expect(play).toHaveAttribute("href", ids.browserPlayAddress);
  await expect(play).toHaveAttribute("target", "_blank");
  await expect(play).toHaveAttribute("rel", "noopener noreferrer");
  await expect(play).toHaveAccessibleDescription(/^Opens 127\.0\.0\.1:\d+ in a new tab\. Nothing to download or install\.$/);
  await expect(box(page).getByText("Before you play:")).toBeVisible();
  await expect(box(page).getByRole("link", { name: "Sign in to add to your library" })).toBeVisible();
  // Signed out, ratings say how to get there.
  await expect(head(page).getByText(/Ratings come from players who played it through SLAPorSLOP\./)).toBeVisible();
  await expect(page.getByRole("region", { name: "Ratings" }).getByText(/and press Play here to vote\./)).toBeVisible();
  await axe(page);

  // Reporting it: the reasons are about its site, not a build.
  await page.locator(".report-row").getByRole("button", { name: "Report" }).click();
  const report = page.getByRole("dialog", { name: "Report this app" });
  await expect(report.getByLabel(/Its site doesn't load, or the game doesn't work/)).toBeVisible();
  await expect(report.getByLabel(/The build is broken/)).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(report).toBeHidden();

  // Signed out, Play opens the game (it's on the open web) and nothing is recorded.
  const tab = await newTab(page, () => play.click());
  expect(tab.url()).toBe(ids.browserPlayAddress);
  await expect(tab.getByRole("heading", { name: "orbit-tap", level: 1 })).toBeVisible();
  expect(await tab.evaluate(() => window.opener)).toBeNull();
  expect(await tab.evaluate(() => document.referrer)).toBe("");
  await tab.close();
  // From the keyboard too.
  await play.focus();
  const again = await newTab(page, () => page.keyboard.press("Enter"));
  expect(again.url()).toBe(ids.browserPlayAddress);
  await again.close();
  expect(sent).toEqual([]);
});

test("Play on the store page opens the game in a new tab and records the play; then the player can rate it and report a bug", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(`/app/${own}`);
  await expect(page.getByRole("heading", { name: ownTitle, level: 1 })).toBeVisible();

  // Added to the library from here, and taken out again.
  const add = box(page).getByRole("button", { name: "+ Add to library" });
  await add.click();
  const added = box(page).getByRole("button", { name: "✓ In your library" });
  await expect(added).toHaveAttribute("aria-pressed", "true");
  expect((await call<Array<{ slug: string; plays_in_browser: boolean }>>("player", "GET", "/v1/me/library")).find((e) => e.slug === own)).toMatchObject({
    plays_in_browser: true,
  });
  await added.click();
  await expect(add).toHaveAttribute("aria-pressed", "false");

  // Pressing Play is the step to rate it, right at the top, on the website too.
  expect((await call<{ mine: { played: boolean } }>("player", "GET", `/v1/apps/${own}/ratings`)).mine.played).toBe(false);
  await expect(head(page).getByText("Press Play to rate it.")).toBeVisible();
  await expect(head(page).getByRole("link", { name: "Play" })).toHaveAttribute("href", playAddress(own));
  await expect(head(page).getByRole("link", { name: "Get the SLAPorSLOP app" })).toHaveCount(0);

  const launch = page.waitForRequest((r) => r.method() === "POST" && isLaunch(own)(new URL(r.url())));
  const tab = await newTab(page, () => box(page).getByRole("link", { name: "Play" }).click());
  expect(new URL((await launch).url()).searchParams.get("platform")).toBe("browser");
  expect(tab.url()).toBe(playAddress(own));
  expect(await tab.evaluate(() => window.opener)).toBeNull();
  await tab.close();

  // Recorded: the page asks again at once (the rating, bug reports), without a reload.
  await expect(head(page).getByRole("button", { name: /^(Rate it|Your rating: .*)$/ })).toBeVisible();
  const ratings = page.getByRole("region", { name: "Ratings" });
  const slaps = ratings.getByRole("button", { name: "Slaps", exact: true });
  await expect(slaps).toHaveAttribute("aria-pressed", "false");
  await slaps.click();
  await expect(slaps).toHaveAttribute("aria-pressed", "true");
  expect((await call<{ mine: { vote: string } }>("player", "GET", `/v1/apps/${own}/ratings`)).mine.vote).toBe("not_slop");

  // A bug report says it was played in the browser; SLAPorSLOP fills in the version Play opens.
  const bugs = page.locator("#bug-reports");
  await bugs.getByRole("button", { name: "Report a bug" }).click();
  await bugs.getByRole("textbox", { name: "What's wrong, in a line" }).fill("The moon falls through the floor");
  await bugs.getByRole("textbox", { name: /What happened/ }).fill("Tap fast three times: the moon drops off the screen.");
  const filed = page.waitForRequest((r) => r.method() === "POST" && new URL(r.url()).pathname === `/v1/apps/${own}/bug-reports`);
  await bugs.getByRole("button", { name: "Send to the maker" }).click();
  const body = (await filed).postDataJSON();
  expect(body.platform).toBe("browser");
  expect(body.version).toBeUndefined();
  await expect(bugs.getByRole("status")).toHaveText("Sent to the maker. You'll be told when they answer.");
  const reports = await call<{ reports: Array<{ title: string; platform: string; version: string }> }>("pendingCreative", "GET", `/v1/apps/${own}/bug-reports?status=all`);
  expect(reports.reports.find((r) => r.title === "The moon falls through the floor")).toMatchObject({ platform: "browser", version: "1.0.0" });
});

test("in the SLAPorSLOP app, the store page's Play records the play first, then opens the system browser; nothing is installed", async ({ page }) => {
  await asDesktop(page, { state: "none" });
  await signIn(page, "player");
  // The address SLAPorSLOP gives at that moment wins; the app opens only https.
  const sent = await answerLaunches(page, game, "https://orbit-tap.example/play/");
  try {
    await page.goto(`/app/${game}`);
    const play = box(page).getByRole("link", { name: "Play" });
    await expect(play).toHaveAccessibleDescription(/^Opens 127\.0\.0\.1:\d+ in your browser\. Nothing to download or install\.$/);
    await expect(box(page).getByRole("button", { name: "Install" })).toHaveCount(0);
    await expect(page.getByText("No approved build yet.")).toHaveCount(0);
    await play.click();
    await expect.poll(() => openedUrls(page)).toEqual(["https://orbit-tap.example/play/"]);
    expect(sent.map((u) => u.searchParams.get("platform"))).toEqual(["browser"]);
    const calls = await desktopCalls(page);
    const launchAt = calls.findIndex((c) => c.cmd === "api_request" && c.args.methodName === "POST" && String(c.args.path).startsWith(`/v1/apps/${game}/launch`));
    const openAt = calls.findIndex((c) => c.cmd === "plugin:opener|open_url");
    expect(launchAt).toBeGreaterThanOrEqual(0);
    expect(launchAt).toBeLessThan(openAt);
    // Nothing installed or started here, and no build asked for.
    expect(calls.filter((c) => ["install_app", "launch_app", "missing_runtimes"].includes(c.cmd))).toEqual([]);
    expect(calls.filter((c) => c.cmd === "api_request" && String(c.args.path).endsWith("/manifest"))).toEqual([]);

    // Install would have added it to the library; here a button does.
    await box(page).getByRole("button", { name: "+ Add to library" }).click();
    await expect(box(page).getByRole("button", { name: "✓ In your library" })).toHaveAttribute("aria-pressed", "true");
    await axe(page);
  } finally {
    await tryCall("player", "DELETE", `/v1/me/library/${game}`);
  }
});

test("in the SLAPorSLOP app, Play opens nothing when the address SLAPorSLOP answers with isn't https", async ({ page }) => {
  await asDesktop(page, { state: "none" });
  await signIn(page, "player");
  // The store page's address is https here; the launch answers with plain http, and that's what
  // the app would open, so it opens nothing.
  const store = "https://orbit-tap.example/play/";
  await patchJson(page, path(`/v1/apps/${game}`), (body) => ({ ...body, play_address: store }));
  const sent = await answerLaunches(page, game, "http://orbit-tap.example/play/");
  await page.goto(`/app/${game}`);
  const play = box(page).getByRole("link", { name: "Play" });
  await expect(play).toHaveAttribute("href", store);
  await play.click();
  await expect(box(page).getByRole("alert")).toHaveText("Not opened: the SLAPorSLOP app opens only https addresses.");
  expect(sent).toHaveLength(1);
  expect(await openedUrls(page)).toEqual([]);
  expect((await desktopCalls(page)).filter((c) => c.cmd === "plugin:opener|open_url")).toEqual([]);
});

test("on the website, the library has browser games in their own group, with Play and nothing to install", async ({ page }) => {
  await call("player", "PUT", `/v1/me/library/${game}`);
  try {
    const sent = await answerLaunches(page, game);
    await signIn(page, "player");
    await page.goto(`/library/${game}`);
    const group = side(page).getByRole("heading", { name: /Plays in your browser/ });
    await expect(group).toContainText("(1)");
    await expect(row(page, "Orbit Tap")).not.toHaveClass(/dim/);
    await expect(side(page).getByRole("group", { name: /In your library/ }).getByRole("link", { name: /^Orbit Tap/ })).toHaveCount(0);

    const orbit = app(page, "Orbit Tap");
    const play = orbit.getByRole("link", { name: "Play" });
    await expect(play).toHaveAttribute("href", ids.browserPlayAddress);
    await expect(play).toHaveAttribute("rel", "noopener noreferrer");
    await expect(orbit.locator(".lib-bar-label")).toHaveText("Plays in your browser");
    await expect(orbit.getByRole("link", { name: "Get the SLAPorSLOP app to play" })).toHaveCount(0);
    await expect(orbit.getByRole("button", { name: /^(Install|Settings for this app)$/ })).toHaveCount(0);
    await expect(orbit.getByRole("button", { name: "✓ In your library" })).toBeVisible();
    // Before playing: Play above is the step, not the SLAPorSLOP app.
    const why = orbit.locator(".rate-prompt-callout").getByRole("group", { name: "Rate it" });
    await expect(why.getByText("Press Play to rate it.")).toBeVisible();
    await expect(why.getByRole("link", { name: "Get the SLAPorSLOP app" })).toHaveCount(0);
    // The info button says where it plays.
    await orbit.getByRole("button", { name: "About Orbit Tap" }).click();
    await expect(orbit.getByRole("group", { name: "About Orbit Tap" }).locator("dt", { hasText: "Plays at" }).locator("xpath=following-sibling::dd[1]")).toHaveText(
      new URL(ids.browserPlayAddress).host,
    );
    await page.keyboard.press("Escape");

    // The filter.
    await side(page).getByRole("combobox", { name: "Show" }).selectOption({ label: "Plays in your browser" });
    await expect(side(page).locator(".lib-row-name")).toHaveText(["Orbit Tap"]);
    await axe(page);
    await side(page).getByRole("combobox", { name: "Show" }).selectOption({ label: "All" });

    const tab = await newTab(page, () => play.click());
    expect(tab.url()).toBe(ids.browserPlayAddress);
    expect(await tab.evaluate(() => window.opener)).toBeNull();
    await tab.close();
    await expect.poll(() => sent.length).toBe(1);
    expect(sent[0].searchParams.get("platform")).toBe("browser");
  } finally {
    await tryCall("player", "DELETE", `/v1/me/library/${game}`);
  }
});

test("in the SLAPorSLOP app, the library plays browser games through the system browser, never as an install", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await asDesktop(page, { state: "none" }, { installed: [{ slug: "castle-rock", title: "Castle Rock", maker: "Sam Seed", version: "2.1.0" }] });
  await call("player", "PUT", `/v1/me/library/${game}`);
  try {
    await answerLaunches(page, game, "https://orbit-tap.example/play/");
    await signIn(page, "player");
    await page.goto(`/library/${game}`);
    // Installed first, then the browser games, never under Not installed.
    const groups = side(page).locator("h2.lib-group");
    await expect(groups.first()).toContainText("Installed");
    await expect(groups.nth(1)).toContainText("Plays in your browser (1)");
    await expect(row(page, "Orbit Tap")).not.toHaveClass(/dim/);
    await expect(side(page).getByRole("group", { name: /Not installed/ }).getByRole("link", { name: /^Orbit Tap/ })).toHaveCount(0);

    const orbit = app(page, "Orbit Tap");
    await expect(orbit.getByText("INSTALLED", { exact: true })).toHaveCount(0);
    await expect(orbit.getByRole("button", { name: /^(Install|Settings for this app)$/ })).toHaveCount(0);
    await expect(orbit.getByRole("tab", { name: "Saves" })).toHaveCount(0);
    await orbit.getByRole("link", { name: "Play" }).click();
    await expect.poll(() => openedUrls(page)).toEqual(["https://orbit-tap.example/play/"]);
    const calls = await desktopCalls(page);
    expect(calls.filter((c) => ["install_app", "launch_app"].includes(c.cmd))).toEqual([]);
    // Updates are asked about installed apps only.
    const updates = calls.filter((c) => c.cmd === "api_request" && String(c.args.path).startsWith("/v1/library/updates"));
    for (const u of updates) expect(String(u.args.path)).not.toContain(game);
    // It can be taken out of the library here, where nothing uninstalls it.
    await orbit.getByRole("button", { name: "✓ In your library" }).click();
    await expect(orbit.getByRole("button", { name: "+ Add to library" })).toHaveAttribute("aria-pressed", "false");
    await axe(page);
  } finally {
    await tryCall("player", "DELETE", `/v1/me/library/${game}`);
  }
});

test("after Play in the library, it asks at once: Slap or Slop?", async ({ page }) => {
  await call("player", "PUT", `/v1/me/library/${own}`);
  await tryCall("player", "POST", `/v1/apps/${own}/vote`, { vote: null });
  try {
    await signIn(page, "player");
    await page.goto(`/library/${own}`);
    const comet = app(page, ownTitle);
    const launch = page.waitForRequest((r) => r.method() === "POST" && isLaunch(own)(new URL(r.url())));
    const tab = await newTab(page, () => comet.getByRole("link", { name: "Play" }).click());
    await launch;
    await tab.close();
    const prompt = comet.getByRole("region", { name: `Rate ${ownTitle}` });
    await expect(prompt.getByRole("heading", { name: `You've played ${ownTitle}: does it slap, or is it slop?` })).toBeVisible();
    await expect(comet.locator(".rate-prompt-callout")).toHaveCount(0);
  } finally {
    await tryCall("player", "DELETE", `/v1/me/library/${own}`);
  }
});
