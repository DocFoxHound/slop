// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("publishing a mod starts from the game it's for", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/app/castle-rock/branch?kind=addon");
  await expect(page.getByRole("heading", { name: "Publish a mod for Castle Rock" })).toBeVisible();
  await expect(page.getByText("The same rules as a new app.")).toBeVisible();
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Castle Rock Dragons");
  await page.getByRole("textbox", { name: /^Address/ }).fill("castle-rock-dragons");
  await page.getByRole("textbox", { name: "One-line summary" }).fill("Dragons for Castle Rock.");
  await page.getByRole("textbox", { name: /What it changes from Castle Rock/ }).fill("Adds three dragons.");
  await page.getByRole("textbox", { name: "Your mod's repository" }).fill("https://github.com/mia/castle-rock-dragons");
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("button", { name: "Publish the mod" }).click();
  await expect(page).toHaveURL(/\/publish\/castle-rock-dragons\/releases$/);
});

test("a game's store page can name its mods folder", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/publish/mias-first");
  await page.getByRole("radio", { name: /A folder in the game's files/ }).check();
  await page.getByRole("textbox", { name: /^Folder/ }).fill("mods");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page).toHaveURL(/\/app\/mias-first$/);
  await page.goto("/publish/mias-first");
  await expect(page.getByRole("radio", { name: /A folder in the game's files/ })).toBeChecked();
});
