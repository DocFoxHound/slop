// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page previews their portfolio waiting for review (people/mod.rs `PreviewSet`,
// components/PeopleStatus.tsx): Pip Preview is live, and her newer portfolio (other works, a new
// banner, a page design) waits. She sees it, unmistakably marked as a preview, with the way to her
// live page and back; the editor leads there; everyone else only ever gets the live page.
// Read-only: Pip's portfolios stay as the seed made them.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { closePages, newPage } from "./admin-helpers";
import { asDesktop } from "./desktop-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test.afterEach(closePages);

async function axe(page: Page) {
  await page.waitForLoadState("networkidle");
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

const pagePath = () => `/people/${ids.previewerHandle}`;
const previewBar = (page: Page) => page.getByRole("region", { name: "Preview: only you can see this" });
const liveBar = (page: Page) => page.getByRole("region", { name: "Your live page" });
const design = (page: Page) => page.getByRole("region", { name: "Pip Preview's page" });

/** Scrolls the page (it scrolls inside <main>) well down. */
async function scrollDown(page: Page) {
  await page.locator("main.view").evaluate((m) => m.scrollTo(0, 900));
  await expect.poll(() => page.locator("main.view").evaluate((m) => m.scrollTop)).toBeGreaterThan(300);
}

/** The live page, as everyone sees it: the approved works, no design, nothing of the newer one. */
async function expectLive(page: Page) {
  await expect(page.getByRole("heading", { name: "Pip Preview", level: 1 })).toBeVisible();
  await expect(page.getByRole("button", { name: "View larger: Knight" })).toBeVisible();
  await expect(page.getByRole("button", { name: "View larger: Moth" })).toHaveCount(0);
  await expect(page.getByText("Pip's new look")).toHaveCount(0);
  await expect(design(page)).toHaveCount(0);
  await expect(previewBar(page)).toHaveCount(0);
  await expect(page.getByText("Preview · not your live page")).toHaveCount(0);
}

test("the creative sees her portfolio waiting for review as a preview, and her live page a click away", async ({ page }) => {
  await signIn(page, "previewer");
  await page.goto(pagePath());

  // On top: it's a preview, only she sees it, why, and since when.
  const bar = previewBar(page);
  await expect(bar).toBeVisible();
  await expect(bar).toContainText("Only you can see this.");
  await expect(bar).toContainText("Your changes are waiting for SLAPorSLOP's review; everyone else sees your live page.");
  await expect(bar).toContainText("Waiting for review since");
  // The previewed page: the newer works, in the newer design, outlined and labelled.
  await expect(design(page).getByRole("heading", { name: "Pip's new look", level: 2 })).toBeVisible();
  await expect(design(page).getByRole("button", { name: "View larger: Moth" })).toBeVisible();
  await expect(design(page).getByRole("button", { name: "View larger: Lighthouse" })).toBeVisible();
  await expect(page.getByRole("button", { name: "View larger: Knight" })).toHaveCount(0);
  await expect(page.getByText("Preview · not your live page")).toHaveCount(2);
  expect(await page.locator(".own-preview-frame").evaluate((f) => getComputedStyle(f).outlineStyle)).toBe("dashed");
  // Her text and services aren't reviewed: they're the same in both.
  await expect(page.getByRole("heading", { name: "Pip Preview", level: 1 })).toBeVisible();
  await expect(page.getByText("Sprite sheet").first()).toBeVisible();
  // It stays on screen while she scrolls.
  await scrollDown(page);
  await expect(bar).toBeInViewport();
  await axe(page);

  // Her live page: exactly what everyone sees, saying so, with the way back.
  await bar.getByRole("link", { name: "View your live page" }).click();
  await expect(page).toHaveURL(`${pagePath()}?view=live`);
  await expect(liveBar(page)).toContainText("This is your live page, as everyone sees it.");
  await expect(liveBar(page)).toContainText("Your newer portfolio is waiting for SLAPorSLOP's review");
  await expectLive(page);
  await axe(page);
  await liveBar(page).getByRole("link", { name: "Back to your preview" }).click();
  await expect(page).toHaveURL(pagePath());
  await expect(design(page).getByRole("heading", { name: "Pip's new look" })).toBeVisible();

  // The editor leads to the preview too.
  await page.goto("/people/me");
  const portfolio = page.getByRole("region", { name: "Portfolio", exact: true });
  await expect(portfolio).toContainText("New portfolio waiting for review (2 works and your page's design).");
  await portfolio.getByRole("link", { name: "See the preview on your page" }).click();
  await expect(page).toHaveURL(pagePath());
  await expect(previewBar(page)).toBeVisible();

  // At phone width it fits, and the preview's label stays on top while she scrolls.
  await page.setViewportSize({ width: 375, height: 800 });
  await expect(previewBar(page).getByRole("link", { name: "View your live page" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  await scrollDown(page);
  await expect(page.locator(".own-preview-ribbon").first()).toBeInViewport();
  await axe(page);
});

test("in the desktop app the preview bar stays on top too", async ({ page }) => {
  await asDesktop(page, { state: "none" });
  await signIn(page, "previewer");
  await page.goto(pagePath());
  const bar = previewBar(page);
  await expect(bar).toBeVisible();
  await scrollDown(page);
  await expect(bar).toBeInViewport();
  // Nothing of the app's covers it: its link is the thing under its own middle.
  const link = bar.getByRole("link", { name: "View your live page" });
  const b = (await link.boundingBox())!;
  expect(await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.closest("a")?.textContent, [b.x + b.width / 2, b.y + b.height / 2])).toBe(
    "View your live page",
  );
});

test("nobody else ever sees the portfolio waiting for review", async ({ browser }) => {
  for (const who of ["client", null] as const) {
    const page = await newPage(browser);
    if (who) await signIn(page, who);
    // Not even in what the server sends.
    const answer = page.waitForResponse((r) => r.url().endsWith(`/v1/people/${ids.previewerHandle}`));
    await page.goto(pagePath());
    const body = await (await answer).json();
    expect(body.preview_set ?? null).toBeNull();
    expect(JSON.stringify(body)).not.toContain("new look");
    await expectLive(page);
    await expect(liveBar(page)).toHaveCount(0);
    // ?view=live is just the page for them.
    await page.goto(`${pagePath()}?view=live`);
    await expectLive(page);
    await expect(liveBar(page)).toHaveCount(0);
    await axe(page);
  }
});
