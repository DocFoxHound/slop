// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("a player who played a game reports a bug to its maker", async ({ page }) => {
  await signIn(page, "player");
  // Launching through Slop is what lets a player report (the Slop app does this on Play).
  const launched = await page.request.post("/v1/apps/castle-rock/launch?platform=windows");
  expect(launched.ok()).toBeTruthy();
  await page.goto("/app/castle-rock");
  const section = page.locator("#bug-reports");
  await section.getByRole("button", { name: "Report a bug" }).click();
  await section.getByRole("textbox", { name: "What's wrong, in a line" }).fill("Towers vanish after saving");
  await section.getByRole("textbox", { name: /What happened/ }).fill("Save, quit, start again: the towers are gone.");
  await section.getByRole("button", { name: "Send to the maker" }).click();
  await expect(section.getByText("Sent to the maker.")).toBeVisible();
  await expect(section.getByText("Towers vanish after saving")).toBeVisible();
});
