// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, server, signIn, test } from "./fixtures";
import { call } from "./publishing-helpers";

test("a maker creates a store page, submits a release that passes the checks, and publishes it: no staff needed", async ({ browser }) => {
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
  // SLAPorSLOP fetches the file from the (fake) release page and checks it: it passed, and waits
  // for Mia. SLAPorSLOP staff have nothing to do.
  const release = maker.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
  await expect(async () => {
    await maker.reload();
    await expect(release.getByText("PASSED, NOT PUBLISHED", { exact: true })).toBeVisible({ timeout: 1000 });
  }).toPass({ timeout: 20_000 });
  const queue = await call<{ releases: Array<{ app: { slug: string } }> }>("admin", "GET", "/v1/admin/queue");
  expect(queue.releases.map((r) => r.app.slug)).not.toContain("pong-test");

  // Mia publishes it: SLAPorSLOP signs it then, and players get it once the app is in the store.
  await release.getByRole("button", { name: "Publish version 1.0.0…" }).click();
  await release.getByRole("button", { name: "Yes, publish it" }).click();
  await expect(release.getByText("PUBLISHED", { exact: true })).toBeVisible();
  const manifest = await call<{ payload: string }>("maker", "GET", "/v1/apps/pong-test/manifest");
  expect(manifest.payload).toBeTruthy();
  // Then putting it in the store is hers, from the progress (or Manage).
  const progress = maker.getByRole("region", { name: "Publishing progress" });
  await expect(progress.locator("li.step").filter({ hasText: "In the store" })).toContainText(
    "Ready: put it in the store when you want players to find it.",
  );
  await expect(progress.getByRole("button", { name: "Put it in the store…" })).toBeVisible();
});
