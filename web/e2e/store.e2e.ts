// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, test } from "./fixtures";

test("the store lists games, search finds them, and a game has its page", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Castle Rock").first()).toBeVisible();
  await page.goto("/search?q=Castle");
  await expect(page.getByRole("link", { name: /Castle Rock/ }).first()).toBeVisible();
  await page.goto("/app/castle-rock");
  await expect(page.getByRole("heading", { name: "Castle Rock", level: 1 })).toBeVisible();
  await expect(page.getByText("Sam Seed").first()).toBeVisible();
});

test("a game that plays online says so", async ({ page }) => {
  await page.goto("/app/neon-drift");
  await expect(page.getByText(/PLAYS ONLINE/)).toBeVisible();
});

test("a missing page says so", async ({ page }) => {
  await page.goto("/app/no-such-game");
  await expect(page.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});
