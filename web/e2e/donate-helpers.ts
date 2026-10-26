// SPDX-License-Identifier: AGPL-3.0-or-later
// The Donate button next to Play / Install / Download (web/src/components/Donate.tsx), for the
// store page and library tests.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page } from "@playwright/test";

import { expect } from "./fixtures";

/** The Donate button, whatever it says: Donate, Donated, or "$5.46/month server donation". */
export const donateButton = (scope: Page | Locator) => scope.locator("button.donate-btn");
/** The panel the button opens. */
export const donatePanel = (scope: Page | Locator) => scope.getByRole("dialog", { name: /^Donate to / });
/** The donate form in it. */
export const donateForm = (scope: Page | Locator) => scope.getByRole("form", { name: /^Donate to / });
/** The Play button's green (--slime). */
export const SLIME = "rgb(163, 230, 53)";

/** Opens the donate form from the button (through Donate again, for someone who has given). */
export async function openDonateForm(page: Page): Promise<Locator> {
  await donateButton(page).click();
  const again = donatePanel(page).getByRole("button", { name: "Donate again" });
  await expect(donateForm(page).or(again)).toBeVisible();
  if (await again.isVisible()) await again.click();
  await expect(donateForm(page)).toBeVisible();
  return donateForm(page);
}

/** The two are side by side: their middles at the same height. */
export async function expectSameRow(a: Locator, b: Locator) {
  const [x, y] = [await a.boundingBox(), await b.boundingBox()];
  expect(x && y, "both on screen").toBeTruthy();
  expect(Math.abs(x!.y + x!.height / 2 - (y!.y + y!.height / 2))).toBeLessThan(4);
}

export async function noSeriousAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}
