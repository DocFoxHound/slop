// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own People page: finding it again (People, the account page), where it stands
// (draft, waiting for review, turned down, live, hidden) and what's next, saving from the editor
// back to the page, and the editor's colour picker keeping focus and the scroll position. Vera's
// editor is only changed on screen, never saved; Remy's profile is saved unchanged; the other
// states are the page's answer, changed on its way to the browser.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, ids, signIn, test } from "./fixtures";

/** Where the page and the document are scrolled to. The page scrolls inside <main>, never the document. */
function scrolled(page: Page) {
  return page.evaluate(() => ({
    view: document.querySelector<HTMLElement>("main.view")!.scrollTop,
    document: document.scrollingElement!.scrollTop,
    documentScrolls: document.scrollingElement!.scrollHeight > window.innerHeight,
  }));
}

test("choosing an accent colour keeps focus and the scroll position, with the mouse or the keyboard", async ({ page }) => {
  await signIn(page, "creative");
  await page.goto("/people/me");
  const colours = page.getByRole("group", { name: "Accent colour" });
  await colours.scrollIntoViewIfNeeded();
  await expect(colours).toBeInViewport();
  const before = await scrolled(page);
  expect(before.view).toBeGreaterThan(0);
  expect(before).toMatchObject({ document: 0, documentScrolls: false });
  const swatch = page.locator("label.swatch").filter({ hasText: "Rose" });
  const at = await swatch.boundingBox();

  // The mouse: the colour is chosen and focused, and nothing moves.
  await swatch.click();
  await expect(page.getByRole("radio", { name: "Rose" })).toBeChecked();
  await expect(page.getByRole("radio", { name: "Rose" })).toBeFocused();
  expect(await scrolled(page)).toEqual(before);
  expect(await swatch.boundingBox()).toEqual(at);
  await expect(swatch).toBeInViewport();

  // The keyboard: arrows move along the colours, focus following, still nothing moves.
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("radio", { name: "Ember" })).toBeChecked();
  await expect(page.getByRole("radio", { name: "Ember" })).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("radio", { name: "Rose" })).toBeFocused();
  expect(await scrolled(page)).toEqual(before);
  // Tab goes on to the next control (the section order), as usual.
  await page.keyboard.press("Tab");
  await expect(page.locator("fieldset").filter({ hasText: "Section order" }).locator("button:enabled").first()).toBeFocused();
  expect((await scrolled(page)).document).toBe(0);

  // The page still scrolls with the wheel, inside <main>, and the document never does.
  await swatch.hover();
  await page.mouse.wheel(0, 300);
  await expect.poll(async () => (await scrolled(page)).view).toBeGreaterThan(before.view);
  expect(await scrolled(page)).toMatchObject({ document: 0, documentScrolls: false });

  // The skill chips work the same way.
  const music = page.locator("fieldset").filter({ hasText: "What you do" }).locator("label.chip").filter({ hasText: "Music" });
  await music.scrollIntoViewIfNeeded();
  const chipsBefore = await scrolled(page);
  await music.click();
  await expect(page.getByRole("checkbox", { name: "Music" })).toBeFocused();
  expect(await scrolled(page)).toEqual(chipsBefore);
  // Nothing was saved: Vera's page keeps its colour.
});

test("a creative finds their own page again from People and from their account, with its status", async ({ browser }) => {
  // Remy: his first portfolio waits for review.
  const remy = await browser.newPage();
  await signIn(remy, "pendingCreative");
  await remy.goto("/people");
  const card = remy.getByRole("region", { name: "Your People page" });
  await expect(card).toContainText("Remy");
  await expect(card).toContainText(`/people/${ids.pendingCreativeHandle}`);
  await expect(card).toContainText("WAITING FOR REVIEW");
  await expect(card).toContainText("Your portfolio is waiting for SLAPorSLOP's review.");
  await expect(remy.getByRole("link", { name: "Offer your services" })).toHaveCount(0);
  await card.getByRole("link", { name: "View your page" }).click();
  await expect(remy).toHaveURL(`/people/${ids.pendingCreativeHandle}`);
  await expect(remy.getByRole("region", { name: "Your page's status" })).toContainText("Waiting for SLAPorSLOP's review");
  await remy.goto("/people");
  await remy.getByRole("region", { name: "Your People page" }).getByRole("link", { name: "Edit", exact: true }).click();
  await expect(remy).toHaveURL("/people/me");

  // On the account page, under Profile; old-style links to it work too.
  await remy.goto("/account");
  const panel = remy.getByRole("region", { name: "Your People page" });
  await expect(panel).toContainText("WAITING FOR REVIEW");
  await expect(panel.getByRole("link", { name: "View your page" })).toHaveAttribute("href", `/people/${ids.pendingCreativeHandle}`);
  await expect(panel.getByRole("link", { name: "Edit", exact: true })).toHaveAttribute("href", "/people/me");
  await remy.goto("/account#people-page");
  await expect(remy.getByRole("region", { name: "Your People page" })).toBeInViewport();

  // Vera's is live.
  // (A page of its own context, for the accessibility check.)
  const veraContext = await browser.newContext();
  const vera = await veraContext.newPage();
  await signIn(vera, "creative");
  await vera.goto("/people");
  await expect(vera.getByRole("region", { name: "Your People page" })).toContainText("LIVE");
  await expect(vera.getByRole("region", { name: "Your People page" })).toContainText("Everyone can see it on People.");
  const results = await new AxeBuilder({ page: vera }).include(".people-mine").withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
  expect(results.violations.map((v) => v.id)).toEqual([]);
  await veraContext.close();

  // Someone without a page is offered one, and the account says nothing about People.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto("/people");
  await expect(pat.getByRole("link", { name: "Offer your services" })).toBeVisible();
  await expect(pat.getByRole("region", { name: "Your People page" })).toHaveCount(0);
  await pat.goto("/account");
  await expect(pat.getByRole("heading", { name: "Privacy" })).toBeVisible();
  await expect(pat.getByRole("region", { name: "Your People page" })).toHaveCount(0);

  // Signed out: just the offer.
  const visitor = await browser.newPage();
  await visitor.goto("/people");
  await expect(visitor.getByRole("link", { name: "Offer your services" })).toBeVisible();
  await expect(visitor.getByRole("region", { name: "Your People page" })).toHaveCount(0);
});

test("saving in the editor goes to the page, which says it's saved and where it stands", async ({ page }) => {
  await signIn(page, "pendingCreative");
  await page.goto("/people/me");
  await expect(page.getByRole("heading", { name: "Your People page", level: 1 })).toBeVisible();
  // The editor says where the page is, too.
  await expect(page.getByRole("region", { name: "Your page's status" })).toContainText("Waiting for SLAPorSLOP's review");
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect(page).toHaveURL(`/people/${ids.pendingCreativeHandle}`);
  const saved = page.getByRole("status").filter({ hasText: "Your People page is saved" });
  await expect(saved).toHaveText("✓ Your People page is saved and waiting for SLAPorSLOP's review of your portfolio. Only you can see it until then.");
  // Focus moves to the message, so keyboard and screen reader users start there.
  await expect(saved).toBeFocused();
  const status = page.getByRole("region", { name: "Your page's status" });
  await expect(status).toContainText("You'll get a notification when it's approved or turned down.");
  await expect(status.getByRole("link", { name: "Edit your page" })).toHaveAttribute("href", "/people/me");
  // Back to the editor and forward again: no second message.
  await page.goBack();
  await expect(page).toHaveURL("/people/me");
  await page.goForward();
  await expect(page.getByRole("heading", { name: "Remy Pending", level: 1 })).toBeVisible();
  await expect(page.getByText("Your People page is saved")).toHaveCount(0);
});

const STATES = {
  draft: {
    status: { state: "draft", reason: null, update_waiting: false, waiting_since: null, problem: "no .slop/portfolio.json found", suspended: false },
    title: "Draft: only you can see your page",
    says: ["Next, add your portfolio", "something was wrong: no .slop/portfolio.json found"],
    link: ["Add your portfolio", "/people/me#portfolio"],
    badge: "DRAFT",
  },
  waiting: {
    status: { state: "waiting", reason: null, update_waiting: false, waiting_since: "2026-10-20T10:00:00Z", problem: null, suspended: false },
    title: "Waiting for SLAPorSLOP's review",
    says: ["(sent ", "Only you can see your page until it's approved", "You'll get a notification when it's approved or turned down."],
    badge: "WAITING FOR REVIEW",
  },
  turned_down: {
    status: { state: "turned_down", reason: "Two works are someone else's art.", update_waiting: false, waiting_since: null, problem: null, suspended: false },
    title: "Changes needed: your portfolio wasn't approved",
    says: ["The reviewer said: Two works are someone else's art.", "Check my portfolio now", "the new portfolio goes back for review"],
    link: ["Fix your portfolio", "/people/me#portfolio"],
    badge: "CHANGES NEEDED",
  },
  live_update: {
    status: { state: "live", reason: null, update_waiting: true, waiting_since: "2026-10-20T10:00:00Z", problem: null, suspended: false },
    title: "Live: everyone can see your page",
    says: ["Your new portfolio is waiting for SLAPorSLOP's review; everyone else sees the approved one until then."],
    badge: "LIVE",
  },
  live_turned_down: {
    status: { state: "live", reason: "The video isn't yours.", update_waiting: false, waiting_since: null, problem: null, suspended: false },
    title: "Live: everyone can see your page",
    says: ["Your newest portfolio wasn't approved: The video isn't yours. Everyone else keeps seeing the one approved before."],
    badge: "LIVE",
  },
  hidden: {
    status: { state: "hidden", reason: null, update_waiting: false, waiting_since: null, problem: null, suspended: true },
    title: "Hidden: only you can see your page",
    says: ["Your account is suspended, so your page is hidden."],
    link: ["see it in your account", "/account#decisions"],
    badge: "HIDDEN",
  },
} as const;

for (const [name, s] of Object.entries(STATES)) {
  test(`the creative's own page explains where it stands: ${name}`, async ({ page }) => {
    await signIn(page, "pendingCreative");
    await page.route(`**/v1/people/${ids.pendingCreativeHandle}`, async (route) => {
      const res = await route.fetch();
      await route.fulfill({ response: res, json: { ...(await res.json()), status: s.status.state, mine: s.status } });
    });
    await page.route("**/v1/me/creative", async (route) => {
      const res = await route.fetch();
      await route.fulfill({ response: res, json: { ...(await res.json()), status: s.status } });
    });
    await page.goto(`/people/${ids.pendingCreativeHandle}`);
    const status = page.getByRole("region", { name: "Your page's status" });
    await expect(status).toContainText(s.title);
    await expect(status).toContainText(s.badge);
    for (const text of s.says) await expect(status).toContainText(text);
    if ("link" in s) await expect(status.getByRole("link", { name: s.link[0] })).toHaveAttribute("href", s.link[1]);
    // The People card and the editor say the same.
    await page.goto("/people");
    await expect(page.getByRole("region", { name: "Your People page" })).toContainText(s.badge);
    await page.goto("/people/me");
    await expect(page.getByRole("region", { name: "Your page's status" })).toContainText(s.title);
  });
}

test("a live page that can't be hired yet says to set up payments", async ({ page }) => {
  await signIn(page, "creative");
  await page.route(`**/v1/people/${ids.creativeHandle}`, async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), can_hire: false } });
  });
  await page.goto(`/people/${ids.creativeHandle}`);
  const status = page.getByRole("region", { name: "Your page's status" });
  await expect(status).toContainText("Live: everyone can see your page");
  await expect(status).toContainText("Makers can't hire you yet: set up payments under Get paid.");
  await expect(status.getByRole("link", { name: "Get paid" })).toHaveAttribute("href", "/people/me");
});
