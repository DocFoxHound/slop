// SPDX-License-Identifier: AGPL-3.0-or-later
// Browse (web/src/pages/Listing.tsx, catalog.rs `browse`): a tree of every category on the left
// and the top of each in a reel; a category's page with its own sidebar of sub-genres and a reel
// for each; everything in a category on its own page; a sub-genre's page marked in that sidebar.
// Orrery Lessons is this file's own (Learning: Science and Coding) and leaves the store at the end.
import type { Page } from "@playwright/test";

import { expect, test } from "./fixtures";
import { call, delist, failApi, listedApp } from "./store-helpers";

const orrery = "orrery-lessons";

test.beforeAll(async () => {
  // A summary without "store" in it: other files look for the STORE link by name while it's listed.
  await listedApp(orrery, "Orrery Lessons", { tags: ["science", "coding"], summary: "Planets on wires: learn the sky by winding it." });
});

test.afterAll(async () => {
  await delist(orrery);
});

const tile = (page: Page, slug: string) => page.locator(`a.tile[href="/app/${slug}"]`);
const section = (page: Page, name: string) =>
  page.locator(".browse-section").filter({ has: page.getByRole("heading", { name, exact: true, level: 2 }) });

type Category = { slug: string; name: string; tags: Array<{ slug: string; name: string }> };

test("Browse: every category in a tree that opens to its sub-genres, and the top of each in one request", async ({ page }) => {
  const categories = await call<Category[]>(null, "GET", "/v1/categories");
  const asked: string[] = [];
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (u.pathname === "/v1/browse" || u.pathname === "/v1/apps") asked.push(`${u.pathname}${u.search}`);
  });
  await page.goto("/browse");
  await expect(page.getByRole("heading", { name: "All categories", level: 1 })).toBeVisible();

  // The tree: each category goes to its page; its sub-genres open under it, by mouse or keyboard.
  const tree = page.getByRole("navigation", { name: "Categories" });
  for (const c of categories) {
    await expect(tree.getByRole("link", { name: c.name, exact: true })).toHaveAttribute("href", `/browse/${c.slug}`);
  }
  const toggle = tree.getByRole("button", { name: "Learning sub-genres" });
  const science = tree.getByRole("link", { name: "Science", exact: true });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(science).toBeHidden();
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(science).toHaveAttribute("href", "/tag/science");
  await toggle.press("Enter");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(science).toBeHidden();

  // A section per category, in order, each with its reel.
  await expect(page.locator(".browse-section h2")).toHaveText(categories.map((c) => c.name));
  const learning = section(page, "Learning");
  await expect(learning.getByRole("list", { name: "Top of Learning this week" }).locator(`a[href="/app/${orrery}"]`)).toHaveCount(1);
  await expect(learning.getByRole("link", { name: "Open Learning →" })).toHaveAttribute("href", "/browse/learning");
  expect(asked).toEqual(["/v1/browse"]);

  await learning.getByRole("heading", { name: "Learning" }).getByRole("link").click();
  await expect(page).toHaveURL(/\/browse\/learning$/);
  await expect(page.getByRole("heading", { name: "Learning", level: 1 })).toBeVisible();
});

test("a category: its own sidebar, a reel per sub-genre, the quiet ones listed, and everything in it on its own page", async ({ page }) => {
  const categories = await call<Category[]>(null, "GET", "/v1/categories");
  const learningTags = categories.find((c) => c.slug === "learning")!.tags;
  await page.goto("/browse/learning");
  await expect(page.getByRole("heading", { name: "Learning", level: 1 })).toBeVisible();

  const side = page.getByRole("navigation", { name: "Learning sub-genres" });
  await expect(side.getByRole("link", { name: "All categories" })).toHaveAttribute("href", "/browse");
  await expect(side.getByRole("link", { name: "Learning", exact: true })).toHaveAttribute("aria-current", "page");
  for (const t of learningTags) await expect(side.getByRole("link", { name: t.name, exact: true })).toHaveAttribute("href", `/tag/${t.slug}`);

  // Sub-genres with apps get a reel; the rest are listed after them.
  const sciences = section(page, "Science");
  await expect(sciences.locator(`a.tile[href="/app/${orrery}"]`)).toHaveCount(1);
  await expect(sciences.getByRole("link", { name: "See all Science →" })).toHaveAttribute("href", "/tag/science");
  await expect(section(page, "Coding").locator(`a.tile[href="/app/${orrery}"]`)).toHaveCount(1);
  const quiet = page.locator(".browse-quiet");
  await expect(quiet.getByRole("heading", { name: "Nothing here yet" })).toBeVisible();
  await expect(quiet.getByRole("link", { name: "History", exact: true })).toHaveAttribute("href", "/tag/history");
  await expect(quiet.getByRole("link", { name: "Science", exact: true })).toHaveCount(0);

  // Everything in it, with the filters.
  await page.getByRole("main").getByRole("link", { name: "Everything in Learning →" }).click();
  await expect(page).toHaveURL(/\/browse\/learning\/all$/);
  await expect(page.getByRole("heading", { name: "Everything in Learning", level: 1 })).toBeVisible();
  await expect(tile(page, orrery)).toHaveCount(1);
  await expect(page.locator(".filters")).toBeVisible();
  await expect(side.getByRole("link", { name: "Everything in Learning" })).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("navigation", { name: "Store" }).getByRole("button", { name: "More" })).toHaveClass(/on/);

  // A sub-genre's page: the same sidebar, with it marked.
  await side.getByRole("link", { name: "Science", exact: true }).click();
  await expect(page).toHaveURL(/\/tag\/science$/);
  await expect(page.getByRole("heading", { name: "Science", level: 1 })).toBeVisible();
  await expect(side.getByRole("link", { name: "Science", exact: true })).toHaveAttribute("aria-current", "page");
  await expect(side.getByRole("link", { name: "Learning", exact: true })).not.toHaveAttribute("aria-current", "page");
  await expect(tile(page, orrery)).toHaveCount(1);
});

test("a failed Browse request offers Try again", async ({ page }) => {
  await failApi(page, (u) => u.pathname === "/v1/browse", { status: 503, message: "SLAPorSLOP is busy. Try again soon." });
  await page.goto("/browse/learning");
  await expect(page.getByRole("alert")).toContainText("SLAPorSLOP is busy. Try again soon.");
  await page.unrouteAll();
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(section(page, "Science").locator(`a.tile[href="/app/${orrery}"]`)).toHaveCount(1);
});

test("on a phone: a category's sidebar is a row that scrolls by itself, and nothing scrolls the page sideways", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const fits = () => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);
  await page.goto("/browse");
  await expect(section(page, "Learning").locator(`a.tile[href="/app/${orrery}"]`)).toHaveCount(1);
  expect(await fits()).toBe(true);

  await page.goto("/browse/games");
  const side = page.getByRole("navigation", { name: "Games sub-genres" });
  await expect(side.getByRole("link", { name: "Games", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Games", level: 1 })).toBeInViewport();
  expect(await side.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
  expect(await fits()).toBe(true);
  // The last sub-genre is a sideways scroll away, and is there.
  const last = side.getByRole("link").last();
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
  expect(await fits()).toBe(true);
});
