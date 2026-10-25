// SPDX-License-Identifier: AGPL-3.0-or-later
// The store bar along the top of store pages (web/src/components/StoreNav.tsx): Browse,
// Recommended, Games, Utilities and More, the store search, and Slop's own links at the bottom.
// Read-only.
import type { Page } from "@playwright/test";

import { call } from "./accounts-helpers";
import { expect, server, signIn, test } from "./fixtures";

const bar = (page: Page) => page.getByRole("navigation", { name: "Store" });

test("each section goes to its page and is marked while you're there", async ({ page }) => {
  await page.goto("/");
  await expect(bar(page).locator("[aria-current=page]")).toHaveCount(0);
  for (const [name, path, heading] of [
    ["Browse", "/browse", "All categories"],
    ["Recommended", "/recommended", "Recommended"],
    ["Games", "/browse/games", "Games"],
    ["Utilities", "/browse/utilities", "Utilities"],
  ]) {
    await bar(page).getByRole("link", { name, exact: true }).click();
    await expect(page).toHaveURL(`${server.url}${path}`);
    await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
    await expect(bar(page).getByRole("link", { name, exact: true })).toHaveAttribute("aria-current", "page");
    await expect(bar(page).locator("[aria-current=page]")).toHaveCount(1);
  }
  // A tag belongs to its category's section.
  const categories = await call<Array<{ slug: string; tags: Array<{ slug: string }> }>>(null, "GET", "/v1/categories");
  const gameTag = categories.find((c) => c.slug === "games")!.tags[0].slug;
  await page.goto(`/tag/${gameTag}`);
  await expect(bar(page).getByRole("link", { name: "Games", exact: true })).toHaveAttribute("aria-current", "page");
});

test("More opens the rest of the store, closes with Escape, and is marked for its pages", async ({ page }) => {
  await page.goto("/");
  const more = bar(page).getByRole("button", { name: "More" });
  await expect(more).toHaveAttribute("aria-expanded", "false");
  await more.click();
  await expect(more).toHaveAttribute("aria-expanded", "true");
  const menu = page.locator(".store-more-menu");
  for (const [name, href] of [
    ["Tools", "/browse/tools"],
    ["Creative", "/browse/creative"],
    ["Learning", "/browse/learning"],
    ["Coming soon & beta", "/upcoming"],
    ["Everything", "/search"],
    ["Made with people", "/made-with-people"],
    ["Community builds", "/community"],
    ["Teams", "/teams"],
    ["Publish your app", "/publish"],
    ["How to publish", "/publish/guide"],
    ["Fix bounties", "/bounties"],
  ]) {
    await expect(menu.getByRole("link", { name })).toHaveAttribute("href", href);
  }
  // Games and Utilities are on the bar itself.
  await expect(menu.getByRole("link", { name: "Games", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await expect(more).toBeFocused();
  await expect(more).toHaveAttribute("aria-expanded", "false");

  await more.click();
  await menu.getByRole("link", { name: "Tools" }).click();
  await expect(page).toHaveURL(/\/browse\/tools$/);
  await expect(menu).toBeHidden();
  await expect(more).toHaveClass(/on/);
  await page.goto("/upcoming");
  await expect(bar(page).getByRole("button", { name: "More" })).toHaveClass(/on/);
  // A click elsewhere closes it.
  await more.click();
  await expect(menu).toBeVisible();
  await page.mouse.click(8, 600);
  await expect(menu).toBeHidden();
});

test("the bar is on the store's pages and app pages, not elsewhere", async ({ page }) => {
  for (const path of ["/", "/browse", "/search?q=castle", "/upcoming", "/community", "/made-with-people", "/recommended", "/app/castle-rock"]) {
    await page.goto(path);
    await expect(bar(page), path).toBeVisible();
  }
  for (const path of ["/library", "/people", "/bounties"]) {
    await page.goto(path);
    await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
    await expect(bar(page), path).toHaveCount(0);
  }
});

test("the store front is all apps; Slop's own links are at the bottom", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("complementary", { name: "Browse" })).toHaveCount(0);
  const footer = page.getByRole("navigation", { name: "About Slop" });
  for (const [name, href] of [
    ["Terms & policies", "/legal"],
    ["Content Policy & reports", "/legal/content"],
    ["Check a file", "/verify"],
    ["Source code", "/source"],
    ["Contact", "/contact"],
    ["Code signing policy", "https://github.com/DocFoxHound/slop/blob/main/docs/CODE-SIGNING.md"],
  ]) {
    await expect(footer.getByRole("link", { name })).toHaveAttribute("href", href);
  }
});

test("Recommended: picks of your own when signed in; popular picks and a way to sign in when not", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/recommended");
  await expect(visitor.getByText("Picks of your own.")).toBeVisible();
  await expect(visitor.getByRole("link", { name: "Sign in for your picks" })).toHaveAttribute("href", "/signin?return_to=%2Frecommended");
  await expect(visitor.getByRole("heading", { name: "Popular picks" })).toBeVisible();
  await expect(visitor.locator(".tile").first()).toBeVisible();
  await expect(visitor.getByRole("heading", { name: "For you" })).toHaveCount(0);

  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto("/recommended");
  await expect(pat.getByRole("heading", { name: "For you" })).toBeVisible();
  await expect(pat.getByText(/^Based on .+, plus something different every fourth$/)).toBeVisible();
  await expect(pat.locator(".tile").first()).toBeVisible();
  await expect(pat.getByText("Picks of your own.")).toHaveCount(0);
});

test("on a phone the bar fits: sections scroll, search has its own row, More still opens", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await expect(bar(page).getByRole("searchbox", { name: "Search the store" })).toBeVisible();
  await bar(page).getByRole("button", { name: "More" }).click();
  const tools = page.locator(".store-more-menu").getByRole("link", { name: "Tools" });
  await expect(tools).toBeInViewport();
  await tools.click();
  await expect(page).toHaveURL(/\/browse\/tools$/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});
