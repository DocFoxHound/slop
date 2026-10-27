// SPDX-License-Identifier: AGPL-3.0-or-later
// The list pages: search and its filters, Show more, categories and tags, Coming soon & beta,
// Community builds, Made with people, and How ratings work.
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";
import { delist, failApi, listedApp } from "./store-helpers";

// Vera's: a beta game with full controller support, and a coming-soon one.
const beta = "gamepad-gulls";
const soon = "tidepool-tales";

test.beforeAll(async () => {
  await listedApp(beta, "Gamepad Gulls", { who: "creative", stage: "beta", tags: ["platformer"], accessibility: ["controller"] });
  await listedApp(soon, "Tidepool Tales", { who: "creative", stage: "coming_soon", tags: ["visual-novel"] });
});

test.afterAll(async () => {
  await delist(beta, soon);
});

/** The tiles shown, by address (a tile with a picture shows no title of its own). */
const tile = (page: Page, slug: string) => page.locator(`a.tile[href="/app/${slug}"]`);
/** These apps show and those don't (other tests' apps may show too). */
async function shows(page: Page, yes: string[], no: string[]) {
  for (const slug of yes) await expect(tile(page, slug)).toHaveCount(1);
  for (const slug of no) await expect(tile(page, slug)).toHaveCount(0);
}
const select = (page: Page, name: string) =>
  page.locator(".filters label").filter({ hasText: new RegExp(`^${name}`) }).getByRole("combobox");

test("search: each filter goes in the address and changes the results", async ({ page }) => {
  await page.goto("/search?q=Lantern");
  await expect(page.getByRole("heading", { name: "Results for “Lantern”", level: 1 })).toBeVisible();
  // With words to search for, the best match comes first.
  await expect(select(page, "Sort")).toHaveValue("relevance");
  await shows(page, ["lantern-keep"], ["castle-rock", "lantern-keep-linux"]);
  await select(page, "Sort").selectOption({ label: "A–Z" });
  await expect(page).toHaveURL(/[?&]sort=title/);

  // Without, there's no "best match".
  await page.goto("/search");
  await expect(page.getByRole("heading", { name: "Everything", level: 1 })).toBeVisible();
  await expect(select(page, "Sort")).toHaveValue("recent");
  await expect(select(page, "Sort").locator("option", { hasText: "Best match" })).toHaveCount(0);
  await expect(tile(page, "castle-rock")).toHaveCount(1);

  await select(page, "Show").selectOption({ label: "Beta" });
  await expect(page).toHaveURL(/[?&]stage=beta/);
  await expect(tile(page, beta)).toHaveCount(1);
  await expect(tile(page, "castle-rock")).toHaveCount(0);
  await select(page, "Show").selectOption({ label: "Upcoming (coming soon and beta)" });
  await expect(page).toHaveURL(/[?&]stage=upcoming/);
  await expect(tile(page, beta)).toHaveCount(1);
  await expect(tile(page, soon)).toHaveCount(1);
  await select(page, "Show").selectOption({ label: "Everything" });
  await expect(page).not.toHaveURL(/stage=/);

  // Runs on: this computer is marked (the browser says Windows); Linux includes community builds.
  await expect(select(page, "Runs on").locator("option", { hasText: "Windows (this computer)" })).toHaveCount(1);
  await select(page, "Runs on").selectOption("linux");
  await expect(page).toHaveURL(/[?&]runs_on=linux/);
  await expect(page.getByText("Includes apps that run on Linux through a community build.")).toBeVisible();
  await shows(page, ["lantern-keep"], ["castle-rock", "neon-drift"]);
  await select(page, "Runs on").selectOption("");

  await select(page, "Slaps").selectOption({ label: "70% or more" });
  await expect(page).toHaveURL(/[?&]not_slop=70/);
  await shows(page, ["lantern-keep"], ["castle-rock", "pocket-golf"]);
  await select(page, "Slaps").selectOption("");

  await select(page, "Accessibility").selectOption({ label: "Full controller support" });
  await expect(page).toHaveURL(/[?&]accessibility=controller/);
  await shows(page, [beta], ["castle-rock", "lantern-keep"]);

  // Nonsense in the address is left out.
  await page.goto("/search?not_slop=500&runs_on=mac&stage=nope");
  await expect(select(page, "Slaps")).toHaveValue("");
  await expect(select(page, "Runs on")).toHaveValue("");
  await expect(tile(page, "castle-rock")).toHaveCount(1);
});

test("search with no results says so; a failed search offers Try again", async ({ page }) => {
  await page.goto("/search?q=zzqqxx-nothing");
  await expect(page.getByText("Nothing matches. Try another search or filter.")).toBeVisible();

  let fail = true;
  await page.route(
    (u) => u.pathname === "/v1/apps",
    (route) => (fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "SLAPorSLOP is busy. Try again soon." } } }) : route.fallback()),
  );
  await page.goto("/search?q=Castle");
  await expect(page.getByRole("alert")).toContainText("SLAPorSLOP is busy. Try again soon.");
  fail = false;
  await page.getByRole("button", { name: "Try again" }).click();
  await shows(page, ["castle-rock"], ["lantern-keep"]);
});

test("Show more adds the next page, says when it can't, and starts over when a filter changes", async ({ page }) => {
  let failNext = true;
  let first: unknown;
  await page.route(
    (u) => u.pathname === "/v1/apps",
    async (route) => {
      const url = new URL(route.request().url());
      const res = await route.fetch();
      const body = await res.json();
      if (!url.searchParams.get("offset")) {
        first = body.apps[0];
        return route.fulfill({ response: res, json: { ...body, next_offset: 24 } });
      }
      if (failNext) {
        failNext = false;
        return route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "SLAPorSLOP is busy. Try again soon." } } });
      }
      const more = { ...(first as object), slug: "page-two-pinnace", title: "Page Two Pinnace" };
      return route.fulfill({ json: { apps: [more], next_offset: null } });
    },
  );
  await page.goto("/search");
  await expect(tile(page, "castle-rock")).toBeVisible();
  const count = await page.locator("a.tile").count();
  await page.getByRole("button", { name: "Show more" }).click();
  await expect(page.getByRole("alert")).toHaveText("SLAPorSLOP is busy. Try again soon.");
  await expect(page.locator("a.tile")).toHaveCount(count);
  await page.getByRole("button", { name: "Show more" }).click();
  await expect(page.locator("a.tile").last()).toHaveAttribute("href", "/app/page-two-pinnace");
  await expect(page.getByRole("alert")).toHaveCount(0);
  // The last page: no more button.
  await expect(page.getByRole("button", { name: "Show more" })).toHaveCount(0);

  // A new sort starts from the first page again.
  await select(page, "Sort").selectOption({ label: "A–Z" });
  await expect(page).toHaveURL(/[?&]sort=title/);
  await expect(tile(page, "castle-rock")).toBeVisible();
  await expect(tile(page, "page-two-pinnace")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Show more" })).toBeVisible();
});

test("categories and tags: each has its page, its tags and its apps; unknown ones aren't found", async ({ page }) => {
  await page.goto("/browse");
  await expect(page.getByRole("heading", { name: "All categories", level: 1 })).toBeVisible();
  const games = page.locator(".browse-group").filter({ has: page.getByRole("link", { name: "Games", exact: true }) });
  await expect(games.getByText("Things you play")).toBeVisible();
  await expect(games.getByRole("link", { name: "Puzzle" })).toHaveAttribute("href", "/tag/puzzle");
  await games.getByRole("link", { name: "Games", exact: true }).click();

  await expect(page).toHaveURL(/\/browse\/games$/);
  await expect(page.getByRole("heading", { name: "Games", level: 1 })).toBeVisible();
  await expect(page.locator(".crumbs").getByRole("link", { name: "Categories" })).toHaveAttribute("href", "/browse");
  await expect(tile(page, "lantern-keep")).toHaveCount(1);
  await page.locator(".tag-cloud").getByRole("link", { name: "Puzzle" }).click();

  await expect(page).toHaveURL(/\/tag\/puzzle$/);
  await expect(page.getByRole("heading", { name: "# Puzzle", level: 1 })).toBeVisible();
  await shows(page, ["lantern-keep"], ["castle-rock", "pocket-golf"]);
  // The crumbs lead back up to the tag's category.
  await page.locator(".crumbs").getByRole("link", { name: "Games" }).click();
  await expect(page).toHaveURL(/\/browse\/games$/);

  await page.goto("/browse/no-such-category");
  await expect(page.getByText("Page not found.")).toBeVisible();
  await page.goto("/tag/no-such-tag");
  await expect(page.getByText("Page not found.")).toBeVisible();
  await page.getByRole("link", { name: "Back to the store" }).click();
  await expect(page).toHaveURL(/\/$/);
});

test("Coming soon & beta: the tabs switch stages, with their own sorts", async ({ page }) => {
  await page.goto("/upcoming");
  await expect(page.getByRole("heading", { name: "Coming soon & beta", level: 1 })).toBeVisible();
  const tabs = page.getByRole("tablist");
  await expect(tabs.getByRole("tab", { name: "Coming soon" })).toHaveAttribute("aria-selected", "true");
  await expect(tile(page, soon)).toHaveCount(1);
  await expect(tile(page, beta)).toHaveCount(0);
  await expect(select(page, "Sort").locator("option")).toHaveText(["Most anticipated", "Recently updated", "A–Z"]);
  // The stage is the tab's: no Show filter here.
  await expect(page.locator(".filters label").filter({ hasText: "Show" })).toHaveCount(0);

  await tabs.getByRole("tab", { name: "Beta" }).click();
  await expect(page).toHaveURL(/\/upcoming\?stage=beta$/);
  await expect(tabs.getByRole("tab", { name: "Beta" })).toHaveAttribute("aria-selected", "true");
  await expect(tile(page, beta)).toHaveCount(1);
  await expect(tile(page, soon)).toHaveCount(0);
  await expect(page.locator(".tile").filter({ hasText: "Gamepad Gulls" }).getByText("BETA")).toBeVisible();
});

test("Community builds lists only branches; Made with people only apps that hired creatives", async ({ page }) => {
  await page.goto("/community");
  await expect(page.getByRole("heading", { name: "Community builds", level: 1 })).toBeVisible();
  await shows(page, ["lantern-keep-linux"], ["lantern-keep", "castle-rock"]);
  // Other test files add community builds of their own, so the badge is looked for on the seed's.
  await expect(tile(page, "lantern-keep-linux").getByText("COMMUNITY")).toBeVisible();
  await expect(select(page, "Sort").locator("option")).toHaveText(["Most Slaps", "Recently updated", "A–Z"]);

  const asked = page.waitForRequest((r) => new URL(r.url()).searchParams.get("made_with_people") === "true");
  await page.goto("/made-with-people");
  await asked;
  await expect(page.getByRole("heading", { name: "Made with people", exact: true, level: 1 })).toBeVisible();
  // Cleo hired Vera for Cleo's Tavern.
  await shows(page, ["cleos-tavern"], ["castle-rock", "lantern-keep"]);
  await expect(tile(page, "cleos-tavern").getByText("Made with people", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "People", exact: true }).first()).toBeVisible();
});

test("How ratings work explains the weighting", async ({ page }) => {
  await page.goto("/ratings");
  await expect(page.getByRole("heading", { name: "How ratings work", level: 1 })).toBeVisible();
  await expect(page.getByRole("heading", { name: "What you see" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Your votes and privacy" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "Overwhelmingly Slaps" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "10 hours or more" })).toBeVisible();
});

test("a failed category list offers Try again", async ({ page }) => {
  await failApi(page, (u) => u.pathname === "/v1/categories", { status: 503, message: "SLAPorSLOP is busy. Try again soon." });
  await page.goto("/browse");
  await expect(page.getByRole("alert")).toContainText("SLAPorSLOP is busy. Try again soon.");
  await page.unrouteAll();
  await page.getByRole("button", { name: "Try again" }).click();
  // The categories' own list (the store bar always has Games).
  await expect(page.locator(".browse-group").getByRole("link", { name: "Games", exact: true })).toBeVisible();
});
