// SPDX-License-Identifier: AGPL-3.0-or-later
// Kids & Family (migration 0089): on the store bar after Games and second among the categories;
// its page and its tags' pages list only apps rated 7+ or under without the themes that aren't for
// children, and say so. The editor's tag picker: grouped, searchable, closing what rarely fits,
// and warning (never refusing) when Kids & Family won't list the app. Remy's two games are this
// file's own and leave the store at the end; the editor is never saved.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { expect, signIn, test } from "./fixtures";
import { delist, listedApp } from "./store-helpers";

const young = "tiny-tugboats";
const older = "grim-tugboats";

test.beforeAll(async () => {
  await listedApp(young, "Tiny Tugboats", { who: "pendingCreative", tags: ["kids", "puzzle"], min_age: 7 });
  await listedApp(older, "Grim Tugboats", { who: "pendingCreative", tags: ["kids", "puzzle"], min_age: 12 });
});

test.afterAll(async () => {
  await delist(young, older);
});

const bar = (page: Page) => page.getByRole("navigation", { name: "Store" });
const tile = (page: Page, slug: string) => page.locator(`a.tile[href="/app/${slug}"]`);

async function noSeriousAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

test("Kids & Family: after Games on the bar and the front page; lists only apps rated for it, and says so", async ({ page }) => {
  await page.goto("/");
  await expect(bar(page).locator(".store-nav-links a")).toHaveText(["Browse", "Discover", "Games", "Kids & Family", "Utilities"]);
  const cats = page.locator(".cat-grid a.cat b");
  await expect(cats.nth(0)).toHaveText("Games");
  await expect(cats.nth(1)).toHaveText("Kids & Family");

  await bar(page).getByRole("link", { name: "Kids & Family", exact: true }).click();
  await expect(page).toHaveURL(/\/browse\/family$/);
  await expect(page.getByRole("heading", { name: "Kids & Family", level: 1 })).toBeVisible();
  await expect(bar(page).getByRole("link", { name: "Kids & Family", exact: true })).toHaveAttribute("aria-current", "page");
  await expect(page.getByText("Simple, gentle fun for younger players and the whole family")).toBeVisible();
  const limit = page.locator(".age-limit");
  await expect(limit).toHaveText(
    /^Only apps rated 7\+ or under, without suggestive themes, horror, strong language or gambling themes\. Makers rate and label their own apps/,
  );
  await expect(tile(page, young)).toHaveCount(1);
  await expect(tile(page, older)).toHaveCount(0);
  // The sidebar has it second, and marks it.
  const sidebar = page.getByRole("complementary", { name: "Browse" });
  await expect(sidebar.getByRole("link", { name: "Kids & Family", exact: true })).toHaveClass(/on/);
  await noSeriousAxeViolations(page);

  // Its tags' pages keep the limit; the crumbs lead back to it.
  await page.locator(".tag-cloud").getByRole("link", { name: "Kids", exact: true }).click();
  await expect(page).toHaveURL(/\/tag\/kids$/);
  await expect(page.getByRole("heading", { name: "# Kids", level: 1 })).toBeVisible();
  await expect(page.locator(".age-limit")).toBeVisible();
  await expect(tile(page, young)).toHaveCount(1);
  await expect(tile(page, older)).toHaveCount(0);
  await expect(bar(page).getByRole("link", { name: "Kids & Family", exact: true })).toHaveAttribute("aria-current", "page");
  await page.locator(".crumbs").getByRole("link", { name: "Kids & Family" }).click();
  await expect(page).toHaveURL(/\/browse\/family$/);

  // The older game is still in the store under its other tags, without the note.
  await page.goto("/tag/puzzle");
  await expect(tile(page, young)).toHaveCount(1);
  await expect(tile(page, older)).toHaveCount(1);
  await expect(page.locator(".age-limit")).toHaveCount(0);
});

test("the editor's tags: grouped, searchable by keyboard, and a warning when Kids & Family won't list it", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/publish/new");
  const tags = page.getByRole("group", { name: /^Tags/ });
  await expect(tags.getByText("(0/10)")).toBeVisible();

  // A game: game tags are open, tool tags start closed and open from their heading.
  await expect(tags.getByRole("button", { name: "Tower Defense" })).toBeVisible();
  await expect(tags.getByRole("button", { name: "Office" })).toBeHidden();
  const tools = tags.locator("summary").filter({ hasText: /^Tools/ });
  await tools.focus();
  await page.keyboard.press("Enter");
  await expect(tags.getByRole("button", { name: "Office" })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(tags.getByRole("button", { name: "Office" })).toBeHidden();

  // Search finds tags in any group; Enter picks the only match without sending the form.
  const find = tags.getByRole("searchbox", { name: "Find a tag" });
  await find.fill("offi");
  await expect(tags.getByRole("button", { name: "Office" })).toBeVisible();
  await expect(tags.getByRole("button", { name: "Tower Defense" })).toHaveCount(0);
  await expect(tags.getByRole("status").first()).toHaveText("1 tag matches.");
  await find.press("Enter");
  await expect(page).toHaveURL(/\/publish\/new$/);
  await expect(tags.getByRole("button", { name: "Office" })).toHaveAttribute("aria-pressed", "true");
  await find.fill("nothing like this");
  await expect(tags.getByText("No tag matches “nothing like this”.")).toBeVisible();
  await find.press("Escape");
  await expect(find).toHaveValue("");
  // Everything's back; what was picked stays in sight, and its group's heading names it.
  await expect(tags.getByRole("button", { name: "Tower Defense" })).toBeVisible();
  await expect(tools).toContainText("Office");
  await tags.getByRole("button", { name: "Office" }).click();
  await expect(tags.getByText("(0/10)")).toBeVisible();

  // Kids on a 12+ game: a warning, and the form still sends (not here: nothing is saved).
  await tags.getByRole("button", { name: "Kids", exact: true }).click();
  const warning = tags.locator(".tag-warning");
  await expect(warning).toHaveCount(0);
  await page.getByLabel("Age rating").selectOption("12");
  await expect(warning).toContainText("Kids & Family lists only apps rated 7+ or under");
  await expect(warning).toContainText("This one is rated 12+, so Kids won't show it there. You can still save");
  await page.getByLabel("Age rating").selectOption("7");
  await expect(warning).toHaveCount(0);
  await page.getByRole("group", { name: "Content labels" }).getByRole("button", { name: "Strong language" }).click();
  await expect(warning).toContainText("This one is labelled strong language");
  await expect(page.getByRole("button", { name: "Create draft" })).toBeEnabled();
  await noSeriousAxeViolations(page);
});
