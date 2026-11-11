// SPDX-License-Identifier: AGPL-3.0-or-later
// WCAG 2.2 AA (AGENTS.md): no serious or critical problems axe can find on the main pages. Two
// apps of this file's own (store-helpers.ts) show maker news; they leave the store at the end.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { expect, signIn, test } from "./fixtures";
import { call, delist, listedApp, tryCall } from "./store-helpers";

const pages: Array<[string, "player" | "maker" | "admin" | null]> = [
  ["/", null],
  ["/search?q=castle", null],
  ["/browse", null],
  ["/browse/family", null],
  ["/discover", "player"],
  ["/tag/puzzle", null],
  ["/app/castle-rock", null],
  ["/app/neon-drift", "player"],
  // A game that plays in the browser (Orbit Tap): Play, its hint and the trust panel.
  ["/app/orbit-tap", null],
  ["/app/orbit-tap", "player"],
  ["/search?runs_on=browser", null],
  ["/library", "player"],
  ["/publish", "maker"],
  ["/publish/new", "maker"],
  ["/app/castle-rock/branch", "maker"],
  ["/publish/guide", null],
  ["/transparency", null],
  ["/verify", null],
  ["/source", null],
  ["/bounties", null],
  ["/admin", "admin"],
  ["/admin?tab=apps&app=castle-rock", "admin"],
  ["/admin?tab=money", "admin"],
  ["/admin?tab=store", "admin"],
  ["/admin?tab=docs", "admin"],
  ["/admin?tab=ai-messages", "admin"],
];

async function noSeriousProblems(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

for (const [path, who] of pages) {
  test(`${path}${who ? ` (${who})` : ""} has no serious accessibility problems`, async ({ page }) => {
    if (who) await signIn(page, who);
    await page.goto(path);
    await page.locator("main").waitFor();
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);
  });
}

// Maker news (web/src/components/News.tsx): an app page with its News section and the news switch,
// and Manage with the news form open.
test.describe("maker news", () => {
  let shown = "";
  let writing = "";
  test.beforeAll(async ({}, info) => {
    shown = await listedApp(`a11y-news-${info.retry}`, "Accessible News", { who: "maker2", stage: "coming_soon" });
    writing = await listedApp(`a11y-news-form-${info.retry}`, "Accessible News Form", { who: "maker2", stage: "coming_soon" });
    const page = await call<{ news: unknown[] }>("maker2", "GET", `/v1/apps/${shown}/news`);
    if (page.news.length === 0) {
      await call("maker2", "POST", `/v1/apps/${shown}/news`, {
        headline: "Beta opens on Friday",
        body: "The first three areas are **ready**. [Read the notes](https://example.com/notes).",
      });
    }
    // Pat marks it after the post, so nobody is told.
    await call("player", "PUT", `/v1/me/interested/${shown}`);
  });
  test.afterAll(async () => {
    await tryCall("player", "DELETE", `/v1/me/interested/${shown}`);
    await delist(shown, writing);
  });

  test("an app page with news and the news switch has no serious accessibility problems", async ({ page }) => {
    await signIn(page, "player");
    await page.goto(`/app/${shown}`);
    const news = page.getByRole("region", { name: "News", exact: true });
    await expect(news.getByRole("heading", { name: "Beta opens on Friday" })).toBeVisible();
    await expect(news.getByRole("checkbox", { name: "Tell me about news from this app" })).toBeChecked();
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);
  });

  test("Manage with the news form open has no serious accessibility problems", async ({ page }) => {
    await signIn(page, "maker2");
    await page.goto(`/publish/${writing}/manage`);
    const news = page.getByRole("region", { name: "News", exact: true });
    await news.getByRole("button", { name: "Write news" }).click();
    await expect(news.getByRole("textbox", { name: "Headline", exact: true })).toBeFocused();
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);
  });
});
