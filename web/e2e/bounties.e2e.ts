// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("a player opens a bounty and pledges toward it", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/app/castle-rock");
  const section = page.locator("#bounties");
  await section.getByRole("button", { name: "Open a bounty" }).click();
  await section.getByRole("textbox", { name: "What you want, in a line" }).fill("Add a colourblind mode");
  await section.getByRole("textbox", { name: /Details/ }).fill("The red and green towers look the same to me.");
  await section.getByRole("button", { name: "Open it" }).click();
  await expect(page.getByRole("heading", { name: "Add a colourblind mode", level: 1 })).toBeVisible();
  await page.getByRole("textbox", { name: /Amount/ }).fill("12.50");
  const pledge = page.getByRole("button", { name: "Pledge" });
  await expect(pledge).toBeDisabled();
  await page.getByRole("checkbox", { name: /Save my card with Stripe/ }).check();
  await page.getByRole("checkbox", { name: "I'm 18 or older" }).check();
  await pledge.click();
  // Stripe Checkout (the fake's page) saves the card, then sends the player back.
  await expect(page.getByRole("heading", { name: "Fake Stripe Checkout" })).toBeVisible();
  await page.getByRole("button", { name: "Pay" }).click();
  await expect(page).toHaveURL(/\/bounties\/[0-9a-f-]+\?pledged=1$/);
  await expect(page.getByText(/your card is saved; charged only if a delivery is confirmed/)).toBeVisible();
  await expect(page.getByText("$12.50").first()).toBeVisible();
  await page.getByRole("button", { name: "Withdraw my pledge" }).click();
  await expect(page.getByText(/withdrawn/)).toBeVisible();
});
