import { expect, server, signIn, test } from "./fixtures";

test("past the deadline, publishing asks for a passkey first, and keeps what was typed", async ({ page }) => {
  await signIn(page, "overdue");
  await page.goto("/publish");
  await expect(page.getByRole("note").filter({ hasText: "need a passkey now" })).toBeVisible();
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill("Late Game");
  await page.getByLabel("One-line summary").fill("Made by someone with no passkey yet.");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`${server.fake}olly/late-game`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("checkbox", { name: /Windows/ }).check();
  await page.getByRole("button", { name: "Create draft" }).click();
  const dialog = page.getByRole("dialog", { name: "Passkey needed" });
  await expect(dialog.getByRole("heading", { name: "Add a passkey first" })).toBeVisible();
  await expect(dialog.getByRole("link", { name: "Add a passkey" })).toHaveAttribute("href", "/account#sign-ins");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Late Game");
});
