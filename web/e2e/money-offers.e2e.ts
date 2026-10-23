// SPDX-License-Identifier: AGPL-3.0-or-later
// Fixes offered from a community build to its original (web/src/components/Offers.tsx, on both
// makers' Manage pages): offering a pull request, withdrawing it, and the original's maker
// accepting or declining with a reply. On the money tests' own pair of apps (money-helpers.ts).
import { expect, ids, signIn, test } from "./fixtures";
import { offerApps } from "./money-helpers";

test("a community build offers fixes; the original's maker accepts or declines them; the builder can withdraw", async ({ browser }) => {
  const { original, build, repo } = await offerApps();
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/publish/${build}/manage`);
  const mine = bo.locator("section#offers");
  await expect(mine.getByRole("heading", { name: `Fixes for the original (${original})` })).toBeVisible();
  const url = mine.getByRole("textbox", { name: "Pull request on the original's repository" });
  const what = mine.getByRole("textbox", { name: "What it fixes or adds" });
  const offerIt = mine.getByRole("button", { name: "Offer it" });
  await expect(offerIt).toBeDisabled();

  // Only a pull request on the original's own repository.
  await url.fill("https://github.com/someone-else/game/pull/4");
  await what.fill("Night-vision colours for the fireflies.");
  await offerIt.click();
  await expect(mine.getByRole("alert")).toHaveText(`a pull or merge request on the original's repository, ${repo}`);
  // Withdrawn before anyone answers.
  await url.fill(`${repo}/pull/1`);
  await offerIt.click();
  const first = mine.getByRole("listitem").filter({ hasText: "Night-vision colours for the fireflies." });
  await expect(first.getByText("WAITING")).toBeVisible();
  await expect(url).toHaveValue("");
  await first.getByRole("button", { name: "Withdraw" }).click();
  await expect(first.getByText("WITHDRAWN")).toBeVisible();
  await expect(first.getByRole("button", { name: "Withdraw" })).toHaveCount(0);
  for (const [n, text] of [
    [2, "A colour-blind palette."],
    [3, "Fireflies glow brighter."],
  ] as const) {
    await url.fill(`${repo}/pull/${n}`);
    await what.fill(text);
    await offerIt.click();
    await expect(mine.getByRole("listitem").filter({ hasText: text }).getByText("WAITING")).toBeVisible();
  }

  // The original's maker sees them, with the pull requests, and answers.
  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/publish/${original}/manage`);
  const theirs = mia.locator("section#offers");
  await expect(theirs.getByRole("heading", { name: "Fixes offered to you" })).toBeVisible();
  const withdrawn = theirs.getByRole("listitem").filter({ hasText: "Night-vision colours" });
  await expect(withdrawn.getByText("WITHDRAWN")).toBeVisible();
  await expect(withdrawn.getByRole("button", { name: "Accept" })).toHaveCount(0);
  const palette = theirs.getByRole("listitem").filter({ hasText: "A colour-blind palette." });
  await expect(palette.getByRole("link", { name: "Firefly Garden Remix" })).toBeVisible();
  await expect(palette.getByText(/by Bo Builder/)).toBeVisible();
  await expect(palette.getByRole("link", { name: "Pull request" })).toHaveAttribute("href", `${repo}/pull/2`);
  await palette.getByRole("textbox", { name: /Reply/ }).fill("Merged, thank you!");
  await palette.getByRole("button", { name: "Accept" }).click();
  await expect(palette.getByText("ACCEPTED")).toBeVisible();
  await expect(palette.getByText("Your reply: Merged, thank you!")).toBeVisible();
  const brighter = theirs.getByRole("listitem").filter({ hasText: "Fireflies glow brighter." });
  await brighter.getByRole("button", { name: "Decline" }).click();
  await expect(brighter.getByText("DECLINED")).toBeVisible();
  await expect(brighter.getByRole("button", { name: "Decline" })).toHaveCount(0);

  // The builder reads the answers.
  await bo.reload();
  await expect(mine.getByRole("listitem").filter({ hasText: "A colour-blind palette." }).getByText("Reply: Merged, thank you!")).toBeVisible();
  await expect(mine.getByRole("listitem").filter({ hasText: "Fireflies glow brighter." }).getByText("DECLINED")).toBeVisible();

  // An original nobody offered anything to has no such section.
  await mia.goto(`/publish/${ids.draftApp}/manage`);
  await expect(mia.getByRole("heading", { name: "Starship Draft" }).first()).toBeVisible();
  await expect(mia.locator("section#offers")).toHaveCount(0);
});
