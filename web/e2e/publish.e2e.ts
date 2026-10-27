// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, server, signIn, test } from "./fixtures";

test("a maker creates a store page, submits a release, and staff approve it", async ({ browser }) => {
  const maker = await browser.newPage();
  await signIn(maker, "maker");
  await maker.goto("/publish/new");
  await maker.getByLabel("Name", { exact: true }).fill("Pong Test");
  await maker.getByLabel("One-line summary").fill("Pong, for the browser tests.");
  await maker.getByLabel("Stage").selectOption("released");
  await maker.getByRole("textbox", { name: /Source repository/ }).fill(`${server.fake}mia/pong-test`);
  await maker.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await maker.getByRole("checkbox", { name: /Windows/ }).check();
  await maker.getByRole("button", { name: "Create draft" }).click();
  await expect(maker).toHaveURL(/\/app\/pong-test$/);

  await maker.goto("/publish/pong-test/releases");
  await maker.getByRole("textbox", { name: "Version", exact: true }).fill("1.0.0");
  await maker.getByRole("textbox", { name: /^Commit/ }).fill("b".repeat(40));
  await maker.getByRole("textbox", { name: /^File URL/ }).fill(`${server.fake}mia/pong-test/releases/download/v1.0.0/pong.zip`);
  await maker.getByRole("textbox", { name: /^Program to start/ }).fill("pong.exe");
  await maker.getByRole("button", { name: "Submit" }).click();
  await expect(maker.getByRole("status")).toContainText("Submitted");
  // SLAPorSLOP fetches the file from the (fake) release page, then it waits for a person.
  await expect(async () => {
    await maker.reload();
    await expect(maker.getByText("IN REVIEW").first()).toBeVisible({ timeout: 1000 });
  }).toPass({ timeout: 20_000 });

  // Staff approve it in the admin queue.
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  // Home lists what's waiting; each links to its category in the queue.
  await admin.getByRole("link", { name: /New releases$/ }).click();
  // One release to a line; the seed has another waiting (Pocket Golf 1.1.0), which stays.
  const releases = admin.getByRole("list", { name: "New releases" });
  const row = releases.getByRole("listitem").filter({ hasText: "Pong Test" });
  await expect(row).toHaveCount(1);
  await row.getByRole("button", { name: "Approve Pong Test 1.0.0" }).click();
  await expect(row).toHaveCount(0, { timeout: 10_000 });
  await expect(admin.getByRole("status").filter({ hasText: "Approved and signed “Pong Test 1.0.0”." })).toBeVisible();
  await expect(releases.getByRole("listitem").filter({ hasText: "Pocket Golf" })).toHaveCount(1);

  await maker.reload();
  await expect(maker.getByText("APPROVED").first()).toBeVisible();
});
