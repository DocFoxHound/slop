// SPDX-License-Identifier: AGPL-3.0-or-later
// Rating a creative after a job (pages/Jobs.tsx): the job page's next step has "Rate <name>" in
// green once the job is complete, "Your rating: …" in grey once rated (the form below still
// changes it), and before then says when it will be possible. Nothing is changed: the review
// missing from Cleo's completed job is answered in the browser.
import { tweak } from "./accounts-helpers";
import { noSeriousAxeViolations, SLIME } from "./donate-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test("the job page: Rate the creative in green once it's done, your rating in grey after, and when before", async ({ page }) => {
  await signIn(page, "client");
  // Cleo's completed job, as if she hadn't reviewed it yet.
  await tweak(page, `/v1/jobs/${ids.completedJob}`, (j) => ({ ...j, review: null }));
  await page.goto(`/jobs/${ids.completedJob}`);
  const rate = page.locator(".next-step").getByRole("button", { name: "Rate Vera Okafor" });
  await expect(rate).toHaveCSS("background-color", SLIME);
  await expect(rate).not.toHaveClass(/\bbig\b/);
  await rate.click();
  await expect(page.locator("form#review")).toBeFocused();
  await expect(page.getByRole("heading", { name: "Did Vera Okafor's work slap, or was it slop?" })).toBeVisible();
  await noSeriousAxeViolations(page);

  // Her real review: grey, and it still takes her to the form to change it.
  const real = await page.context().newPage();
  await real.goto(`/jobs/${ids.completedJob}`);
  const mine = real.locator(".next-step").getByRole("button", { name: /^Your rating: (Slaps|Slop)$/ });
  await expect(mine).toBeVisible();
  await expect(mine).not.toHaveCSS("background-color", SLIME);
  await mine.click();
  await expect(real.locator("form#review")).toBeFocused();
  await expect(real.getByRole("button", { name: "Update review" })).toBeVisible();

  // Work still under way: when she can rate.
  await real.goto(`/jobs/${ids.workingJob}`);
  await expect(real.locator(".next-step").getByText("You rate Vera Okafor's work once the job is complete.")).toBeVisible();
  await expect(real.getByRole("button", { name: /^Rate / })).toHaveCount(0);
});
