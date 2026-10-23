// SPDX-License-Identifier: AGPL-3.0-or-later
// The front page and what staff put on it: the intro, Featured, For you, the shelves (staff's,
// and the automatic ones, which staff can hide), Coming soon & beta, and staff shelves' own
// pages. Featured, shelves and hidden shelves are put back as they were (none) by each test.
import type { Page } from "@playwright/test";
import { expect, ids, signIn, test } from "./fixtures";
import { call, delist, failApi, listedApp, path, tryCall } from "./store-helpers";

// Vera's: a coming-soon game players are Interested in, and one rated 18+.
const soon = "harbour-lights";
const adult = "night-harbour";
const shelfA = "quiet-evenings";
const shelfB = "loud-mornings";

test.beforeAll(async () => {
  await listedApp(soon, "Harbour Lights", { who: "creative", stage: "coming_soon", tags: ["puzzle"] });
  await listedApp(adult, "Night Harbour", { who: "creative", stage: "coming_soon", tags: ["strategy"], min_age: 18 });
  await call("client", "PUT", `/v1/me/interested/${soon}`);
});

test.afterAll(async () => {
  await tryCall("client", "DELETE", `/v1/me/interested/${soon}`);
  await tryCall("admin", "PUT", "/v1/admin/featured", { apps: [] });
  for (const s of [shelfA, shelfB]) await tryCall("admin", "DELETE", `/v1/admin/shelves/${s}`);
  for (const key of ["popular", "upcoming", "glow_ups"]) await tryCall("admin", "POST", `/v1/admin/front-shelves/${key}`, { hidden: false });
  await delist(soon, adult);
});

const section = (page: Page, title: string) =>
  page.locator(".section-h").filter({ has: page.getByRole("heading", { name: title, exact: true }) });

/** The shelf of tiles after a section heading. */
const shelfAfter = (page: Page, title: string) => section(page, title).locator("xpath=following-sibling::*[1]");

test("visitors get the intro and the test-server note; signed-in players get For you instead", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/");
  await expect(visitor.getByRole("heading", { name: /Apps and games made with AI\./, level: 1 })).toBeVisible();
  // This server doesn't scan, so it says a person checks every build instead.
  await expect(visitor.getByText("Reviewed before it's listed.")).toBeVisible();
  await expect(visitor.getByText("Every build is checked by a person.")).toBeVisible();
  await expect(visitor.getByRole("note").filter({ hasText: "Test server." })).toContainText("uploads are not scanned");
  await expect(visitor.getByRole("heading", { name: "For you" })).toHaveCount(0);
  // The categories, each to its page.
  await expect(visitor.locator(".cat-grid").getByRole("link", { name: /Games/ })).toHaveAttribute("href", "/browse/games");
  await expect(section(visitor, "New and barely seen")).toBeVisible();

  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto("/");
  await expect(section(pat, "For you")).toBeVisible();
  await expect(pat.getByRole("heading", { name: /Apps and games made with AI\./ })).toHaveCount(0);
  await expect(pat.getByRole("note").filter({ hasText: "Test server." })).toBeVisible();
});

test("For you says what it's based on, or that it isn't personal yet, and hides when it fails", async ({ browser }) => {
  // Pat has a library and votes.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto("/");
  const forYou = section(pat, "For you");
  await expect(forYou.getByText(/^Based on .+, plus something different every fourth$/)).toBeVisible();
  await expect(forYou.getByRole("link", { name: "Settings" })).toHaveAttribute("href", "/account");
  await expect(shelfAfter(pat, "For you").locator(".tile").first()).toBeVisible();

  // Sue has done nothing yet.
  const sue = await browser.newPage();
  await signIn(sue, "support");
  await sue.goto("/");
  await expect(section(sue, "For you").getByText(/^Popular, well rated and new\. Add apps to your library/)).toBeVisible();

  // When it can't load, the shelf just isn't there.
  await failApi(sue, path("/v1/me/for-you"));
  await sue.reload();
  await expect(section(sue, "Popular this month")).toBeVisible();
  await expect(sue.getByRole("heading", { name: "For you" })).toHaveCount(0);
});

test("staff feature apps; the carousel pages by buttons, dots and arrow keys, and never by itself", async ({ browser }) => {
  await call("admin", "PUT", "/v1/admin/featured", { apps: [] });
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=store");
  const editor = admin.locator("details").filter({ hasText: "Featured on the front page" });
  await expect(editor.getByText("(0 of 8)")).toBeVisible();
  const address = editor.getByRole("textbox", { name: "App address to feature" });
  for (const slug of [ids.mod, "castle-rock", `/app/${ids.listedApp}`]) {
    await address.fill(slug);
    await editor.getByRole("button", { name: "Add", exact: true }).click();
  }
  // A link works as well as an address; it's added once.
  await expect(editor.getByRole("textbox", { name: `Why ${ids.listedApp} is featured` })).toBeVisible();
  await editor.getByRole("button", { name: `Move ${ids.listedApp} up` }).click();
  await editor.getByRole("button", { name: `Move ${ids.listedApp} up` }).click();
  await editor.getByRole("textbox", { name: `Why ${ids.listedApp} is featured` }).fill("Lantern puzzles with real tension.");
  // The mod goes again.
  await editor.locator("li").filter({ hasText: ids.mod }).getByRole("button", { name: "Remove" }).click();
  // An app that doesn't exist is refused, and nothing is saved.
  await address.fill("no-such-app");
  await editor.getByRole("button", { name: "Add", exact: true }).click();
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor.getByRole("alert")).toContainText("no public app at no-such-app");
  await editor.locator("li").filter({ hasText: "no-such-app" }).getByRole("button", { name: "Remove" }).click();
  // A draft can be picked, but doesn't show until it's in the store.
  await address.fill(ids.draftApp);
  await editor.getByRole("button", { name: "Add", exact: true }).click();
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor.getByText("(3 of 8)")).toBeVisible();
  await expect(editor.locator("li").filter({ hasText: "Starship Draft" }).getByText("NOT IN THE STORE: HIDDEN")).toBeVisible();
  await expect(editor.locator("li").first()).toContainText("Lantern Keep");

  const page = await browser.newPage();
  await page.clock.install();
  await page.goto("/");
  const carousel = page.getByRole("region", { name: "Featured" });
  const slide = carousel.getByRole("group");
  await expect(slide).toHaveAttribute("aria-label", "1 of 2: Lantern Keep");
  await expect(carousel.getByText("Lantern puzzles with real tension.")).toBeVisible();
  await expect(carousel.getByText("86% Not Slop")).toBeVisible();
  // It never moves by itself.
  await page.clock.runFor(120_000);
  await expect(slide).toHaveAttribute("aria-label", "1 of 2: Lantern Keep");
  await carousel.getByRole("button", { name: "Next featured app" }).click();
  await expect(slide).toHaveAttribute("aria-label", "2 of 2: Castle Rock");
  await expect(carousel.getByRole("button", { name: "Show Castle Rock" })).toHaveAttribute("aria-current", "true");
  // Wraps around both ways.
  await carousel.getByRole("button", { name: "Next featured app" }).click();
  await expect(slide).toHaveAttribute("aria-label", "1 of 2: Lantern Keep");
  await carousel.getByRole("button", { name: "Previous featured app" }).click();
  await expect(slide).toHaveAttribute("aria-label", "2 of 2: Castle Rock");
  await carousel.getByRole("button", { name: "Show Lantern Keep" }).click();
  await expect(slide).toHaveAttribute("aria-label", "1 of 2: Lantern Keep");
  await carousel.getByRole("button", { name: "Show Lantern Keep" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(slide).toHaveAttribute("aria-label", "2 of 2: Castle Rock");
  await page.keyboard.press("ArrowLeft");
  await expect(slide).toHaveAttribute("aria-label", "1 of 2: Lantern Keep");
  await slide.getByRole("link", { name: "Lantern Keep" }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${ids.listedApp}$`));

  // Staff take everything off: no carousel at all.
  for (let i = 0; i < 3; i++) await editor.locator("li").first().getByRole("button", { name: "Remove" }).click();
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor.getByText("(0 of 8)")).toBeVisible();
  await page.goto("/");
  await expect(section(page, "Popular this month")).toBeVisible();
  await expect(page.getByRole("region", { name: "Featured" })).toHaveCount(0);
});

test("staff shelves show on the front page in staff's order, and each has its own page", async ({ browser }) => {
  await call("admin", "POST", "/v1/admin/shelves", { slug: shelfA, title: "Quiet evenings", blurb: "Calm games for after work.", on_front: true, apps: [ids.listedApp, "castle-rock", adult] });
  await call("admin", "POST", "/v1/admin/shelves", { slug: shelfB, title: "Loud mornings", blurb: "", on_front: true, apps: [adult] });
  try {
    const admin = await browser.newPage();
    await signIn(admin, "admin");
    await admin.goto("/admin?tab=store");
    // Pat is in the full view (US); 18+ apps show for him.
    const page = await browser.newPage();
    await signIn(page, "player");
    await page.goto("/");
    const headings = page.locator(".section-h h2");
    await expect(headings.filter({ hasText: /^(Quiet evenings|Loud mornings)$/ })).toHaveText(["Quiet evenings", "Loud mornings"]);
    await expect(section(page, "Quiet evenings").getByText("Calm games for after work.")).toBeVisible();

    // Staff swap them.
    await admin.getByRole("button", { name: "Move Loud mornings up" }).click();
    await expect(admin.getByRole("button", { name: "Move Loud mornings up" })).toBeDisabled();
    await page.reload();
    await expect(headings.filter({ hasText: /^(Quiet evenings|Loud mornings)$/ })).toHaveText(["Loud mornings", "Quiet evenings"]);

    // Its page: title, blurb, apps in staff's order.
    await section(page, "Quiet evenings").getByRole("link", { name: "See all →" }).click();
    await expect(page).toHaveURL(new RegExp(`/shelf/${shelfA}$`));
    await expect(page.getByRole("heading", { name: "Quiet evenings", level: 1 })).toBeVisible();
    await expect(page.getByText("Calm games for after work.")).toBeVisible();
    await expect(page.locator(".tile .d")).toHaveText([/Light every lantern/, /Castle Rock/, /Night Harbour/]);

    // The under-18 view (Tia in the UK, and anyone signed out) leaves out 18+ apps: a shelf of
    // only those isn't on the front page, and its page says so.
    const teen = await browser.newPage();
    await signIn(teen, "teen");
    await teen.goto(`/shelf/${shelfB}`);
    await expect(teen.getByRole("heading", { name: "Loud mornings", level: 1 })).toBeVisible();
    await expect(teen.getByText("None of these apps are in the store for you right now.")).toBeVisible();
    await teen.goto(`/shelf/${shelfA}`);
    await expect(teen.locator(".tile")).toHaveCount(2);
    const visitor = await browser.newPage();
    await visitor.goto("/");
    await expect(section(visitor, "Quiet evenings")).toBeVisible();
    await expect(section(visitor, "Loud mornings")).toHaveCount(0);

    await page.goto("/shelf/no-such-shelf");
    await expect(page.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
  } finally {
    for (const s of [shelfA, shelfB]) await tryCall("admin", "DELETE", `/v1/admin/shelves/${s}`);
  }
});

test("staff hide an automatic shelf and it leaves the front page until shown again", async ({ browser }) => {
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=store");
  const popular = admin.getByRole("checkbox", { name: /^Popular this month/ });
  await expect(popular).toBeChecked();
  try {
    await popular.click();
    await expect(admin.locator("label").filter({ hasText: "Popular this month" }).getByText("HIDDEN")).toBeVisible();
    await expect(popular).not.toBeChecked();
    const page = await browser.newPage();
    await page.goto("/");
    await expect(section(page, "Most Not Slop right now")).toBeVisible();
    await expect(section(page, "Popular this month")).toHaveCount(0);

    await popular.click();
    await expect(popular).toBeChecked();
    await expect(admin.locator("label").filter({ hasText: "Popular this month" }).getByText("HIDDEN")).toHaveCount(0);
    await page.reload();
    await expect(section(page, "Popular this month")).toBeVisible();
  } finally {
    await tryCall("admin", "POST", "/v1/admin/front-shelves/popular", { hidden: false });
  }
});

test("Coming soon & beta: tabs for signed-in players, and only upcoming, living apps", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/");
  const upcoming = shelfAfter(visitor, "Coming soon & in beta");
  await expect(visitor.getByRole("tablist", { name: "Upcoming apps" })).toHaveCount(0);
  await expect(upcoming.locator(".tile").filter({ hasText: "Harbour Lights" })).toBeVisible();
  await expect(upcoming.getByText("★ 1 interested")).toBeVisible();
  await expect(section(visitor, "Coming soon & in beta").getByRole("link", { name: "See all →" })).toHaveAttribute("href", "/upcoming");

  // Cleo is Interested in a puzzle game: For you starts there.
  const cleo = await browser.newPage();
  await signIn(cleo, "client");
  await cleo.goto("/");
  const tabs = cleo.getByRole("tablist", { name: "Upcoming apps" });
  await expect(tabs.getByRole("tab", { name: "For you" })).toHaveAttribute("aria-selected", "true");
  const forYouRequest = cleo.waitForRequest((r) => new URL(r.url()).pathname === "/v1/upcoming/for-you");
  const anticipated = cleo.waitForRequest((r) => new URL(r.url()).searchParams.get("sort") === "anticipated");
  await tabs.getByRole("tab", { name: "Most anticipated" }).click();
  await anticipated;
  await expect(tabs.getByRole("tab", { name: "Most anticipated" })).toHaveAttribute("aria-selected", "true");
  await expect(cleo.locator(".section-h").filter({ hasText: "Coming soon & in beta" }).locator("xpath=following-sibling::*[2]").locator(".tile").filter({ hasText: "Harbour Lights" })).toBeVisible();
  await tabs.getByRole("tab", { name: "For you" }).click();
  await forYouRequest;
  await expect(tabs.getByRole("tab", { name: "For you" })).toHaveAttribute("aria-selected", "true");

  // Released and dead apps never show here, whatever the ranking says.
  const page = await browser.newPage();
  await patchJson2(page);
  await page.goto("/");
  const shelf = shelfAfter(page, "Coming soon & in beta");
  await expect(shelf.locator(".tile")).toHaveCount(1);
  await expect(shelf.locator(".tile")).toContainText("Harbour Lights");
});

/** The anticipated ranking with a released and a dead app in front. */
async function patchJson2(page: Page) {
  await page.route(
    (u) => u.pathname === "/v1/apps" && u.searchParams.get("sort") === "anticipated",
    async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      const coming = body.apps.filter((a: { slug: string }) => a.slug === soon);
      const released = body.apps.find((a: { stage: string }) => a.stage === "released");
      const dead = { ...coming[0], slug: "dead-dinghy", title: "Dead Dinghy", is_dead: true };
      await route.fulfill({ response: res, json: { ...body, apps: [released, dead, ...coming] } });
    },
  );
}

test("Popular leaves out coming-soon and dead apps; Glow-ups show how far an app came", async ({ page }) => {
  await page.route(
    (u) => u.pathname === "/v1/apps" && u.searchParams.get("sort") === "popular",
    async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      const base = body.apps[0];
      const extra = [
        { ...base, slug: "soon-skiff", title: "Soon Skiff", summary: "Soon Skiff, coming soon.", stage: "coming_soon" },
        { ...base, slug: "dead-dory", title: "Dead Dory", summary: "Dead Dory, abandoned.", is_dead: true },
      ];
      await route.fulfill({ response: res, json: { ...body, apps: [...extra, ...body.apps] } });
    },
  );
  const lantern = (await call<{ apps: Array<{ slug: string }> }>(null, "GET", "/v1/apps?sort=rating&limit=1")).apps[0];
  await page.route(path("/v1/shelves/glow-ups"), (route) => route.fulfill({ json: [{ ...lantern, before: 41, after: 86 }] }));
  await page.goto("/");
  const popular = shelfAfter(page, "Popular this month");
  await expect(popular.locator(".tile").first()).toBeVisible();
  await expect(popular.getByText("Soon Skiff")).toHaveCount(0);
  await expect(popular.getByText("Dead Dory")).toHaveCount(0);
  await expect(section(page, "Glow-ups")).toContainText("Updates turned these from Slop toward Not Slop");
  await expect(shelfAfter(page, "Glow-ups").getByText("41% → 86% Not Slop")).toBeVisible();
});

test("a shelf that fails to load offers Try again; empty shelves say so", async ({ page }) => {
  let fail = true;
  await page.route(
    (u) => u.pathname === "/v1/apps" && u.searchParams.get("sort") === "popular",
    (route) => (fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy. Try again soon." } } }) : route.fallback()),
  );
  await page.goto("/");
  const popular = shelfAfter(page, "Popular this month");
  await expect(popular).toHaveAttribute("role", "alert");
  await expect(popular).toContainText("Slop is busy. Try again soon.");
  fail = false;
  await popular.getByRole("button", { name: "Try again" }).click();
  await expect(shelfAfter(page, "Popular this month").locator(".tile").first()).toBeVisible();

  // A brand-new Slop: nothing anywhere.
  await page.unrouteAll();
  await page.route(path("/v1/apps"), (route) => route.fulfill({ json: { apps: [], next_offset: null } }));
  await page.reload();
  await expect(shelfAfter(page, "Popular this month")).toHaveText("Nothing to play yet.");
  await expect(shelfAfter(page, "Coming soon & in beta")).toHaveText("Nothing upcoming right now.");
  await expect(shelfAfter(page, "Most Not Slop right now")).toHaveText("Not enough recent votes yet.");
  await expect(shelfAfter(page, "Community branches worth installing")).toHaveText("No community builds yet.");
  await expect(shelfAfter(page, "New & updated")).toHaveText("Nothing released yet. Be the first to publish.");
  await expect(shelfAfter(page, "New & updated").getByRole("link", { name: "Be the first to publish" })).toHaveAttribute("href", "/publish");
  // Shelves that are only for when there's something: gone.
  await expect(section(page, "New and barely seen")).toHaveCount(0);
  await expect(section(page, "Fresh slop")).toHaveCount(0);
});
