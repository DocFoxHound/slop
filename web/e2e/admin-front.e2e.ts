// SPDX-License-Identifier: AGPL-3.0-or-later
// The front page from the admin panel: the Featured carousel, staff shelves (editing, ordering,
// an address already taken) and hiding an automatic shelf. Everything is put back as it was:
// admin.e2e.ts expects no shelves.
import { closePages, newPage, ok, tag } from "./admin-helpers";
import { expect, server, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("Featured: added by address or link, with a line each, reordered, saved, and shown first on the front page", async ({ browser }) => {
  const before = await ok("admin", "GET", "/v1/admin/featured");
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=store");
  const featured = admin.locator("details").filter({ hasText: "Featured on the front page" });
  const add = featured.getByRole("textbox", { name: "App address to feature" });
  const save = featured.getByRole("button", { name: "Save" });
  // Start from nothing.
  for (const f of before) await featured.getByRole("listitem").filter({ hasText: f.title }).getByRole("button", { name: "Remove" }).click();
  if (before.length) await save.click();
  await expect(featured.getByText(/\(0 of 8\)/)).toBeVisible();
  await expect(save).toBeDisabled();

  await add.fill("castle-rock");
  await featured.getByRole("button", { name: "Add", exact: true }).click();
  await featured.getByRole("textbox", { name: "Why castle-rock is featured" }).fill("A tower defence classic, now at 1.0.");
  await add.fill(`${server.url}/app/mias-first`);
  await add.press("Enter");
  await featured.getByRole("textbox", { name: "Why mias-first is featured" }).fill("Mia's very first game.");
  // The same app twice is added once.
  await add.fill("castle-rock");
  await featured.getByRole("button", { name: "Add", exact: true }).click();
  await expect(featured.getByRole("listitem")).toHaveCount(2);
  await featured.getByRole("button", { name: "Move mias-first up" }).click();
  await expect(featured.getByRole("button", { name: "Move mias-first up" })).toBeDisabled();
  await save.click();
  await expect(featured.getByText(/\(2 of 8\)/)).toBeVisible();
  await expect(featured.getByRole("listitem").first().getByRole("link")).toHaveText("Mia's First Game");
  await expect(featured.getByRole("listitem").nth(1).getByRole("link")).toHaveText("Castle Rock");

  // The front page shows them in that order.
  const visitor = await newPage(browser);
  await visitor.goto("/");
  const carousel = visitor.getByRole("region", { name: "Featured" });
  await expect(carousel.getByRole("group", { name: "1 of 2: Mia's First Game" })).toBeVisible();
  await expect(carousel.getByText("Mia's very first game.")).toBeVisible();
  await carousel.getByRole("button", { name: "Next featured app" }).click();
  await expect(carousel.getByRole("group", { name: "2 of 2: Castle Rock" })).toBeVisible();
  await expect(carousel.getByText("A tower defence classic, now at 1.0.")).toBeVisible();

  // An address that isn't an app is refused, and nothing changes.
  await add.fill("no-such-app");
  await featured.getByRole("button", { name: "Add", exact: true }).click();
  await save.click();
  await expect(featured.getByRole("alert")).toHaveText("no public app at no-such-app");
  await featured.getByRole("listitem").filter({ hasText: "no-such-app" }).getByRole("button", { name: "Remove" }).click();
  await save.click();
  await expect(featured.getByRole("alert")).toHaveCount(0);
  await expect(featured.getByText(/\(2 of 8\)/)).toBeVisible();

  // Back as it was.
  await ok("admin", "PUT", "/v1/admin/featured", { apps: before.map((f: { slug: string; blurb: string }) => ({ slug: f.slug, blurb: f.blurb })) });
  await visitor.reload();
  if (before.length === 0) await expect(visitor.getByRole("region", { name: "Featured" })).toHaveCount(0);
});

test("shelves: edited, taken off the front page, their apps and the shelves reordered; an address can't be used twice", async ({ browser }) => {
  const t = tag();
  const one = { slug: `adm-one-${t}`, title: `Tower games ${t}` };
  const two = { slug: `adm-two-${t}`, title: `Night games ${t}` };
  for (const s of [one, two])
    await ok("admin", "POST", "/v1/admin/shelves", { ...s, blurb: "", on_front: true, apps: ["castle-rock", "neon-drift"] });
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=store");

  // Edit the first: a new title and line, its apps the other way round, off the front page.
  // Found by its page's address (inside the closed <details>), which stays as its title changes.
  const shelf = admin.locator(`details.admin-shelf:has(a[href="/shelf/${one.slug}"])`);
  await shelf.locator("summary").click();
  await expect(shelf.getByRole("link", { name: `/shelf/${one.slug}` })).toBeVisible();
  const save = shelf.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await shelf.getByRole("textbox", { name: "Title" }).fill(`Towers and castles ${t}`);
  await shelf.getByRole("textbox", { name: "What ties them together" }).fill("Build up, hold out.");
  await shelf.getByRole("button", { name: "Move neon-drift up" }).click();
  await shelf.getByRole("checkbox", { name: "Show on the front page" }).uncheck();
  await save.click();
  await expect(shelf.locator("summary")).toContainText(`Towers and castles ${t} 2 apps · off the front page`);
  const list = await ok("admin", "GET", "/v1/admin/shelves");
  const saved = list.shelves.find((s: { slug: string }) => s.slug === one.slug);
  expect(saved.apps.map((a: { slug: string }) => a.slug)).toEqual(["neon-drift", "castle-rock"]);
  expect(saved.blurb).toBe("Build up, hold out.");

  // The shelves' order.
  const order = () => ok("admin", "GET", "/v1/admin/shelves").then((l) => l.shelves.map((s: { slug: string }) => s.slug));
  expect((await order()).slice(-2)).toEqual([one.slug, two.slug]);
  await admin.getByRole("button", { name: `Move ${two.title} up` }).click();
  await expect.poll(async () => (await order()).slice(-2)).toEqual([two.slug, one.slug]);

  // Off the front page, but still on its own page.
  const visitor = await newPage(browser);
  await visitor.goto("/");
  await expect(visitor.getByRole("region", { name: two.title })).toBeVisible();
  await expect(visitor.getByRole("region", { name: `Towers and castles ${t}` })).toHaveCount(0);
  await visitor.goto(`/shelf/${one.slug}`);
  await expect(visitor.getByText(`Towers and castles ${t}`).first()).toBeVisible();

  // A new shelf can't take an address that's in use.
  await admin.getByRole("button", { name: "New shelf" }).click();
  const fresh = admin.locator("details.admin-shelf").filter({ has: admin.getByRole("button", { name: "Make the shelf" }) });
  await fresh.getByRole("textbox", { name: "Address", exact: true }).fill(one.slug);
  await fresh.getByRole("textbox", { name: "Title", exact: true }).fill("A copy");
  await fresh.getByRole("button", { name: "Make the shelf" }).click();
  await expect(fresh.getByRole("alert")).toHaveText("there's already a shelf at that address");
  await fresh.getByRole("button", { name: "Cancel" }).click();

  // Both deleted: no shelves, as the other tests expect.
  for (const s of [one, two]) await ok("admin", "DELETE", `/v1/admin/shelves/${s.slug}`);
  await admin.reload();
  await expect(admin.getByText(`Towers and castles ${t}`)).toHaveCount(0);
});

test("an automatic shelf hidden from the front page, and shown again", async ({ browser }) => {
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=store");
  const auto = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "Automatic shelves" }) });
  const categories = auto.getByRole("checkbox", { name: /Browse by category/ });
  await expect(categories).toBeChecked();

  const visitor = await newPage(browser);
  await visitor.goto("/");
  await expect(visitor.getByRole("heading", { name: "Browse by category" })).toBeVisible();

  // It changes once SLAPorSLOP has it (the box follows the server).
  await categories.click();
  await expect(categories).not.toBeChecked();
  await expect(auto.getByText("HIDDEN", { exact: true })).toBeVisible();
  await visitor.reload();
  await expect(visitor.getByRole("heading", { name: "Popular this month" })).toBeVisible();
  await expect(visitor.getByRole("heading", { name: "Browse by category" })).toHaveCount(0);

  await categories.click();
  await expect(categories).toBeChecked();
  await expect(auto.getByText("HIDDEN", { exact: true })).toHaveCount(0);
  await visitor.reload();
  await expect(visitor.getByRole("heading", { name: "Browse by category" })).toBeVisible();
});
