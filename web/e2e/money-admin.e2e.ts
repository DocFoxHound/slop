// SPDX-License-Identifier: AGPL-3.0-or-later
// Staff and money (web/src/pages/admin/DonationHolds.tsx, Money.tsx), with the maker's side on
// the Manage page (DonationsManage.tsx): turning an app's donations off until a problem is fixed,
// the maker asking, staff keeping it with an answer, then lifting it; and the payments record.
// The hold is on Vera's own game, made for it, and lifted again at the end.
import { expect, signIn, test } from "./fixtures";
import { listedApp } from "./money-helpers";

const SLUG = "held-gems";

test("staff turn an app's donations off, keep it with an answer, then lift it", async ({ browser }) => {
  await listedApp("creative", SLUG, "Held Gems");
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=apps&app=${SLUG}`);
  await expect(admin.getByRole("heading", { name: "Held Gems", level: 2 })).toBeVisible();

  await admin.getByRole("button", { name: "Pause donations" }).click();
  await admin.getByRole("radio", { name: /Turn off until fixed\./ }).check();
  await admin
    .getByRole("textbox", { name: /What's wrong and what would fix it/ })
    .fill("The gem art is from a paid asset pack without a licence. Show the licence or replace it.");
  await admin.getByRole("textbox", { name: /Ground \(optional\)/ }).fill("Content Policy: others' work");
  // Turning off is a sensitive action: where a second person must approve, staff are told so.
  let waited = false;
  await admin.route(`**/v1/admin/apps/${SLUG}/donation-hold`, (route) => {
    if (waited) return route.fallback();
    waited = true;
    return route.fulfill({
      status: 202,
      contentType: "application/json",
      body: JSON.stringify({ pending_action: "00000000-0000-0000-0000-000000000000", approvals_still_needed: 1 }),
    });
  });
  await admin.getByRole("button", { name: "Turn off donations" }).click();
  await expect(admin.getByRole("status").filter({ hasText: "Recorded. It happens once another staff member approves it" })).toBeVisible();
  // Here one approval is enough: done at once.
  await admin.getByRole("button", { name: "Pause donations" }).click();
  await admin.getByRole("radio", { name: /Turn off until fixed\./ }).check();
  await admin
    .getByRole("textbox", { name: /What's wrong and what would fix it/ })
    .fill("The gem art is from a paid asset pack without a licence. Show the licence or replace it.");
  await admin.getByRole("textbox", { name: /Ground \(optional\)/ }).fill("Content Policy: others' work");
  await admin.getByRole("button", { name: "Turn off donations" }).click();
  await expect(admin.getByText("TURNED OFF UNTIL FIXED")).toBeVisible();

  // The maker reads why, and asks once it's fixed (a refusal shows; then it's sent).
  const vera = await browser.newPage();
  await signIn(vera, "creative");
  await vera.goto(`/publish/${SLUG}/manage#donations`);
  const notice = vera.locator("#donations").getByRole("status");
  await expect(notice.getByText("Donations to this app are turned off until the problem below is fixed.")).toBeVisible();
  await expect(notice.getByText(/without a licence/)).toBeVisible();
  await expect(notice.getByText("Ground: Content Policy: others' work")).toBeVisible();
  const ask = notice.getByRole("textbox", { name: /Fixed it\?/ });
  await ask.fill("Bought the licence; it's in LICENSES.md.");
  await vera.route(`**/v1/apps/${SLUG}/donations/hold/request`, (route) =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: { code: "conflict", message: "you asked already: wait for Slop's answer" } }),
    }),
  );
  await notice.getByRole("button", { name: "Ask for a review" }).click();
  await expect(notice.getByRole("alert")).toHaveText("you asked already: wait for Slop's answer");
  await vera.unroute(`**/v1/apps/${SLUG}/donations/hold/request`);
  await notice.getByRole("button", { name: "Ask for a review" }).click();
  await expect(notice.getByText("Waiting for Slop's answer.")).toBeVisible();
  await expect(ask).toHaveCount(0);

  // Staff keep it, and say why.
  await admin.goto("/admin?tab=money");
  const card = admin.locator(".panel").filter({ has: admin.getByRole("link", { name: "Held Gems" }) });
  await expect(card.getByText("MAKER ASKS TO LIFT")).toBeVisible();
  await expect(card.getByText(/Bought the licence/)).toBeVisible();
  const answer = card.getByRole("textbox", { name: "Your answer to the maker" });
  await expect(card.getByRole("button", { name: "Not yet: keep it" })).toBeDisabled();
  await answer.fill("LICENSES.md doesn't list the gem pack yet.");
  await card.getByRole("button", { name: "Not yet: keep it" }).click();
  await expect(card.getByText("Kept: LICENSES.md doesn't list the gem pack yet.")).toBeVisible();
  await expect(card.getByText("MAKER ASKS TO LIFT")).toHaveCount(0);

  // The maker reads the answer, and can't ask again for a day.
  await vera.reload();
  await expect(notice.getByText("Slop kept the pause: LICENSES.md doesn't list the gem pack yet.")).toBeVisible();
  await expect(notice.getByText(/^You can ask again after /)).toBeVisible();
  await expect(notice.getByRole("textbox", { name: /Fixed it\?/ })).toHaveCount(0);

  // Staff lift it after all.
  await card.getByRole("button", { name: "Lift the pause" }).click();
  await card.getByRole("textbox", { name: "Why, as the maker will read it" }).fill("Confirmed with the asset pack's seller.");
  await card.getByRole("button", { name: "Lift it: donations back on" }).click();
  await expect(card).toHaveCount(0);
  // Lifted ones are shown on request.
  await admin.getByRole("checkbox", { name: "Show lifted ones too" }).check();
  await expect(card.getByText("LIFTED", { exact: true }).first()).toBeVisible();
  await expect(card.getByText(/^Lifted by .*: Confirmed with the asset pack's seller\.$/).first()).toBeVisible();
  await vera.reload();
  await expect(vera.locator("#donations").getByRole("heading", { name: "Donations" })).toBeVisible();
  await expect(notice).toHaveCount(0);
});

test("the payments record is for finance staff", async ({ browser }) => {
  const fin = await browser.newPage();
  await signIn(fin, "finance");
  await fin.goto("/admin?tab=money");
  await expect(fin.getByRole("heading", { name: "Payments", exact: true })).toBeVisible();
  await expect(fin.getByText(/^Payments are live through Stripe · \d+ Stripe events in the last day · \d+ monthly server donations active/)).toBeVisible();
  const months = fin.locator("section").filter({ has: fin.getByRole("heading", { name: "The last 12 months" }) });
  await expect(months.getByRole("row").filter({ hasText: "donation" }).first()).toBeVisible();
  const newest = fin.locator("section").filter({ has: fin.getByRole("heading", { name: "Newest payments" }) });
  await expect(newest.getByRole("row").nth(1)).toBeVisible();
  await expect(newest.getByRole("row").nth(1).getByRole("cell").nth(4)).toContainText(/acct_/);

  // Others see the donation pauses, but not the payments record.
  const sue = await browser.newPage();
  await signIn(sue, "support");
  await sue.goto("/admin?tab=money");
  await expect(sue.getByRole("heading", { name: /^Donation pauses/ })).toBeVisible();
  await expect(sue.getByText("The payments record is for staff with the finance role.")).toBeVisible();
  await expect(sue.getByRole("heading", { name: "Newest payments" })).toHaveCount(0);
});
