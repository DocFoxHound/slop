// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("staff pause an app's donations, the maker asks for them back, and staff lift the pause", async ({ browser }) => {
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  await expect(admin.getByRole("heading", { name: "Needs you" })).toBeVisible();

  // Find the app from Home and open its record.
  await admin.getByRole("textbox", { name: /Find/ }).fill("Mia");
  await admin.getByRole("button", { name: "Apps", exact: true }).click();
  await admin.getByRole("link", { name: "Mia's First Game" }).click();
  await expect(admin.getByRole("heading", { name: "Mia's First Game", level: 2 })).toBeVisible();

  await admin.getByRole("button", { name: "Pause donations" }).click();
  await admin
    .getByRole("textbox", { name: /What's wrong and what would fix it/ })
    .fill("A complaint says the music is someone else's. Replace it or show the licence.");
  await admin.getByRole("button", { name: "Pause donations" }).click();
  await expect(admin.getByText("PAUSED FOR REVIEW")).toBeVisible();

  // The maker reads why and asks for a review.
  const maker = await browser.newPage();
  await signIn(maker, "maker");
  await maker.goto("/publish/mias-first/manage");
  await expect(maker.getByText("Donations to this app are paused while Slop reviews it.")).toBeVisible();
  await expect(maker.getByText(/the music is someone else's/)).toBeVisible();
  await maker.getByRole("textbox", { name: /Fixed it\?/ }).fill("Swapped in CC0 music; licence file added.");
  await maker.getByRole("button", { name: "Ask for a review" }).click();
  await expect(maker.getByText("Waiting for Slop's answer.")).toBeVisible();

  // Staff see the request under Money and lift the pause.
  await admin.goto("/admin");
  await admin.getByRole("link", { name: /Makers asking for donations back$/ }).click();
  await expect(admin.getByText("MAKER ASKS TO LIFT")).toBeVisible();
  await admin.getByRole("textbox", { name: "Your answer to the maker" }).fill("Thanks, the licence checks out.");
  await admin.getByRole("button", { name: "It's fixed: lift the pause" }).click();
  await expect(admin.getByText("No donation pauses.")).toBeVisible();

  await maker.reload();
  await expect(maker.getByText(/Donations to this app are paused/)).toHaveCount(0);

  // The maker's account, from Accounts.
  await admin.goto("/admin?tab=users");
  await admin.getByRole("textbox", { name: /Find an account/ }).fill("Mia Maker");
  await admin.getByRole("button", { name: "Search" }).click();
  await admin.getByRole("link", { name: "Mia Maker" }).click();
  await expect(admin.getByRole("link", { name: "Mia's First Game" })).toBeVisible();
});
