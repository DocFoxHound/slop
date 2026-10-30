// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page design (people/design.rs, components/PeopleDesign.tsx): Dee Designer
// checks her portfolio, whose repository has .slop/page/ (e2e_server/main.rs); her editor previews
// the design with what SLAPorSLOP removed; staff see it in the review inbox and approve it; then her
// page shows SLAPorSLOP's header, her design, and SLAPorSLOP's services and reviews below. Dee is only used
// here: she's live afterwards, like Ivy after the inbox test.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page } from "@playwright/test";

import { closePages, inbox, inboxRow, newPage } from "./admin-helpers";
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

async function box(l: Locator) {
  const b = await l.boundingBox();
  if (!b) throw new Error("not on screen");
  return b;
}

test("a creative's design: previewed in the editor, reviewed with the portfolio, shown between SLAPorSLOP's header and reviews", async ({ browser }) => {
  test.slow();
  // Dee checks her portfolio: the works and the design wait for review together.
  const dee = await newPage(browser);
  await signIn(dee, "designer");
  await dee.goto("/people/me");
  await dee.getByRole("textbox", { name: "Repository" }).fill("https://github.com/e2e/dee-design");
  await dee.getByRole("button", { name: "Check my portfolio now" }).click();
  await expect(dee.getByText("New portfolio waiting for review (3 works and your page's design).")).toBeVisible();

  // The editor's preview: the design as it will show, SLAPorSLOP's works in it, and what was removed.
  const preview = dee.getByRole("region", { name: "Preview of Dee Designer's page" });
  await expect(preview.getByRole("heading", { name: "Hello, I'm Dee", level: 2 })).toBeVisible();
  await expect(preview.getByRole("img", { name: "A row of lanterns at dusk" })).toBeVisible();
  await expect(preview.getByRole("button", { name: "View larger: Lantern" })).toHaveCount(2);
  await expect(preview.getByRole("button", { name: "Play video: Showreel" })).toBeVisible();
  await expect(preview.getByRole("link", { name: /my site/ })).toHaveAttribute("rel", "nofollow noopener noreferrer ugc");
  await dee.getByText("3 notes on the design: what SLAPorSLOP removed, and what it hides").click();
  await expect(dee.getByText(".slop/page/index.html line 5: <script> was removed with what's inside: pages can't run scripts")).toBeVisible();
  // Services are never inside a design: the preview shows them below it, as the page does.
  await expect(dee.getByText(/index\.html line 16: <slop-services> was removed: your services always appear below your design/)).toBeVisible();
  await expect(dee.getByRole("region", { name: "Services, below the design" }).getByText("Sprite sheet")).toBeVisible();
  await expect(dee.getByText(/style\.css line 7: "position: fixed" was removed/)).toBeVisible();
  // Its pictures open large, like on the page.
  await preview.getByRole("button", { name: "View larger: Lantern" }).first().click();
  await expect(dee.getByRole("dialog", { name: "Lantern" })).toBeVisible();
  await dee.getByRole("button", { name: "Close" }).click();
  await axe(dee);

  // Staff see the same preview in the review inbox, with the exact HTML and CSS, and approve it.
  const staff = await newPage(browser);
  await signIn(staff, "reviewer");
  await staff.goto(inbox("people"));
  await inboxRow(staff, "Dee Designer").getByRole("link", { name: /^Dee Designer/ }).click();
  const card = staff.locator(".inbox-detail");
  const review = card.getByRole("region", { name: "Page design" });
  await expect(review.getByRole("region", { name: "Preview of Dee Designer's page design" }).getByRole("heading", { name: "Hello, I'm Dee" })).toBeVisible();
  await expect(review.getByRole("region", { name: "Services, below the design" }).getByText("Sprite sheet")).toBeVisible();
  // At phone width, the design's width query applies (it's about the design's own box).
  const reviewHeading = review.getByRole("heading", { name: "Hello, I'm Dee" });
  expect(await reviewHeading.evaluate((h) => getComputedStyle(h).fontSize)).toBe("32px");
  await review.getByRole("button", { name: "Phone" }).click();
  await expect(review.getByRole("button", { name: "Phone" })).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => reviewHeading.evaluate((h) => getComputedStyle(h).fontSize)).toBe("20px");
  await review.getByRole("button", { name: "Desktop" }).click();
  await review.getByText("The HTML and CSS, as SLAPorSLOP will show them").click();
  await expect(review.getByText(/<slop-work work="image:works\/lantern.png"><\/slop-work>/)).toBeVisible();
  await axe(staff);
  await card.getByRole("button", { name: "Approve portfolio" }).click();
  await expect(staff.getByText(/“Dee Designer” isn.t waiting any more/)).toBeVisible();

  // Everyone sees it: SLAPorSLOP's header on top, the design in its own region, then SLAPorSLOP's sections.
  const visitor = await newPage(browser);
  await visitor.goto(`/people/${ids.designerHandle}`);
  const name = visitor.getByRole("heading", { name: "Dee Designer", level: 1 });
  await expect(name).toBeVisible();
  const design = visitor.getByRole("region", { name: "Dee Designer's page" });
  await expect(design.getByRole("heading", { name: "Hello, I'm Dee", level: 2 })).toBeVisible();
  const reviews = visitor.getByRole("heading", { name: "Reviews from clients" });
  const services = visitor.getByRole("heading", { name: "Services", exact: true });
  await expect(reviews).toBeVisible();
  // SLAPorSLOP's Services section follows the design, always (its <slop-services> was removed).
  await expect(services).toBeVisible();
  const head = await box(visitor.locator(".person-head"));
  const area = await box(design);
  expect(head.y + head.height).toBeLessThanOrEqual(area.y);
  expect(area.y + area.height).toBeLessThanOrEqual((await box(services)).y);
  expect(area.y + area.height).toBeLessThanOrEqual((await box(reviews)).y);
  // The design's CSS styles the design (SLAPorSLOP's own font for it), and nothing outside it.
  const heading = design.getByRole("heading", { name: "Hello, I'm Dee" });
  expect(await heading.evaluate((h) => getComputedStyle(h).fontFamily)).toContain("Pixelify Sans");
  expect(await heading.evaluate((h) => getComputedStyle(h).color)).toBe("rgb(255, 209, 102)");
  expect(await services.evaluate((h) => getComputedStyle(h).fontFamily)).not.toContain("Pixelify Sans");
  expect(await design.evaluate((el) => getComputedStyle(el).contain)).toContain("paint");
  // No script ran, nothing is fixed over the page, and SLAPorSLOP's header still works.
  expect(await visitor.title()).not.toBe("not allowed");
  const report = visitor.locator(".person-head").getByRole("button", { name: "Report", exact: true });
  await expect(report).toBeVisible();
  const r = await box(report);
  expect(
    await visitor.evaluate(([x, y]) => document.elementFromPoint(x, y)?.closest(".person-head") !== null, [r.x + r.width / 2, r.y + r.height / 2]),
  ).toBe(true);
  // The works in the design are SLAPorSLOP's: the grid opens pictures large; keyboard reaches them.
  await design.getByRole("button", { name: "View larger: The keep" }).focus();
  await visitor.keyboard.press("Enter");
  await expect(visitor.getByRole("dialog", { name: "The keep" })).toBeVisible();
  await visitor.getByRole("button", { name: "Close" }).click();
  await axe(visitor);

  // At phone width it fits.
  await visitor.setViewportSize({ width: 375, height: 800 });
  await expect(design.getByRole("heading", { name: "Hello, I'm Dee" })).toBeVisible();
  expect(await visitor.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});
