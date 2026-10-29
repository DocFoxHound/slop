// SPDX-License-Identifier: AGPL-3.0-or-later
// WCAG 2.2 AA (AGENTS.md): no serious or critical problems axe can find on the main pages.
import AxeBuilder from "@axe-core/playwright";
import { expect, signIn, test } from "./fixtures";

const pages: Array<[string, "player" | "maker" | "admin" | null]> = [
  ["/", null],
  ["/search?q=castle", null],
  ["/browse/family", null],
  ["/discover", "player"],
  ["/app/castle-rock", null],
  ["/app/neon-drift", "player"],
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

for (const [path, who] of pages) {
  test(`${path}${who ? ` (${who})` : ""} has no serious accessibility problems`, async ({ page }) => {
    if (who) await signIn(page, who);
    await page.goto(path);
    await page.locator("main").waitFor();
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    const serious = results.violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
    expect(serious, serious.join("\n")).toEqual([]);
  });
}
