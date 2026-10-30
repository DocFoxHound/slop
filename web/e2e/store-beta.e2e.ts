// SPDX-License-Identifier: AGPL-3.0-or-later
// A Beta app's store page (owner decision 2026-10-25: Betas are playable, so they're rateable,
// and they take donations): Rate it at the top for a player who played it, otherwise why not with
// the step to take right there; "Rated in Beta" on the votes; the Donate button always there next
// to Get it, saying why donations are off, with the maker's fix in its panel. Also the maker's Get
// paid section on the Edit store page. Remy's Beta game is this file's own; it leaves the store
// again at the end.
import AxeBuilder from "@axe-core/playwright";
import type { Page, Route } from "@playwright/test";

import { asDesktop } from "./desktop-helpers";
import { donateButton, donatePanel } from "./donate-helpers";
import { expect, signIn, test } from "./fixtures";
import { call, delist, listedApp, path, played, tryCall } from "./store-helpers";

const beta = "beta-buffalo";
const title = "Beta Buffalo";

test.beforeAll(async () => {
  await listedApp(beta, title, { who: "pendingCreative", stage: "beta", tags: ["strategy"] });
  await played("player", beta);
});

test.afterAll(async () => {
  await tryCall("player", "DELETE", `/v1/apps/${beta}/review`);
  await tryCall("player", "POST", `/v1/apps/${beta}/vote`, { vote: null });
  await delist(beta);
});

const head = (page: Page) => page.locator(".rating-head");
const ratings = (page: Page) => page.getByRole("region", { name: "Ratings" });
const donate = (page: Page) => donatePanel(page);
/** Opens the Donate button's panel, beside Get it's controls. */
async function openDonate(page: Page) {
  await donateButton(page.locator(".install-box")).click();
  await expect(donate(page)).toBeVisible();
}

async function noSeriousAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

/** The maker's real donation settings, with some fields changed. */
const settingsAs = (change: Record<string, unknown>) => async (route: Route) => {
  if (route.request().method() !== "GET") return route.fallback();
  const res = await route.fetch();
  return route.fulfill({ response: res, json: { ...(await res.json()), ...change } });
};

test("a player who played the Beta rates it from the top of the page, and it says Rated in Beta", async ({ page }) => {
  await tryCall("player", "POST", `/v1/apps/${beta}/vote`, { vote: null });
  await signIn(page, "player");
  await page.goto(`/app/${beta}`);
  await expect(page.getByRole("heading", { name: title, level: 1 })).toBeVisible();
  const rate = head(page).getByRole("button", { name: "Rate it" });
  await expect(rate).toBeVisible();
  await expect(head(page).getByText("It's in Beta: your rating is marked Rated in Beta.")).toBeVisible();
  await noSeriousAxeViolations(page);

  await rate.click();
  await expect(ratings(page)).toBeFocused();
  await ratings(page).getByRole("button", { name: "👍 Slaps" }).click();
  await expect(head(page).getByRole("button", { name: "Your rating: 👍 Slaps" })).toBeVisible();
  await expect(ratings(page).getByText("This vote was given while it was in Beta.")).toBeVisible();

  // A written review shows with its vote, marked as given in Beta.
  await ratings(page).getByRole("button", { name: "Add a written review" }).click();
  await ratings(page).getByLabel("Your review").fill("Rough around the edges, but the herds are great.");
  await ratings(page).getByRole("button", { name: "Save review" }).click();
  const review = page.locator(".review-list li").filter({ hasText: "Rough around the edges" });
  await expect(review.getByText("RATED IN BETA")).toBeVisible();
});

test("why others can't rate it yet, with the step to take right there", async ({ browser }) => {
  // Signed out: Sign in.
  const visitor = await (await browser.newContext()).newPage();
  await visitor.goto(`/app/${beta}`);
  await expect(head(visitor).getByText(`Played ${title}? Sign in to rate it.`)).toBeVisible();
  await expect(head(visitor).getByRole("link", { name: "Sign in to rate" })).toHaveAttribute(
    "href",
    `/signin?return_to=${encodeURIComponent(`/app/${beta}`)}`,
  );
  await noSeriousAxeViolations(visitor);

  // Signed in on the website, never played it: get the SLAPorSLOP app.
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${beta}`);
  await expect(head(teen).getByText("Play it from the SLAPorSLOP app to rate it.")).toBeVisible();
  await expect(head(teen).getByRole("link", { name: "⬇ Get the SLAPorSLOP app" })).toHaveAttribute("href", "/download");
  await expect(head(teen).getByRole("button", { name: "Rate it" })).toHaveCount(0);

  // In the SLAPorSLOP app, the Install button is right there.
  const app = await browser.newPage();
  await asDesktop(app, { state: "none" });
  await signIn(app, "teen");
  await app.goto(`/app/${beta}`);
  await expect(head(app).getByText("Play it from the SLAPorSLOP app to rate it.")).toBeVisible();
  await expect(head(app).getByRole("button", { name: "Install" })).toBeVisible();
  await expect(head(app).getByRole("link", { name: "⬇ Get the SLAPorSLOP app" })).toHaveCount(0);

  // The maker.
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${beta}`);
  await expect(head(maker).getByText("Makers can't rate their own app.")).toBeVisible();
  await expect(head(maker).getByRole("button", { name: "Rate it" })).toHaveCount(0);
});

test("Donate is always there on a Beta page, next to Get it: why donations are off, and the maker's fix", async ({ browser }) => {
  // Everyone sees why: quiet, with "Donations off" beside it and the reason on hover and focus.
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${beta}`);
  const button = donateButton(visitor.locator(".install-box"));
  await expect(button).toHaveAccessibleName("Donate");
  await expect(button).toHaveAccessibleDescription("Donations are off. The maker hasn't turned on donations.");
  await expect(visitor.locator(".install-box").getByText("Donations off")).toBeVisible();
  await openDonate(visitor);
  const box = donate(visitor);
  await expect(box.getByRole("heading", { name: "♥ Donate" })).toBeVisible();
  await expect(box.getByText("Donations are off.")).toBeVisible();
  await expect(box).toContainText("The maker hasn't turned on donations.");
  await expect(box.getByRole("button")).toHaveAccessibleName("Close");
  // One place: no separate Donate box under Get it any more.
  await expect(visitor.locator(".side-col > .donate-box")).toHaveCount(0);

  // The maker without a Stripe account: Stripe's sign-up, right there, coming back here.
  const maker = await (await browser.newContext()).newPage();
  await signIn(maker, "pendingCreative");
  await maker.route(path(`/v1/apps/${beta}/donations/settings`), settingsAs({ account_status: "none", donations_on: false }));
  let onboard: Record<string, unknown> | undefined;
  await maker.route(path("/v1/me/donation-account"), (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    onboard = route.request().postDataJSON();
    return route.fulfill({ json: { url: null, status: "pending" } });
  });
  await maker.goto(`/app/${beta}`);
  await openDonate(maker);
  const fix = donate(maker);
  await expect(fix.getByText("Donations are off.")).toBeVisible();
  const enable = fix.getByRole("button", { name: "Enable donations with Stripe" });
  await expect(enable).toBeDisabled();
  await fix.getByRole("checkbox", { name: /donations may be taxable income for me/ }).check();
  const adult = fix.getByRole("checkbox", { name: "I'm 18 or older." });
  if (await adult.count()) await adult.check();
  await enable.click();
  await expect.poll(() => onboard?.return_to).toBe(`/app/${beta}`);
  expect(onboard?.tax_notice_ack).toBe(true);
  await noSeriousAxeViolations(maker);

  // Stripe ready, the box switched off: one button turns it on.
  const ready = await browser.newPage();
  await signIn(ready, "pendingCreative");
  let saved: Record<string, unknown> | undefined;
  await ready.route(path(`/v1/apps/${beta}/donations/settings`), async (route) => {
    if (route.request().method() === "PUT") {
      saved = route.request().postDataJSON();
      return route.fulfill({ status: 204 });
    }
    return settingsAs({ account_status: "active", donations_on: false, donation_suggested_minor: 700 })(route);
  });
  await ready.goto(`/app/${beta}`);
  await openDonate(ready);
  await expect(donate(ready).getByText(/Your Stripe account is ready\. Donors are offered amounts around \$7\.00/)).toBeVisible();
  await donate(ready).getByRole("button", { name: "Turn on donations" }).click();
  await expect.poll(() => saved?.donations_on).toBe(true);
  expect(saved?.donation_suggested_minor).toBe(700);
  // Saving reloads the settings: let that answer (and any other still on its way) finish before
  // the windows close, or its handler reads a response that's already gone and fails the test.
  await ready.unrouteAll({ behavior: "wait" });
  await maker.unrouteAll({ behavior: "wait" });
});

test("the Edit store page has a Get paid section beside the pictures, with every step to set it up", async ({ page }) => {
  await signIn(page, "pendingCreative");
  const section = page.getByRole("region", { name: "Get paid: donations" });
  const settings = path(`/v1/apps/${beta}/donations/settings`);

  // Off on this server: it says so.
  await page.route(settings, settingsAs({ payments: "off" }));
  await page.goto(`/publish/${beta}`);
  await expect(page.getByRole("heading", { name: "Basics" })).toBeVisible();
  await expect(section.getByText("Payments aren't switched on on this server yet.")).toBeVisible();
  // Beside the pictures, after the store page form (on a wide screen).
  const side = await section.boundingBox();
  const pictures = await page.getByRole("heading", { name: "Pictures & videos" }).boundingBox();
  expect(side!.x).toBeGreaterThan(pictures!.x);

  // Not set up: the tax notice, then Stripe's sign-up, coming back to this page.
  await page.unroute(settings);
  await page.route(settings, settingsAs({ payments: "stripe", account_status: "none", donations_on: false }));
  let onboard: Record<string, unknown> | undefined;
  await page.route(path("/v1/me/donation-account"), (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    onboard = route.request().postDataJSON();
    return route.fulfill({ json: { url: null, status: "pending" } });
  });
  await page.reload();
  const enable = section.getByRole("button", { name: "Enable donations with Stripe" });
  await expect(enable).toBeDisabled();
  await section.getByRole("checkbox", { name: /donations may be taxable income for me/ }).check();
  const adult = section.getByRole("checkbox", { name: "I'm 18 or older." });
  if (await adult.count()) await adult.check();
  await enable.click();
  await expect.poll(() => onboard?.return_to).toBe(`/publish/${beta}#donations`);

  // Waiting on Stripe: its status, and the way back to it.
  await page.unroute(settings);
  await page.route(settings, settingsAs({ payments: "stripe", account_status: "pending", donations_on: false }));
  await page.reload();
  await expect(section.getByText("Stripe is still checking your account. Finish Stripe's sign-up, then come back.")).toBeVisible();
  await expect(section.getByRole("button", { name: "Continue on Stripe" })).toBeVisible();
  await expect(section.getByRole("button", { name: "I've finished on Stripe" })).toBeVisible();

  // Ready and on: says so, with the settings.
  await page.unroute(settings);
  await page.route(settings, settingsAs({ payments: "stripe", account_status: "active", donations_on: true, donation_suggested_minor: 500, donation_currency: "usd" }));
  await page.reload();
  await expect(section.getByText(/Donations on\. Your store page has a Donate button \(next to Play\) offering amounts around \$5\.00/)).toBeVisible();
  await expect(section.getByRole("checkbox", { name: "Show the Donate button on the store page" })).toBeChecked();
  await noSeriousAxeViolations(page);
});

test("a new app's form says where getting paid is set up", async ({ page }) => {
  await signIn(page, "pendingCreative");
  await page.goto("/publish/new");
  await expect(page.getByText(/Getting paid: players can donate to you once it's in Beta or released/)).toBeVisible();
  // Not for Private, which takes no donations.
  await page.getByRole("radio", { name: /Private/ }).check();
  await expect(page.getByText(/Getting paid:/)).toHaveCount(0);
});

test("the API: Beta votes count, and anyone signed out can read the donate box", async () => {
  const r = await call<{ open: boolean; stage: string }>(null, "GET", `/v1/apps/${beta}/ratings`);
  expect([r.open, r.stage]).toEqual([true, "beta"]);
  const info = await call<{ on: boolean; off_reason: string | null; cant: string }>(null, "GET", `/v1/apps/${beta}/donate`);
  expect(info.cant).toBe("Sign in to donate.");
  expect(info.on).toBe(false);
});
