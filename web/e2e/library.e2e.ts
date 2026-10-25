// SPDX-License-Identifier: AGPL-3.0-or-later
// The library laid out like Steam's (pages/Library.tsx): the sidebar's groups, counts, search,
// filter and arrow keys; the selected app in the address; the big button on the website and in
// the Slop app (installed apps, updates, downloads and running apps come from the pretend app in
// desktop-helpers.ts); tabs; friends' ratings; the "is it Slop?" prompt after playing; a phone's
// two screens; and accessibility. The apps added to Pat's library here are taken out again.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { asDesktop, type FakeLibrary } from "./desktop-helpers";
import { expect, ids, signIn, test } from "./fixtures";
import { call, delist, listedApp, played, tryCall } from "./store-helpers";

// Remy's games for this file: one Pat has played, one coming soon he's waiting for.
const larks = "loot-larks";
const later = "loot-later";

test.beforeAll(async () => {
  await listedApp(larks, "Loot Larks", { who: "pendingCreative" });
  await listedApp(later, "Loot Later", { who: "pendingCreative", stage: "coming_soon" });
  await played("player", larks);
  for (const slug of [larks, "castle-rock", "neon-drift"]) await call("player", "PUT", `/v1/me/library/${slug}`);
  await call("player", "PUT", `/v1/me/interested/${later}`);
});

test.afterAll(async () => {
  await tryCall("player", "POST", `/v1/apps/${larks}/vote`, { vote: null });
  for (const slug of [larks, "neon-drift"]) await tryCall("player", "DELETE", `/v1/me/library/${slug}`);
  await tryCall("player", "DELETE", `/v1/me/interested/${later}`);
  await delist(larks, later);
});

const side = (page: Page) => page.getByRole("navigation", { name: "Your apps" });
const row = (page: Page, name: string) => side(page).getByRole("link", { name: new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}( – |$)`) });
const app = (page: Page, name: string) => page.getByRole("region", { name, exact: true });

async function axe(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

test("the sidebar: groups with counts, search, filter, arrow keys, and the app in the address", async ({ page }) => {
  await signIn(page, "player");
  const library = await call<Array<{ slug: string }>>("player", "GET", "/v1/me/library");
  const priv = await call<{ mine: Array<{ slug: string }>; shared: Array<{ slug: string }> }>("player", "GET", "/v1/me/private");
  const privates = new Set([...priv.mine, ...priv.shared].map((a) => a.slug));
  const inLibrary = library.filter((a) => !privates.has(a.slug)).length;

  await page.goto("/library");
  await expect(page.getByRole("heading", { name: "Library", level: 1 })).toBeVisible();
  const group = side(page).getByRole("heading", { name: /In your library/ });
  await expect(group).toContainText(`(${inLibrary})`);
  await expect(side(page).getByRole("heading", { name: /Interested/ })).toContainText("(");

  // On a wide screen the first app is open already.
  await expect(page.getByRole("region").first()).toBeVisible();

  // Searching narrows each group, Steam's "(1/23)".
  const search = side(page).getByRole("searchbox", { name: "Search your library" });
  await search.fill("LOOT LA");
  await expect(side(page).locator(".lib-row-name")).toHaveText(["Loot Larks", "Loot Later"]);
  await expect(group).toContainText(`(1/${inLibrary})`);
  await search.fill("nothing is called this");
  await expect(side(page).getByText("No apps match.")).toBeVisible();
  await search.fill("");

  // The filter shows one kind.
  await side(page).getByRole("combobox", { name: "Show" }).selectOption({ label: "Interested" });
  await expect(row(page, "Loot Later")).toBeVisible();
  await expect(row(page, "Castle Rock")).toHaveCount(0);
  await side(page).getByRole("combobox", { name: "Show" }).selectOption({ label: "All" });

  // Groups fold away.
  await group.getByRole("button").click();
  await expect(group.getByRole("button")).toHaveAttribute("aria-expanded", "false");
  await expect(row(page, "Castle Rock")).toHaveCount(0);
  await group.getByRole("button").click();

  // Choosing an app puts it in the address and on the right; Back goes to the one before.
  await row(page, "Castle Rock").click();
  await expect(page).toHaveURL(/\/library\/castle-rock$/);
  await expect(app(page, "Castle Rock").getByRole("heading", { name: "Castle Rock", level: 2 })).toBeVisible();
  await expect(row(page, "Castle Rock")).toHaveAttribute("aria-current", "page");
  await row(page, "Neon Drift").click();
  await expect(page).toHaveURL(/\/library\/neon-drift$/);
  await page.goBack();
  await expect(page).toHaveURL(/\/library\/castle-rock$/);
  await expect(app(page, "Castle Rock")).toBeVisible();

  // Arrow keys move through the list, and the app follows.
  await row(page, "Castle Rock").focus();
  await page.keyboard.press("ArrowDown");
  const next = await side(page).locator("a.lib-row:focus").getAttribute("data-slug");
  expect(next).not.toBe("castle-rock");
  await expect(page).toHaveURL(new RegExp(`/library/${next}$`));
  await expect(side(page).locator("a.lib-row:focus")).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("ArrowUp");
  await expect(page).toHaveURL(/\/library\/castle-rock$/);

  // Links to the old library's parts still open the app.
  await page.goto("/library#neon-drift");
  await expect(page).toHaveURL(/\/library\/neon-drift$/);
});

test("on the website: get the Slop app to play, tabs, the info button, and friends' ratings", async ({ page }) => {
  // Bo, Pat's friend, rated Castle Rock on his public profile.
  await page.route(
    (u) => u.pathname === `/v1/users/${ids.maker2}`,
    async (route) => {
      const res = await route.fetch();
      const profile = await res.json();
      await route.fulfill({
        response: res,
        json: { ...profile, votes: [{ slug: "castle-rock", title: "Castle Rock", my_vote: "not_slop", review: null, voted_at: "2026-10-01T00:00:00Z" }] },
      });
    },
  );
  await signIn(page, "player");
  await page.goto("/library/castle-rock");
  const castle = app(page, "Castle Rock");
  await expect(castle.getByRole("link", { name: "⬇ Get the Slop app to play" })).toHaveAttribute("href", "/download");
  await expect(castle.getByRole("button", { name: "✓ In your library" })).toBeVisible();
  await expect(castle.getByRole("button", { name: "Install" })).toHaveCount(0);

  // Friends who rated it, from what they show everyone.
  const friends = castle.locator(".lib-box").filter({ hasText: "Friends who rated it" });
  await expect(friends.getByText("1 friend rated it")).toBeVisible();
  await expect(friends.getByRole("link", { name: "Bo Builder" })).toHaveAttribute("href", `/u/${ids.maker2}`);
  await expect(friends.getByText("👍 Not Slop")).toBeVisible();
  await page.goto("/library/neon-drift");
  await expect(app(page, "Neon Drift").getByText("None of your friends has rated it on their profile yet.")).toBeVisible();

  // Tabs, by mouse and by arrow keys.
  const tabs = app(page, "Neon Drift").getByRole("tablist");
  const activity = tabs.getByRole("tab", { name: "Activity" });
  const reviews = tabs.getByRole("tab", { name: "Reviews" });
  await expect(activity).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tabpanel")).toContainText("Activity");
  await reviews.click();
  await expect(reviews).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tabpanel")).toContainText("What players wrote with their vote.");
  await page.keyboard.press("Home");
  await expect(activity).toBeFocused();
  await expect(activity).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowLeft");
  await expect(tabs.getByRole("tab").last()).toBeFocused();
  await expect(app(page, "Neon Drift").getByRole("link", { name: "Store page", exact: true })).toHaveAttribute("href", "/app/neon-drift");

  // The info button: what it is, its licence and source.
  const info = app(page, "Neon Drift").getByRole("button", { name: "About Neon Drift" });
  await info.click();
  await expect(info).toHaveAttribute("aria-expanded", "true");
  const panel = app(page, "Neon Drift").getByRole("group", { name: "About Neon Drift" });
  await expect(panel.getByText("What it does")).toBeVisible();
  await expect(panel.getByText("Licence")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(info).toBeFocused();

  // An app you're waiting for has nothing to install yet.
  await row(page, "Loot Later").click();
  await expect(app(page, "Loot Later").getByRole("button", { name: "Coming soon" })).toBeDisabled();
  await expect(app(page, "Loot Later").getByRole("button", { name: "Not interested" })).toBeVisible();

  await page.goto("/library/castle-rock");
  await axe(page);
});

test("after playing, the library asks: Slop or Not Slop? It keeps the vote, which can be changed", async ({ page }) => {
  await tryCall("player", "POST", `/v1/apps/${larks}/vote`, { vote: null });
  await signIn(page, "player");
  await page.goto(`/library/${larks}`);
  const prompt = app(page, "Loot Larks").getByRole("region", { name: "Rate Loot Larks" });
  await expect(prompt.getByRole("heading", { name: "You've played Loot Larks: is it Slop or Not Slop?" })).toBeVisible();
  await prompt.getByRole("button", { name: "👍 Not Slop" }).click();
  await expect(prompt.getByText("Your rating: 👍 Not Slop")).toBeVisible();
  expect((await call<{ mine: { vote: string } }>("player", "GET", `/v1/apps/${larks}/ratings`)).mine.vote).toBe("not_slop");
  // A written review goes with it.
  await expect(prompt.getByRole("button", { name: "Add a written review" })).toBeVisible();
  await prompt.getByRole("button", { name: "Change" }).click();
  await prompt.getByRole("button", { name: "👎 Slop" }).click();
  await expect(prompt.getByText("Your rating: 👎 Slop")).toBeVisible();
  expect((await call<{ mine: { vote: string } }>("player", "GET", `/v1/apps/${larks}/ratings`)).mine.vote).toBe("slop");

  // Not played: no prompt.
  await page.goto("/library/neon-drift");
  await expect(app(page, "Neon Drift").getByRole("heading", { level: 2 })).toBeVisible();
  await expect(page.getByRole("region", { name: /^Rate / })).toHaveCount(0);
});

const installed: FakeLibrary["installed"] = [
  { slug: "lantern-keep", title: "Lantern Keep", maker: "Mia Maker", version: "1.0.0" },
  { slug: "lantern-keep-linux", title: "Lantern Keep for Linux", maker: "Bo Builder", version: "1.0.0", build_of: "lantern-keep" },
  { slug: "castle-rock", title: "Castle Rock", maker: "Sam Seed", version: "2.1.0" },
  { slug: "mias-first", title: "Mia's First Game", maker: "Mia Maker", version: "0.9.0" },
  { slug: "pocket-golf", title: "Pocket Golf", maker: "Bo Builder", version: "1.0.0" },
];

/** Pat in the Slop app, with apps installed in different states. */
async function desktopLibrary(page: Page, extra: FakeLibrary = {}) {
  await asDesktop(page, { state: "none" }, {
    installed,
    running: ["castle-rock"],
    sandbox: true,
    saveCopies: [{ id: "copy-1", taken_at: "2026-10-20T10:00:00Z", bytes: 4096 }],
    ...extra,
  });
  await page.route(
    (u) => u.pathname === "/v1/library/updates",
    (route) =>
      route.fulfill({
        json: [
          { slug: "lantern-keep", latest: { release_id: "lk-new", version: "1.1.0" }, pending: null, builds: [], mods: [] },
          { slug: "mias-first", latest: { release_id: "release-mias-first", version: "0.9.0" }, pending: { version: "1.0.0", status: "fetching" }, builds: [] },
          { slug: "pocket-golf", latest: { release_id: "pg-new", version: "1.1.0" }, pending: null, builds: [] },
          { slug: "castle-rock", latest: { release_id: "release-castle-rock", version: "2.1.0" }, pending: null, builds: [] },
        ],
      }),
  );
  // Every app has an approved build to install.
  await page.route((u) => /^\/v1\/apps\/[^/]+\/manifest$/.test(u.pathname), (route) => route.fulfill({ json: {} }));
  await signIn(page, "player");
}

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(e as string, p), [event, payload]);

test("in the Slop app: what each app is doing, the big button, and downloads", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await desktopLibrary(page);
  await page.goto("/library/lantern-keep");

  // The groups: installed (community builds under their original), then the rest.
  await expect(side(page).getByRole("heading", { name: /Installed/ })).toContainText("(5)");
  await expect(side(page).getByRole("heading", { name: /Not installed/ })).toBeVisible();
  await expect(row(page, "Neon Drift")).toHaveClass(/dim/);
  await expect(row(page, "Lantern Keep for Linux")).toHaveClass(/nested/);

  // Each says what it's doing.
  await expect(row(page, "Lantern Keep")).toHaveAccessibleName("Lantern Keep – Update queued");
  await expect(row(page, "Castle Rock")).toHaveAccessibleName("Castle Rock – Running");
  await expect(row(page, "Mia's First Game")).toHaveAccessibleName("Mia's First Game – Scanning update");
  await expect(row(page, "Lantern Keep for Linux")).toHaveAccessibleName("Lantern Keep for Linux");
  await emit(page, "install-progress", { slug: "pocket-golf", got: 45, total: 100 });
  await expect(row(page, "Pocket Golf")).toHaveAccessibleName("Pocket Golf – Downloading 45%");

  // An update waiting: Update first, playing this version beside it, and Play's list of builds.
  const lantern = app(page, "Lantern Keep");
  await expect(lantern.getByRole("button", { name: "Update", exact: true })).toBeVisible();
  await expect(lantern.getByText("Update queued", { exact: true })).toBeVisible();
  await expect(lantern.getByText("0% complete · version 1.0.0 → 1.1.0")).toBeVisible();
  await expect(lantern.getByRole("button", { name: "▶ Play 1.0.0" })).toBeVisible();
  await expect(lantern.getByRole("combobox", { name: "Which version of Lantern Keep Play starts" })).toContainText(
    "Lantern Keep for Linux (community, by Bo Builder)",
  );
  await expect(lantern.getByText(/^\d+% Not Slop$/)).toBeVisible();
  await expect(lantern.locator(".lib-hero").getByText("INSTALLED", { exact: true })).toBeVisible();

  // Downloads: one going, one queued; then the one going finishes.
  const downloads = side(page).getByRole("button", { name: /^Downloads/ });
  await expect(downloads).toHaveAccessibleName("Downloads – 0 of 2 complete");
  await emit(page, "install-progress", { slug: "pocket-golf", got: 100, total: 100 });
  await expect(downloads).toHaveAccessibleName("Downloads – 1 of 2 complete");
  await downloads.click();
  const list = side(page).getByRole("group", { name: "Downloads" });
  await expect(list.locator("li").filter({ hasText: "Lantern Keep" })).toContainText("Update queued: 1.0.0 → 1.1.0");
  await expect(list.locator("li").filter({ hasText: "Pocket Golf" })).toContainText("Complete");
  await expect(list.locator("li").filter({ hasText: "Mia's First Game" })).toContainText("being downloaded and scanned by Slop");
  await expect(list.getByRole("button", { name: "Update 1 app now" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(list).toHaveCount(0);

  // With automatic updates off, an update is ready rather than queued.
  await side(page).getByRole("button", { name: "Library settings" }).click();
  await side(page).getByRole("checkbox", { name: /Update apps automatically/ }).uncheck();
  await expect(row(page, "Lantern Keep")).toHaveAccessibleName("Lantern Keep – Update ready");
  await expect(lantern.getByText("Update ready", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");

  // Running, being checked, not installed.
  await row(page, "Castle Rock").click();
  await expect(app(page, "Castle Rock").getByRole("button", { name: "Running" })).toBeDisabled();
  await row(page, "Mia's First Game").click();
  const mia = app(page, "Mia's First Game");
  await expect(mia.getByRole("button", { name: "▶ Play" })).toBeEnabled();
  await expect(mia.getByText("Scanning update", { exact: true })).toBeVisible();
  await expect(mia.getByText("BEING CHECKED", { exact: true })).toBeVisible();
  await row(page, "Neon Drift").click();
  await expect(app(page, "Neon Drift").getByRole("button", { name: "Install" })).toBeEnabled();
  await expect(app(page, "Neon Drift").getByText("Not installed", { exact: true })).toBeVisible();

  await axe(page);
});

test("in the Slop app: installing shows its progress; settings, saves and uninstalling", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await desktopLibrary(page, { holdInstalls: true, installable: [{ slug: "neon-drift", title: "Neon Drift", maker: "Sam Seed" }] });
  await page.goto("/library/neon-drift");
  const neon = app(page, "Neon Drift");
  await neon.getByRole("button", { name: "Install" }).click();
  await expect(neon.getByRole("button", { name: "Starting download…" })).toBeDisabled();
  await emit(page, "install-progress", { slug: "neon-drift", got: 30, total: 100 });
  await expect(neon.getByRole("button", { name: "Downloading 30%" })).toBeDisabled();
  await expect(neon.getByRole("progressbar", { name: "Download progress" })).toBeVisible();
  await expect(row(page, "Neon Drift")).toHaveAccessibleName("Neon Drift – Downloading 30%");
  await page.evaluate(() => (window as unknown as { __finishInstall: () => void }).__finishInstall());
  // Installed now: in the Installed group, with Play.
  await expect(side(page).getByRole("heading", { name: /Installed/ })).toContainText("(6)");
  await expect(neon.getByRole("button", { name: "▶ Play" })).toBeVisible();

  // The settings button: sandbox mode, saves, uninstall.
  await row(page, "Mia's First Game").click();
  const mia = app(page, "Mia's First Game");
  await mia.getByRole("button", { name: "Settings for this app" }).click();
  const settings = mia.getByRole("group", { name: "Settings for this app" });
  await settings.getByRole("checkbox", { name: /^Sandbox mode/ }).check();
  await expect(settings.getByRole("button", { name: "Run normally this time" })).toBeVisible();
  await settings.getByRole("button", { name: "Saves…" }).click();
  await expect(mia.getByRole("tab", { name: "Saves" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tabpanel").getByRole("button", { name: "Go back to this" })).toBeVisible();

  await mia.getByRole("button", { name: "Settings for this app" }).click();
  await settings.getByRole("button", { name: "Uninstall…" }).click();
  await mia.getByRole("checkbox", { name: "Delete my saves too" }).check();
  await mia.getByRole("button", { name: "Uninstall", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __desktopCalls: Array<{ cmd: string; args: Record<string, unknown> }> }).__desktopCalls
          .filter((c) => c.cmd === "uninstall_app")
          .map((c) => c.args),
      ),
    )
    .toEqual([{ slug: "mias-first", deleteSaves: true }]);
  // It's gone from the installed apps (and Pat's library doesn't have it).
  await expect(row(page, "Mia's First Game")).toHaveCount(0);
});

test("on a phone, the list and the app are two screens", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signIn(page, "player");
  await page.goto("/library");
  await expect(side(page)).toBeVisible();
  await expect(page.getByRole("region")).toHaveCount(0);
  await row(page, "Castle Rock").click();
  await expect(page).toHaveURL(/\/library\/castle-rock$/);
  await expect(side(page)).toBeHidden();
  const castle = app(page, "Castle Rock");
  await expect(castle.getByRole("heading", { name: "Castle Rock", level: 2 })).toBeFocused();
  await expect(castle.getByRole("link", { name: "⬇ Get the Slop app to play" })).toBeVisible();
  await axe(page);
  await castle.getByRole("link", { name: "← Library" }).click();
  await expect(page).toHaveURL(/\/library$/);
  await expect(side(page)).toBeVisible();
  await expect(row(page, "Castle Rock")).toBeFocused();
  // No sideways scrolling.
  expect(await page.evaluate(() => document.querySelector("main")!.scrollWidth <= window.innerWidth)).toBe(true);
});
