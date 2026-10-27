// SPDX-License-Identifier: AGPL-3.0-or-later
// Where players and makers meet on an app: bug reports ("me too", the maker's answer, how each
// release landed) and fixes a community build offers its original. Lantern Keep's seeded reports
// are put back as they were; the new report and offers are this test's own.
import type { Browser, Page } from "@playwright/test";
import { expect, ids, signIn, test, type Who } from "./fixtures";
import { call } from "./people-helpers";

async function as(browser: Browser, who: Who, path: string): Promise<Page> {
  const page = await browser.newPage();
  await signIn(page, who);
  await page.goto(path);
  return page;
}

test("bug reports: a player says 'me too' and takes it back; the maker answers a report; release feedback", async ({ browser }) => {
  // Pat has the seed's "Text too small on the map" too, then not.
  const pat = await as(browser, "player", `/app/${ids.listedApp}`);
  const section = pat.locator("#bug-reports");
  const small = section.locator("li").filter({ hasText: "Text too small on the map" });
  const meToo = small.getByRole("button", { name: "I have this too" });
  await expect(meToo).toHaveAttribute("aria-pressed", "false");
  // Your own report has no "me too".
  await expect(section.locator("li").filter({ hasText: "Fog covers the menu" }).getByRole("button", { name: "I have this too" })).toHaveCount(0);
  await meToo.click();
  await expect(meToo).toHaveAttribute("aria-pressed", "true");
  await expect(small).toContainText("2 players");
  await meToo.click();
  await expect(meToo).toHaveAttribute("aria-pressed", "false");
  await expect(small).not.toContainText("players");

  // Olly played it and reports a bug.
  await call("overdue", "POST", `/v1/apps/${ids.listedApp}/launch?platform=windows`);
  const olly = await as(browser, "overdue", `/app/${ids.listedApp}`);
  const ollySection = olly.locator("#bug-reports");
  await ollySection.getByRole("button", { name: "Report a bug" }).click();
  await ollySection.getByRole("textbox", { name: "What's wrong, in a line" }).fill("Lanterns flicker on the bridge");
  await ollySection.getByRole("textbox", { name: /What happened/ }).fill("Walk onto the bridge at night: every lantern flickers.");
  await ollySection.getByRole("button", { name: "Send to the maker" }).click();
  await expect(ollySection.getByText("Sent to the maker.")).toBeVisible();

  // Mia sees it on her own app's page only as a pointer to Manage, and answers it there.
  const mia = await as(browser, "maker", `/app/${ids.listedApp}`);
  await expect(mia.locator("#bug-reports").getByRole("link", { name: "Manage page" })).toHaveAttribute(
    "href",
    `/publish/${ids.listedApp}/manage#bug-reports`,
  );
  await mia.goto(`/publish/${ids.listedApp}/manage#bug-reports`);
  const panel = mia.locator("#bug-reports");
  const report = panel.locator("li").filter({ hasText: "Lanterns flicker on the bridge" });
  await expect(report).toContainText("Olly Overdue");
  await expect(report).toContainText("every lantern flickers");
  await expect(report.getByRole("link", { name: "Open as a GitHub issue" })).toHaveAttribute(
    "href",
    /^https:\/\/github\.com\/e2e\/lantern-keep\/issues\/new\?title=Lanterns%20flicker/,
  );
  await report.getByRole("textbox", { name: "Note for the player" }).fill("Fixed in 1.0.1: the bridge lights were doubled.");
  await report.getByRole("button", { name: "Mark fixed" }).click();
  await expect(report).toHaveCount(0);
  await panel.getByRole("combobox", { name: "Show" }).selectOption("fixed");
  const fixed = panel.locator("li").filter({ hasText: "Lanterns flicker on the bridge" });
  await expect(fixed).toContainText("FIXED");
  await expect(fixed.getByRole("textbox", { name: "Note for the player" })).toHaveValue("Fixed in 1.0.1: the bridge lights were doubled.");
  // The seed's fixed report is there too.
  await expect(panel.locator("li").filter({ hasText: "FIXED" })).toHaveCount(2);

  // Olly is told.
  await olly.goto("/notifications");
  await expect(olly.getByText(/Your bug report/).first()).toBeVisible();
  // Mia files it away as a duplicate after all, so Lantern Keep's fixed list is as the seed left it.
  await fixed.getByRole("button", { name: "Mark duplicate" }).click();
  await expect(fixed).toHaveCount(0);

  // How each release landed, from votes, reviews and reports.
  const feedback = mia.getByRole("region", { name: "How each release landed" }).or(mia.locator("section").filter({ hasText: "How each release landed" }));
  await expect(feedback.first()).toContainText("1.0.0");
  await expect(feedback.first()).toContainText(/% Slaps from 7 votes/);
  await expect(feedback.first()).toContainText("bug report");
});

test("a community build offers a fix to its original; the original declines one, accepts one, and one is withdrawn", async ({ browser }) => {
  const bo = await as(browser, "maker2", `/publish/${ids.communityBuild}/manage#offers`);
  const offers = bo.locator("#offers");
  await expect(offers.getByRole("heading", { name: /Fixes for the original/ })).toBeVisible();
  const url = offers.getByRole("textbox", { name: "Pull request on the original's repository" });
  const message = offers.getByRole("textbox", { name: "What it fixes or adds" });
  const offerIt = offers.getByRole("button", { name: "Offer it" });
  await expect(offerIt).toBeDisabled();

  // Only a pull request on the original's own repository.
  await url.fill("https://github.com/someone/else/pull/3");
  await message.fill("Linux paths fixed.");
  await offerIt.click();
  await expect(offers.getByRole("alert")).toContainText("a pull or merge request on the original's repository");

  for (const [n, text] of [
    [101, "Case-sensitive asset paths, for Linux."],
    [102, "A Vulkan fallback renderer."],
    [103, "Steam Deck controls."],
  ] as const) {
    await url.fill(`https://github.com/e2e/lantern-keep/pull/${n}`);
    await message.fill(text);
    await offerIt.click();
    await expect(url).toHaveValue("");
    await expect(offers.locator("li").filter({ hasText: text })).toContainText("WAITING");
  }
  // Bo takes the third back.
  const deck = offers.locator("li").filter({ hasText: "Steam Deck controls." });
  await deck.getByRole("button", { name: "Withdraw" }).click();
  await expect(deck).toContainText("WITHDRAWN");

  // Mia, on the original, declines one with a reply and accepts the other.
  const mia = await as(browser, "maker", `/publish/${ids.listedApp}/manage#offers`);
  const received = mia.locator("#offers");
  await expect(received.getByRole("heading", { name: "Fixes offered to you" })).toBeVisible();
  await expect(received.locator("li").filter({ hasText: "Steam Deck controls." })).toContainText("WITHDRAWN");
  const vulkan = received.locator("li").filter({ hasText: "A Vulkan fallback renderer." });
  await expect(vulkan.getByRole("link", { name: "Pull request" })).toHaveAttribute("href", "https://github.com/e2e/lantern-keep/pull/102");
  await vulkan.getByRole("textbox", { name: /Reply/ }).fill("Not for now: we're GL only.");
  await vulkan.getByRole("button", { name: "Decline" }).click();
  await expect(vulkan).toContainText("DECLINED");
  await expect(vulkan).toContainText("Your reply: Not for now: we're GL only.");
  const paths = received.locator("li").filter({ hasText: "Case-sensitive asset paths" });
  await paths.getByRole("button", { name: "Accept" }).click();
  await expect(paths).toContainText("ACCEPTED");

  // Bo sees the answers.
  await bo.reload();
  await expect(offers.locator("li").filter({ hasText: "A Vulkan fallback renderer." })).toContainText("Reply: Not for now: we're GL only.");
  await expect(offers.locator("li").filter({ hasText: "Case-sensitive asset paths" })).toContainText("ACCEPTED");
});
