import { expect, signIn, test } from "./fixtures";

test("signed out, adding to the library asks you to sign in", async ({ page }) => {
  await page.goto("/app/castle-rock");
  await expect(page.getByRole("link", { name: /Sign in to add to your library/ })).toBeVisible();
});

test("a player adds a game to their library", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/app/castle-rock");
  await page.getByRole("button", { name: "+ Add to library" }).click();
  await expect(page.getByRole("button", { name: "✓ In your library" })).toBeVisible();
  await page.goto("/library");
  await expect(page.getByRole("link", { name: "Castle Rock" }).first()).toBeVisible();
});

test("a player reports a game, and it reaches staff", async ({ browser }) => {
  const player = await browser.newPage();
  await signIn(player, "player");
  await player.goto("/app/neon-drift");
  await player.getByRole("button", { name: "⚑ Report" }).click();
  const dialog = player.getByRole("dialog", { name: "Report this app" });
  await dialog.getByRole("radio").first().check();
  await dialog.getByRole("textbox", { name: /What did you see/ }).fill("The browser tests saw something odd here.");
  await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("button", { name: "Close" })).toBeVisible();

  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  await expect(admin.getByText("The browser tests saw something odd here.").first()).toBeVisible();
});
