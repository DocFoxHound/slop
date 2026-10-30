// SPDX-License-Identifier: AGPL-3.0-or-later
// Discover (web/src/pages/Discover.tsx, crates/slop-api/src/discover.rs): the store bar's tab of
// reels. Signed in: Recommended for you first, then the person's genres, each saying why, with the
// general reels mixed in. Signed out or switched off: the general reels and why there's nothing
// personal. Two puzzle games of Vera's give Pat (who 👍 Lantern Keep, a puzzle game) a genre reel;
// they leave the store when the file is done.
import type { Page } from "@playwright/test";
import { expect, signIn, test } from "./fixtures";
import { delist, failApi, listedApp, path } from "./store-helpers";

const puzzles = ["discover-puzzle-a", "discover-puzzle-b"];
const GENERAL = ["All-time favourites", "Up-and-coming", "Popular this week", "Popular this month"];

test.beforeAll(async () => {
  for (const slug of puzzles) await listedApp(slug, `Puzzle ${slug.slice(-1).toUpperCase()}`, { who: "creative", tags: ["puzzle"] });
});

test.afterAll(async () => {
  await delist(...puzzles);
});

/** A reel's section, by its heading. */
const reel = (page: Page, title: string | RegExp) =>
  page.locator("section.discover-reel").filter({ has: page.getByRole("heading", { level: 2, name: title }) });

test("signed in: Recommended for you first, then Pat's genres, each saying why", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/discover");
  await expect(page.getByRole("heading", { name: "Discover", level: 1 })).toBeVisible();
  const headings = page.locator("section.discover-reel h2");
  await expect(headings.first()).toHaveText("Recommended for you");
  const forYou = reel(page, "Recommended for you");
  await expect(forYou.getByText(/^From your library, Interested list, playtime and votes, leaning on .+\. Every fourth is something different\.$/)).toBeVisible();
  await expect(forYou.getByRole("link", { name: "Settings" })).toHaveAttribute("href", "/account");
  await expect(forYou.getByRole("list", { name: "Recommended for you" }).getByRole("listitem").first()).toBeVisible();

  // Pat 👍 Lantern Keep, a puzzle game: a reel of puzzle games Pat doesn't have.
  const puzzle = reel(page, /^Because you (like|play a lot of) Puzzle$/);
  await expect(puzzle).toBeVisible();
  await expect(puzzle.locator(".section-h .sub").first()).toHaveText(/(Slaps votes|spent the most time)/);
  await expect(puzzle.getByRole("link", { name: /^See all: Because you/ })).toHaveAttribute("href", "/tag/puzzle?sort=rating");
  for (const slug of puzzles) await expect(puzzle.locator(`a[href="/app/${slug}"]`)).toBeVisible();
  await expect(puzzle.locator('a[href="/app/lantern-keep"]')).toHaveCount(0);
  // Never anything Pat already has.
  await expect(page.locator('section.discover-reel a[href="/app/lantern-keep"]')).toHaveCount(0);
  await expect(page.getByText("Picks of your own.")).toHaveCount(0);
});

test("signed out: the general reels, why, and a way to sign in; the old address still works", async ({ page }) => {
  await page.goto("/recommended");
  await expect(page).toHaveURL(/\/discover$/);
  await expect(page.getByText("Picks of your own.")).toBeVisible();
  await expect(page.getByRole("link", { name: "Sign in for your picks" })).toHaveAttribute("href", "/signin?return_to=%2Fdiscover");
  const headings = page.locator("section.discover-reel h2");
  await expect(headings.first()).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recommended for you" })).toHaveCount(0);
  // Each general reel there is says what it's based on. (Which ones show depends on the day's
  // order: an app is in two reels at most, and this store is small.)
  const why: Record<string, RegExp> = {
    "All-time favourites": /^The best rated ever: the highest % Slaps, at least 70%, among apps with 5 or more votes\.$/,
    "Up-and-coming": /^Released in the last 60 days and most added and played this week\.$/,
    "Popular this week": /^Most players at once this week, then most added to libraries this week\.$/,
    "Popular this month": /^Most added to libraries this month, then most players\.$/,
  };
  for (const t of await headings.allTextContents()) {
    expect(GENERAL).toContain(t);
    await expect(reel(page, t).locator(".section-h .sub").first()).toHaveText(why[t]);
    await expect(reel(page, t).locator(".tile").first()).toBeVisible();
  }
});

test("with recommendations off (the UK under-18 view's default), the page says so and links to settings", async ({ page }) => {
  await signIn(page, "teen");
  await page.goto("/discover");
  await expect(page.getByText("Recommendations based on your activity are off.")).toBeVisible();
  await expect(page.locator(".signin-nudge").getByRole("link", { name: "Settings" })).toHaveAttribute("href", "/account");
  await expect(page.getByRole("heading", { name: "Recommended for you" })).toHaveCount(0);
  await expect(page.locator("section.discover-reel h2").first()).toBeVisible();
});

test("when it can't load, it says so and tries again", async ({ page }) => {
  const discover = path("/v1/discover");
  await failApi(page, discover);
  await page.goto("/discover");
  await expect(page.getByRole("alert")).toContainText("Something broke on SLAPorSLOP's side.");
  await page.unroute(discover);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.locator("section.discover-reel h2").first()).toBeVisible();
});

test("on a phone, reels scroll sideways inside the page", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signIn(page, "player");
  await page.goto("/discover");
  await expect(page.locator("section.discover-reel h2").first()).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});
