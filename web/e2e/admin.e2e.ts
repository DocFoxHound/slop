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

  // Staff see the request in the review queue, open it and lift the pause.
  await admin.goto("/admin");
  await admin.getByRole("link", { name: /^1 Donation pause requests$/ }).click();
  const request = admin.getByRole("list", { name: "Donation pause requests" }).getByRole("listitem").filter({ hasText: "Mia's First Game" });
  await expect(request.getByText("Swapped in CC0 music; licence file added.")).toBeVisible();
  await request.getByRole("link").click();
  await expect(admin.getByText("MAKER ASKS TO LIFT")).toBeVisible();
  await admin.getByRole("textbox", { name: "Your answer to the maker" }).fill("Thanks, the licence checks out.");
  await admin.getByRole("button", { name: "It's fixed: lift the pause" }).click();
  await expect(admin.getByText("Nothing waiting here.")).toBeVisible();
  await expect(admin.getByRole("status").filter({ hasText: "isn't waiting any more" })).toBeVisible();
  await admin.goto("/admin?tab=money");
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

test("staff freeze an app, make a front-page shelf, and fix profiles", async ({ browser }) => {
  const admin = await browser.newPage();
  await signIn(admin, "admin");

  // A hand freeze: the maker sees why, and staff lift it.
  await admin.goto("/admin?tab=apps&app=mias-first");
  await admin.getByRole("button", { name: "Freeze" }).click();
  await admin.getByRole("textbox", { name: "Freeze: reason" }).fill("Checking a report about the last build.");
  await admin.getByRole("button", { name: "Freeze" }).click();
  await expect(admin.getByText("FROZEN", { exact: true })).toBeVisible();
  const maker = await browser.newPage();
  await signIn(maker, "maker");
  await maker.goto("/publish/mias-first/manage");
  await expect(maker.getByRole("heading", { name: "Frozen by Slop" })).toBeVisible();
  await expect(maker.getByText(/Checking a report about the last build/)).toBeVisible();
  await admin.getByRole("button", { name: "Lift the freeze" }).click();
  await admin.getByRole("textbox", { name: "Lift the freeze: reason" }).fill("The report was wrong.");
  await admin.getByRole("button", { name: "Lift the freeze" }).click();
  await expect(admin.getByText("FROZEN", { exact: true })).toHaveCount(0);

  // A shelf on the front page, then gone again.
  await admin.goto("/admin?tab=store");
  await admin.getByRole("button", { name: "New shelf" }).click();
  await admin.getByRole("textbox", { name: "Address", exact: true }).fill("staff-picks");
  await admin.getByRole("textbox", { name: "Title", exact: true }).fill("Staff picks this week");
  await admin.getByRole("textbox", { name: /App address to add/ }).fill("castle-rock");
  await admin.getByRole("button", { name: "Add app" }).click();
  await admin.getByRole("button", { name: "Make the shelf" }).click();
  await expect(admin.getByText("Staff picks this week").first()).toBeVisible();
  const visitor = await browser.newPage();
  await visitor.goto("/");
  await expect(visitor.getByRole("heading", { name: "Staff picks this week" })).toBeVisible();
  await admin.getByText("Staff picks this week").first().click();
  admin.once("dialog", (d) => void d.accept());
  await admin.getByRole("button", { name: "Delete shelf" }).click();
  await expect(admin.getByText("No shelves yet.")).toBeVisible();

  // An account's profile can be fixed from Accounts.
  await admin.goto("/admin?tab=users&q=Mia%20Maker");
  await admin.getByRole("link", { name: "Mia Maker" }).click();
  await admin.getByText("Fix their profile").click();
  await expect(admin.getByRole("textbox", { name: "Display name" })).toHaveValue("Mia Maker");

  // The Terms & policies tab lists every document.
  await admin.goto("/admin?tab=docs");
  await expect(admin.getByRole("heading", { name: "Terms of Service" })).toBeVisible();
});
