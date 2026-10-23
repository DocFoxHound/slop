// SPDX-License-Identifier: AGPL-3.0-or-later
// WCAG 2.2 AA on the account, sign-in and social pages (as a11y.e2e.ts does for the main pages):
// no serious or critical problems axe can find.
import AxeBuilder from "@axe-core/playwright";

import { expect, ids, signIn, test, type Who } from "./fixtures";

const pages: Array<[string, Who | null]> = [
  ["/signin", null],
  ["/signin?error=provider&reason=link", null],
  ["/signup", null],
  ["/account", "player"],
  ["/account", "maker"],
  ["/account", "client"],
  ["/account", "suspended"],
  ["/account/combine", "player"],
  ["/notifications", "player"],
  ["/friends", "player"],
  ["/friends", "teen"],
  ["/teams", "player"],
  [`/teams/${ids.inviteTeam}`, "player"],
  [`/teams/${ids.openTeam}`, "teen"],
  [`/u/${ids.player}`, null],
  [`/u/${ids.creative}`, "player"],
  ["/chat", null],
  ["/admin?tab=legal", "admin"],
  ["/people/me", "creative"],
  ["/publish", "maker2"],
  ["/nope", null],
];

for (const [path, who] of pages) {
  test(`${path}${who ? ` (${who})` : ""} has no serious accessibility problems`, async ({ page }) => {
    // Listing Sid's decisions would mark his seeded one seen; an empty list checks the rest.
    if (who === "suspended") await page.route("**/v1/me/decisions", (r) => r.fulfill({ json: [] }));
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
