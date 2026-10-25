// SPDX-License-Identifier: AGPL-3.0-or-later
// Passkeys are optional (crates/slop-api/src/passkeys.rs, the owner's decision of 2026-10-25):
// Olly has none and is long past the deadline publishers once had, and publishing never asks.
import { expect, server, signIn, test } from "./fixtures";
import { uniq } from "./publishing-helpers";

test("with no passkey, past the old deadline, a draft is made without being asked for one", async ({ page }) => {
  const slug = uniq("late-game");
  await signIn(page, "overdue");
  await page.goto("/publish");
  // A gentle suggestion, not a warning.
  const note = page.getByRole("note").filter({ hasText: "passkey" });
  await expect(note).toHaveText("Optional: a passkey lets you sign in quickly, and nobody can phish it.");
  await expect(note.getByRole("link", { name: "passkey" })).toHaveAttribute("href", "/account/security#passkeys");
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill("Late Game");
  await page.getByRole("textbox", { name: /^Store address/ }).fill(slug);
  await page.getByLabel("One-line summary").fill("Made by someone with no passkey.");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`${server.fake}olly/${slug}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("checkbox", { name: /Windows/ }).check();
  await page.getByRole("button", { name: "Create draft" }).click();
  await expect(page).toHaveURL(new RegExp(slug));
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
