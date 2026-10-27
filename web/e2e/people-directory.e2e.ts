// SPDX-License-Identifier: AGPL-3.0-or-later
// People: the directory of creatives, a creative's page, and "Made with people". Read-only: the
// seed's Vera (vera-pixels, live) and Remy (remy-sounds, portfolio waiting) stay as they are.
import AxeBuilder from "@axe-core/playwright";
import { expect, ids, signIn, test, type Who } from "./fixtures";

test("the directory filters by skill, availability, AI use and kind, and says when nobody matches", async ({ page }) => {
  await page.goto("/people");
  await expect(page.getByRole("heading", { name: "People", level: 1 })).toBeVisible();
  // Everyone, under the day's shelves.
  const vera = page.locator(".people-grid").getByRole("link", { name: /Vera Okafor/ });
  await expect(vera).toBeVisible();
  // Remy's first portfolio isn't approved yet: he isn't listed.
  await expect(page.getByText("Remy Pending")).toHaveCount(0);
  await expect(vera).toContainText("from $120");

  const skills = page.getByRole("group", { name: "Skill" });
  await expect(skills.getByRole("button", { name: "Everyone" })).toHaveAttribute("aria-pressed", "true");
  await skills.getByRole("button", { name: /^Music/ }).click();
  await expect(page).toHaveURL(/[?&]skill=music/);
  await expect(skills.getByRole("button", { name: /^Music/ })).toHaveAttribute("aria-pressed", "true");
  await expect(vera).toBeVisible();

  await page.getByRole("checkbox", { name: "Taking work now" }).click();
  await expect(page.getByRole("checkbox", { name: "Taking work now" })).toBeChecked();
  await expect(page).toHaveURL(/[?&]taking=1/);
  await expect(vera).toBeVisible();

  await page.getByRole("combobox", { name: "AI use" }).selectOption("none");
  await expect(page).toHaveURL(/[?&]ai=none/);
  await expect(vera).toBeVisible();

  await page.getByRole("combobox", { name: "Order" }).selectOption("rating");
  await expect(page).toHaveURL(/[?&]sort=rating/);
  await expect(vera).toBeVisible();

  // Vera works alone: studios only finds nobody.
  await page.getByRole("checkbox", { name: "Studios only" }).click();
  await expect(page).toHaveURL(/[?&]studios=1/);
  await expect(page.getByText("Nobody matches those filters yet.")).toBeVisible();
  await expect(vera).toHaveCount(0);
  await page.getByRole("checkbox", { name: "Studios only" }).click();
  await expect(vera).toBeVisible();

  // Back to everyone; the filters stay in the address, so a reload keeps them.
  await page.reload();
  await expect(page.getByRole("checkbox", { name: "Taking work now" })).toBeChecked();
  await skills.getByRole("button", { name: "Everyone" }).click();
  await expect(page).not.toHaveURL(/skill=/);
  await vera.click();
  await expect(page).toHaveURL(`/people/${ids.creativeHandle}`);
  await expect(page.getByRole("heading", { name: "Vera Okafor", level: 1 })).toBeVisible();
});

test("the AI-use filter is neutral: each way of working on its own, in the address", async ({ page }) => {
  await page.goto("/people");
  const vera = page.locator(".people-grid").getByRole("link", { name: /Vera Okafor/ });
  await expect(vera).toBeVisible();
  const ai = page.getByRole("combobox", { name: "AI use" });
  await expect(ai.locator("option")).toHaveText(["Any", "No AI", "AI-assisted", "Mostly AI"]);
  // Vera makes everything by hand: she's under No AI only, not under the others.
  await ai.selectOption("assisted");
  await expect(page).toHaveURL(/[?&]ai=assisted/);
  await expect(vera).toHaveCount(0);
  await ai.selectOption("heavy");
  await expect(page).toHaveURL(/[?&]ai=heavy/);
  await expect(vera).toHaveCount(0);
  await ai.selectOption("none");
  await expect(page).toHaveURL(/[?&]ai=none/);
  await expect(vera).toBeVisible();
  await page.reload();
  await expect(ai).toHaveValue("none");
  await ai.selectOption("");
  await expect(page).not.toHaveURL(/ai=/);
});

test("AI-use badges: their own neutral colours, readable, with words and not colour alone", async ({ page }) => {
  // Vera's card three times over, one for each way of working.
  await page.route((url) => url.pathname === "/v1/people", async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    const vera = body.people.find((c: { handle: string }) => c.handle === ids.creativeHandle);
    const people = (["none", "assisted", "heavy"] as const).map((ai_use, i) => ({ ...vera, handle: `${vera.handle}-${i}`, name: `Vera ${ai_use}`, ai_use }));
    await route.fulfill({ response: res, json: { ...body, people } });
  });
  await page.goto("/people");
  const badges = page.locator(".people-grid .person-card .badge");
  await expect(badges).toHaveText([/NO AI/, /AI-ASSISTED/, /MOSTLY AI-GENERATED/]);
  const looks = await badges.evaluateAll((els) => els.map((e) => [e.className, getComputedStyle(e).color]));
  expect(looks.map(([cls]) => cls)).toEqual(["badge ai-none", "badge ai-assisted", "badge ai-heavy"]);
  // Blue, teal and green: three different colours, none of them the warning or error ones.
  expect(looks.map(([, color]) => color)).toEqual(["rgb(124, 184, 251)", "rgb(45, 212, 191)", "rgb(74, 222, 128)"]);
  // Everyone is under the day's shelves: in view, so the contrast can be measured.
  await badges.first().scrollIntoViewIfNeeded();
  const results = await new AxeBuilder({ page }).include(".people-grid .person-card .badge").withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  expect(results.passes.map((v) => v.id)).toContain("color-contrast");
});

test("the skills and the search are the page's header; the day's shelves show until you filter", async ({ page }) => {
  await page.goto("/people");
  const header = page.getByRole("navigation", { name: "People" });
  await expect(header.getByRole("group", { name: "Skill" }).getByRole("button", { name: "Everyone" })).toHaveAttribute("aria-pressed", "true");
  await expect(header.getByRole("searchbox", { name: "Search people" })).toBeVisible();
  // Vera is live, set up for payments and taking work; she makes pixel art and music.
  const taking = page.getByRole("region", { name: "Taking work now" });
  await expect(taking.getByRole("link", { name: /Vera Okafor/ })).toBeVisible();
  await expect(page.getByRole("region", { name: /^Spotlight: / })).toBeVisible();
  await expect(page.getByRole("region", { name: "Everyone" }).getByRole("link", { name: /Vera Okafor/ })).toBeVisible();
  await expect(page.getByText("How hiring works")).toHaveCount(0);
  const results = await new AxeBuilder({ page }).include(".people-nav").include(".people-shelf-section").withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);

  // See all: the directory filtered to match, without the shelves.
  await taking.getByRole("link", { name: "See all →" }).click();
  await expect(page).toHaveURL(/\/people\?taking=1$/);
  await expect(page.getByRole("checkbox", { name: "Taking work now" })).toBeChecked();
  await expect(page.getByRole("region", { name: "Taking work now" })).toHaveCount(0);
  await header.getByRole("button", { name: /^Music/ }).click();
  await expect(page.getByRole("heading", { name: "Music", level: 2 })).toBeVisible();
  await expect(page.locator(".people-grid").getByRole("link", { name: /Vera Okafor/ })).toBeVisible();
});

test("someone without a page is told what a People page is for, with the way to make one", async ({ page }) => {
  await page.goto("/people");
  const offer = page.locator(".people-offer");
  await expect(offer).toContainText("A People page is your shop window");
  await expect(offer).toContainText("portfolio");
  await expect(offer.getByRole("link", { name: "Make your People page" })).toHaveAttribute("href", "/people/me");
});

test("a creative's page shows services, portfolio, reviews and AI use; signed out, hiring asks to sign in", async ({ page }) => {
  await page.goto(`/people/${ids.creativeHandle}`);
  await expect(page.getByRole("heading", { name: "Vera Okafor", level: 1 })).toBeVisible();
  await expect(page.getByText("Pixel art and tiny animations")).toBeVisible();
  await expect(page.getByText("Offers services as a business")).toBeVisible();

  // Services, each with its own Hire button.
  await expect(page.getByRole("heading", { name: "Services" })).toBeVisible();
  const service = page.locator(".service").filter({ hasText: "Sprite sheet" });
  await expect(service).toContainText("from $120");
  await expect(service).toContainText("about 7 days");

  // The portfolio: a picture opens larger, and closes.
  await page.getByRole("button", { name: "View larger: Knight" }).click();
  const lightbox = page.getByRole("dialog", { name: "Knight" });
  await expect(lightbox).toBeVisible();
  await lightbox.getByRole("button", { name: "Close" }).click();
  await expect(lightbox).toBeHidden();

  // Reviews from verified clients (Cleo's, under dispute, still shows), and the AI box.
  await expect(page.getByRole("heading", { name: "Reviews from clients" })).toBeVisible();
  const review = page.locator(`#review-${ids.completedJob}`);
  await expect(review).toContainText("Cleo Client");
  await expect(review).toContainText("VERIFIED CLIENT");
  await expect(review).toContainText("Make me a second sign for free");
  await expect(page.getByText("No AI: made entirely by hand.").or(page.getByText(/NO AI/)).first()).toBeVisible();

  // Only Vera answers her reviews: visitors can report them, not reply or dispute.
  await expect(review.getByRole("button", { name: "Reply" })).toHaveCount(0);
  await expect(review.getByRole("button", { name: "Dispute this review" })).toHaveCount(0);

  // Signed out, hiring asks you to sign in and comes back here.
  await service.getByRole("button", { name: "Hire for this" }).click();
  await expect(page.getByText("Sign in to hire Vera Okafor.")).toBeVisible();
  await expect(page.getByRole("link", { name: "Sign in", exact: true }).last()).toHaveAttribute(
    "href",
    `/signin?return_to=${encodeURIComponent(`/people/${ids.creativeHandle}`)}`,
  );

  // Anyone can report the profile or a review: the form says what's being reported.
  await page.locator(".person-head-side").getByRole("button", { name: "⚑ Report" }).click();
  const dialog = page.getByRole("dialog", { name: "Report this profile" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await review.getByRole("button", { name: "⚑ Report" }).click();
  await expect(page.getByRole("dialog", { name: "Report this review" })).toBeVisible();
});

test("a page waiting for its first portfolio shows only to its creative", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto(`/people/${ids.pendingCreativeHandle}`);
  await expect(visitor.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();

  const remy = await browser.newPage();
  await signIn(remy, "pendingCreative");
  await remy.goto(`/people/${ids.pendingCreativeHandle}`);
  await expect(remy.getByRole("heading", { name: "Remy Pending", level: 1 })).toBeVisible();
  const status = remy.getByRole("region", { name: "Your page's status" });
  await expect(status).toContainText("This is your page.");
  await expect(status).toContainText("Waiting for SLAPorSLOP's review");
  await expect(status).toContainText("Only you can see your page until it's approved");
  await expect(remy.getByRole("link", { name: "Edit your page" })).toHaveAttribute("href", "/people/me");
  // Your own page has no Hire or Report buttons, and can't be hired yet.
  await expect(remy.getByRole("button", { name: /^Hire/ })).toHaveCount(0);
  await expect(remy.locator(".person-head-side").getByRole("button", { name: "⚑ Report" })).toHaveCount(0);
});

test("a hidden page tells its creative why; a closed account's page takes no work", async ({ browser }) => {
  // As if staff had hidden Vera's page, and (for visitors) as if she had closed her account.
  const vera = await browser.newPage();
  await signIn(vera, "creative");
  await vera.route(`**/v1/people/${ids.creativeHandle}`, async (route) => {
    const res = await route.fetch();
    const page = await res.json();
    const mine = { ...page.mine, state: "hidden", reason: "a link to a gold seller" };
    await route.fulfill({ response: res, json: { ...page, status: "hidden", hidden_reason: "a link to a gold seller", mine } });
  });
  await vera.goto(`/people/${ids.creativeHandle}`);
  const status = vera.getByRole("region", { name: "Your page's status" });
  await expect(status).toContainText("Hidden: only you can see your page");
  await expect(status).toContainText("SLAPorSLOP hid your page: a link to a gold seller.");
  await expect(status.getByRole("link", { name: "see it in your account" })).toHaveAttribute("href", "/account#decisions");
  // Her own reviews have Reply and, if not yet disputed, Dispute; no Report.
  const review = vera.locator(`#review-${ids.completedJob}`);
  await expect(review.getByRole("button", { name: "Reply" })).toBeVisible();
  await expect(review.getByText("Dispute: waiting for a mediator")).toBeVisible();
  await expect(review.getByRole("button", { name: "⚑ Report" })).toHaveCount(0);

  const visitor = await browser.newPage();
  await visitor.route(`**/v1/people/${ids.creativeHandle}`, async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), closed: true, can_hire: false, taking_work: false } });
  });
  await visitor.goto(`/people/${ids.creativeHandle}`);
  await expect(visitor.getByText(/Vera Okafor closed their SLAPorSLOP account\./)).toBeVisible();
  await expect(visitor.getByRole("button", { name: "Account closed" })).toBeDisabled();
  await expect(visitor.getByRole("button", { name: "Hire for this" })).toHaveCount(0);
});

test("Made with people lists apps whose makers finished a People job", async ({ page }) => {
  await page.goto("/made-with-people");
  await expect(page.getByRole("heading", { name: /Made with people/, level: 1 })).toBeVisible();
  // Cleo's Tavern: the finished Tavern sign job credits Vera.
  await expect(page.getByText("Cleo's Tavern").first()).toBeVisible();
  await expect(page.getByText("Lantern Keep")).toHaveCount(0);
});

const pages: Array<[string, Who | null]> = [
  ["/people", null],
  [`/people/${ids.creativeHandle}`, null],
  ["/people/me", "creative"],
  ["/people/me", "maker"],
  ["/people/jobs", "client"],
  [`/jobs/${ids.deliveredJob}`, "client"],
  [`/jobs/${ids.requestedJob}`, "creative"],
  ["/made-with-people", null],
  ["/friends", "player"],
  ["/teams", "player"],
  [`/teams/${ids.inviteTeam}`, "player"],
];

for (const [path, who] of pages) {
  test(`${path}${who ? ` (${who})` : ""} has no serious accessibility problems`, async ({ page }) => {
    if (who) await signIn(page, who);
    await page.goto(path);
    await page.locator("main").waitFor();
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    const serious = results.violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
    expect(serious, serious.join("\n")).toEqual([]);
  });
}
