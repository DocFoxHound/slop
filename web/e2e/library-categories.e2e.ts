// SPDX-License-Identifier: AGPL-3.0-or-later
// The library's Show filter by store category (pages/Library.tsx, libraryModel.ts
// `categoryFilters`): after the kinds, a Categories group with each store category, in the store's
// order, that an app in the library is in (one of its tags is the category's); choosing one shows
// only those apps. On the website and in the SLAPorSLOP app, where an installed app the store no
// longer lists is in none. Relay Rig (a dedicated server: the Servers category, migration 0093)
// and Quarry Nap (a cozy puzzle game) are this file's own: they're added to Pat's library and
// taken out again, and leave the store at the end.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { asDesktop } from "./desktop-helpers";
import { expect, signIn, test } from "./fixtures";
import { call, delist, listedApp, tryCall } from "./store-helpers";

const relay = "relay-rig";
const quarry = "quarry-nap";

test.beforeAll(async () => {
  await listedApp(relay, "Relay Rig", { tags: ["dedicated-server", "multiplayer"], summary: "A dedicated server for co-op nights." });
  await listedApp(quarry, "Quarry Nap", { tags: ["puzzle", "cozy"], summary: "Dig slowly, nap often." });
  for (const slug of [relay, quarry]) await call("player", "PUT", `/v1/me/library/${slug}`);
});

test.afterAll(async () => {
  for (const slug of [relay, quarry]) await tryCall("player", "DELETE", `/v1/me/library/${slug}`);
  await delist(relay, quarry);
});

type Category = { slug: string; name: string; tags: Array<{ slug: string }> };
type Tagged = { slug: string; title: string; tags: string[] };

const side = (page: Page) => page.getByRole("navigation", { name: "Your apps" });
const show = (page: Page) => side(page).getByRole("combobox", { name: "Show" });
const categoriesGroup = (page: Page) => show(page).locator("optgroup[label='Categories']");
const rows = (page: Page) => side(page).locator(".lib-row-name");

/** The store's categories, in its order, that one of Pat's library's apps is in (with `extra`). */
async function patsCategories(extra: Tagged[] = []): Promise<{ categories: Category[]; names: string[]; apps: Tagged[] }> {
  const categories = await call<Category[]>(null, "GET", "/v1/categories");
  const priv = await call<{ mine: Tagged[]; shared: Tagged[] }>("player", "GET", "/v1/me/private");
  const apps = [
    ...(await call<Tagged[]>("player", "GET", "/v1/me/library")),
    ...(await call<Tagged[]>("player", "GET", "/v1/me/interested")),
    ...priv.mine,
    ...priv.shared,
    ...extra,
  ];
  const inIt = (c: Category, a: Tagged) => a.tags.some((t) => c.tags.some((ct) => ct.slug === t));
  const names = categories.filter((c) => apps.some((a) => inIt(c, a))).map((c) => c.name);
  return { categories, names, apps: apps.filter((a, i) => apps.findIndex((b) => b.slug === a.slug) === i) };
}

async function axe(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

test("on the website: Show lists the categories the library has apps in, in the store's order, and filters by them", async ({ page }) => {
  const { categories, names, apps } = await patsCategories();
  for (const name of ["Games", "Servers", "Play style", "Vibe"]) expect(names).toContain(name);

  await signIn(page, "player");
  await page.goto("/library");
  await expect(page.getByRole("heading", { name: "Library", level: 1 })).toBeVisible();
  await expect(categoriesGroup(page).locator("option")).toHaveText(names);
  // The kinds come first; the categories are the select's last part.
  await expect(show(page).locator(":scope > option").first()).toHaveText("All");
  await expect(show(page).locator(":scope > *").last()).toHaveAttribute("label", "Categories");

  await show(page).selectOption({ label: "Servers" });
  await expect(rows(page)).toHaveText(["Relay Rig"]);
  // The group's count says the rest are hidden, Steam's "(1/23)".
  await expect(side(page).getByRole("heading", { name: /In your library/ })).toContainText(/\(1\/\d+\)/);
  await axe(page);

  // Another category: just its apps, from every group.
  const games = categories.find((c) => c.slug === "games")!;
  const inGames = apps.filter((a) => a.tags.some((t) => games.tags.some((g) => g.slug === t)));
  await show(page).selectOption({ label: "Games" });
  await expect(rows(page)).toHaveCount(inGames.length);
  await expect(side(page).getByRole("link", { name: /^Quarry Nap/ })).toBeVisible();
  await expect(side(page).getByRole("link", { name: /^Relay Rig/ })).toHaveCount(0);

  // A search narrows it further; All shows everything again.
  await show(page).selectOption({ label: "Servers" });
  await side(page).getByRole("searchbox", { name: "Search your library" }).fill("nothing is called this");
  await expect(side(page).getByText("No apps match.")).toBeVisible();
  await side(page).getByRole("searchbox", { name: "Search your library" }).fill("");
  await show(page).selectOption({ label: "All" });
  await expect(side(page).getByRole("link", { name: /^Relay Rig/ })).toBeVisible();
  await expect(rows(page)).toHaveCount(apps.length);
});

test("in the SLAPorSLOP app: installed apps are in their categories too, and one the store no longer lists in none", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await asDesktop(
    page,
    { state: "none" },
    {
      installed: [
        { slug: relay, title: "Relay Rig", maker: "Bo Builder" },
        { slug: "long-gone-server", title: "Long Gone Server", maker: "Someone" },
      ],
    },
  );
  await page.route(
    (u) => u.pathname === "/v1/library/updates",
    (route) => route.fulfill({ json: [] }),
  );
  const { names } = await patsCategories();
  await signIn(page, "player");
  await page.goto("/library");
  await expect(side(page).getByRole("heading", { name: /Installed/ })).toContainText("(2)");
  // The app's own filters, then the categories.
  await expect(show(page).locator(":scope > option")).toContainText(["All", "Installed"]);
  await expect(categoriesGroup(page).locator("option")).toHaveText(names);

  await show(page).selectOption({ label: "Servers" });
  await expect(rows(page)).toHaveText(["Relay Rig"]);
  await expect(side(page).getByRole("heading", { name: /Installed/ })).toContainText("(1/2)");
  await expect(side(page).getByRole("link", { name: /^Long Gone Server/ })).toHaveCount(0);
  await axe(page);
});
