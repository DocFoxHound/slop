// SPDX-License-Identifier: AGPL-3.0-or-later
// Money on the account page (components/AccountSections.tsx): your donations, where donations to
// your apps are paid, and Slop's fee statements with the VAT in them.
import { apiError, call, fake, passkeyOptions, stubPasskeys, tweak } from "./accounts-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test.describe("your donations", () => {
  test("a one-off donation shows what was paid", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/account#donations");
    const section = page.locator("#donations");
    const row = section.getByRole("row").filter({ hasText: "Lantern Keep" });
    await expect(row.getByRole("link", { name: "Lantern Keep" })).toHaveAttribute("href", "/app/lantern-keep");
    await expect(row).toContainText("$5.46");
    await expect(row).toContainText("Paid");
    await expect(row).not.toContainText("monthly");
    await expect(section).toContainText("Receipts for real payments come by email from the maker's Stripe account.");
  });

  test("a monthly server donation can be cancelled, after saying yes (and Keep it keeps it)", async ({ page }) => {
    await signIn(page, "client");
    await page.goto("/account#donations");
    const section = page.locator("#donations");
    const monthly = section.getByRole("listitem").filter({ hasText: "Server donation to Lantern Keep" });
    await expect(monthly).toContainText("$4.43 a month");
    await expect(monthly).toContainText(/Next payment \w{3} \d+, \d{4}/);
    await monthly.getByText("What you agreed to").click();
    await expect(monthly).toContainText("I'm starting a monthly server donation to Lantern Keep");
    await expect(section.getByRole("row").filter({ hasText: "Lantern Keep" })).toContainText("monthly");
    // Cleo's donation is the seed's only one; cancelling it is checked against a stand-in.
    await monthly.getByRole("button", { name: "Cancel" }).click();
    await monthly.getByRole("button", { name: "Keep it" }).click();
    await expect(monthly.getByRole("button", { name: "Yes, cancel it" })).toHaveCount(0);

    let cancelled = false;
    const cancel = await fake(page, "POST", `/v1/me/server-donations/${ids.serverDonation}/cancel`, () => {
      cancelled = true;
      return { status: 204, body: undefined };
    });
    await tweak(page, "/v1/me/donations", (d) => ({
      ...d,
      server_donations: d.server_donations.map((s: Record<string, unknown>) =>
        cancelled ? { ...s, status: "cancelled", cancelled_at: "2026-10-23T12:00:00Z", cancelled_by: "donor" } : s,
      ),
    }));
    await monthly.getByRole("button", { name: "Cancel" }).click();
    await monthly.getByRole("button", { name: "Yes, cancel it" }).click();
    await expect(monthly).toContainText("Cancelled Oct 23, 2026");
    await expect(monthly.getByRole("button")).toHaveCount(0);
    expect(cancel.calls).toHaveLength(1);
    expect((await call("client", "GET", "/v1/me/donations")).server_donations[0].status).toBe("active");
  });

  test("a cancel that fails says why; servers shutting down and simulated payments are marked", async ({ page }) => {
    await fake(page, "POST", `/v1/me/server-donations/${ids.serverDonation}/cancel`, {
      status: 409,
      body: apiError("conflict", "that donation is already cancelled"),
    });
    await tweak(page, "/v1/me/donations", (d) => ({
      donations: d.donations.map((x: Record<string, unknown>) => ({ ...x, simulated: true })),
      server_donations: [
        ...d.server_donations,
        {
          ...d.server_donations[0],
          id: "01a00000-0000-7000-8000-0000000000aa",
          app_title: "Neon Drift",
          status: "cancelled",
          cancelled_at: "2026-10-20T12:00:00Z",
          cancelled_by: "servers_shut_down",
          simulated: true,
        },
      ],
    }));
    await signIn(page, "client");
    await page.goto("/account#donations");
    const section = page.locator("#donations");
    await expect(section.getByRole("listitem").filter({ hasText: "Neon Drift" })).toContainText(
      "Cancelled Oct 20, 2026 (the servers shut down) · simulated",
    );
    await expect(section.getByRole("row").filter({ hasText: "Lantern Keep" })).toContainText("simulated");
    const monthly = section.getByRole("listitem").filter({ hasText: "Server donation to Lantern Keep" });
    await monthly.getByRole("button", { name: "Cancel" }).click();
    await monthly.getByRole("button", { name: "Yes, cancel it" }).click();
    await expect(section.getByRole("alert")).toHaveText("that donation is already cancelled");
  });

  test("with no donations, there's no donations section", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account");
    await expect(page.getByRole("heading", { name: "Terms and policies" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Your donations" })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Receiving donations" })).toHaveCount(0);
  });
});

test.describe("receiving donations", () => {
  test("switching to another Stripe account waits 72 hours and can be cancelled", async ({ page, context }) => {
    await signIn(page, "maker");
    try {
      await page.goto("/account#receiving");
      const section = page.locator("#receiving");
      await expect(section).toContainText("Donations to your apps go to your own Stripe account. Bank details are changed in Stripe, not here.");
      await section.getByRole("button", { name: "Use a different Stripe account" }).click();
      await expect(section).toContainText("new donations start going to it 72 hours from now");
      await section.getByRole("button", { name: "Never mind" }).click();
      await section.getByRole("button", { name: "Use a different Stripe account" }).click();
      // The new account's sign-up opens on Stripe (the fake one) in a new tab.
      const stripe = context.waitForEvent("page");
      await section.getByRole("button", { name: "Set up the new account" }).click();
      await (await stripe).close();
      const notice = section.getByRole("status");
      await expect(notice).toContainText("Switching to a new Stripe account");
      const asked = await call("maker", "GET", "/v1/me/donation-account");
      const on = new Date(asked.change.effective_at).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
      await expect(notice).toContainText(`It happens on ${on}`);
      await expect(notice).toContainText("If you didn't ask for this, cancel it now");
      if (!asked.change.new_account_ready) {
        await expect(notice).toContainText("once you've finished the new account's sign-up on Stripe");
        await notice.getByRole("button", { name: "I've finished on Stripe" }).click();
      }
      await notice.getByRole("button", { name: "Cancel the change" }).click();
      await expect(section.getByRole("button", { name: "Use a different Stripe account" })).toBeVisible();
      expect((await call("maker", "GET", "/v1/me/donation-account")).change).toBeNull();
    } finally {
      const now = await call("maker", "GET", "/v1/me/donation-account");
      if (now.change) await call("maker", "POST", "/v1/me/donation-account/change/cancel");
    }
  });

  test("disconnecting says what stops, and can be cancelled", async ({ page }) => {
    await signIn(page, "maker");
    try {
      await page.goto("/account#receiving");
      const section = page.locator("#receiving");
      await section.getByRole("button", { name: "Disconnect Stripe" }).click();
      await expect(section).toContainText("In 72 hours donations turn off on all your apps and monthly server donations end.");
      await section.getByRole("button", { name: "Disconnect in 72 hours" }).click();
      const notice = section.getByRole("status");
      await expect(notice).toContainText("Disconnecting your Stripe account");
      await expect(notice).toContainText("Donations then turn off on all your apps, and monthly server donations end.");
      await expect(notice.getByRole("button", { name: "I've finished on Stripe" })).toHaveCount(0);
      await notice.getByRole("button", { name: "Cancel the change" }).click();
      await expect(section.getByRole("button", { name: "Disconnect Stripe" })).toBeVisible();
    } finally {
      const now = await call("maker", "GET", "/v1/me/donation-account");
      if (now.change) await call("maker", "POST", "/v1/me/donation-account/change/cancel");
    }
  });

  test("an old sign-in is sent to the sign-in page, not straight to GitHub (b64)", async ({ page }) => {
    const change = await fake(page, "POST", "/v1/me/donation-account/change", {
      status: 403,
      body: apiError("step_up_required", "this needs a recent sign-in"),
    });
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    await fake(page, "POST", "/v1/me/step-up/finish", { status: 204, body: undefined });
    await signIn(page, "maker");
    await page.goto("/account#receiving");
    const section = page.locator("#receiving");
    await section.getByRole("button", { name: "Disconnect Stripe" }).click();
    await section.getByRole("button", { name: "Disconnect in 72 hours" }).click();
    const alert = section.getByRole("alert");
    await expect(alert).toContainText("For your safety, this needs a sign-in from the last few minutes.");
    // Someone who signs in with Google, GitLab or a passkey can't use GitHub to do it.
    await expect(alert.getByRole("link", { name: "sign in again" })).toHaveAttribute("href", "/signin?return_to=%2Faccount%23receiving");
    await alert.getByRole("button", { name: "Confirm with a passkey" }).click();
    await expect(section.getByRole("status")).toHaveText("Confirmed. Try once more.");
    expect(change.calls).toHaveLength(1);
  });

  test("a payout change asks for a passkey first", async ({ page }) => {
    const change = await fake(page, "POST", "/v1/me/donation-account/change", {
      status: 403,
      body: apiError("passkey_required", "changing where money goes needs a passkey", "add"),
    });
    await signIn(page, "maker");
    await page.goto("/account#receiving");
    const section = page.locator("#receiving");
    await section.getByRole("button", { name: "Use a different Stripe account" }).click();
    await section.getByRole("button", { name: "Set up the new account" }).click();
    const dialog = page.getByRole("dialog", { name: "Passkey needed" });
    await expect(dialog.getByRole("heading", { name: "Add a passkey first" })).toBeVisible();
    await dialog.getByRole("button", { name: "I've added one: try again" }).click();
    await expect(dialog.getByRole("heading", { name: "Add a passkey first" })).toBeVisible();
    expect(change.calls).toHaveLength(2);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(section.getByRole("alert")).toHaveText("changing where money goes needs a passkey");
  });

  test("Stripe still checking, or restricted, is said", async ({ page }) => {
    let status = "pending";
    await tweak(page, "/v1/me/donation-account", (a) => ({ ...a, status }));
    await signIn(page, "maker");
    await page.goto("/account#receiving");
    await expect(page.locator("#receiving")).toContainText("go to your own Stripe account, which Stripe is still checking.");
    status = "restricted";
    await page.reload();
    await expect(page.locator("#receiving")).toContainText("which Stripe has restricted: open Stripe to fix it.");
  });
});

test.describe("fee statements", () => {
  test("show Slop's fee by month, with no VAT where there's none", async ({ page }) => {
    await signIn(page, "maker");
    await page.goto("/account#fee-statements");
    const section = page.locator("#fee-statements");
    await expect(section).toContainText("Slop's 5% fee on the donations and People payments you received, by month.");
    await expect(section).toContainText("There's no VAT on Slop's fee where you are.");
    await expect(section.getByRole("columnheader")).toHaveText(["Month", "For", "Payments", "Received", "Slop fee"]);
    const statement = (await call("maker", "GET", "/v1/me/fee-statements")).statements[0];
    const row = section.getByRole("row").filter({ hasText: statement.month });
    await expect(row).toContainText("Donations");
    await expect(row).toContainText(`$${(statement.received_minor / 100).toFixed(2)}`);
    await expect(row).toContainText(`$${(statement.slop_fee_minor / 100).toFixed(2)}`);
    await expect(section.getByRole("textbox", { name: /VAT number/ })).toHaveCount(0);
  });

  test("with VAT in the fee: the column, and a VAT number to save, which the server checks", async ({ page }) => {
    // Mia is in the US; her statements are shown as an EU maker's would be. Saving goes to the
    // server, which checks the number belongs to her country.
    await tweak(page, "/v1/me/fee-statements", (s) => ({
      statements: [...s.statements, { ...s.statements[0], kind: "people", vat_minor: 7, simulated: true }],
      vat: { kind: "included", rate_bp: 2000, country: "US", vat_number: null, vat_number_checked: false, slop_vat_id: null },
    }));
    await signIn(page, "maker");
    await page.goto("/account#fee-statements");
    const section = page.locator("#fee-statements");
    await expect(section).toContainText("The fee includes 20% VAT, which Slop pays out of its 5%; it's never added on top.");
    await expect(section).toContainText("Slop's VAT registration is still being set up.");
    await expect(section.getByRole("columnheader", { name: "VAT in the fee" })).toBeVisible();
    await expect(section.getByRole("row").filter({ hasText: "People jobs" })).toContainText("$0.07");
    await expect(section.getByRole("row").filter({ hasText: "People jobs" })).toContainText("simulated");
    const vat = section.getByRole("textbox", { name: /VAT number, if you're a VAT-registered business/ });
    await expect(vat).toHaveAttribute("placeholder", "US123456789");
    await vat.fill("12");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(section.getByRole("alert")).toHaveText("a VAT number starts with its country code, like DE123456789");
    await vat.fill("DE 123.456.789");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(section.getByRole("alert")).toHaveText("the VAT number has to be from the country on your account");
    // Empty clears it (she had none).
    await vat.fill("");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(section.getByText("Removed.")).toBeVisible();
    await expect(section.getByRole("alert")).toHaveCount(0);
  });

  test("a confirmed number is reverse-charged; an unconfirmed one says so", async ({ page }) => {
    let vat = { kind: "included", rate_bp: 1900, country: "DE", vat_number: "DE123456789", vat_number_checked: false, slop_vat_id: "EU372000041" };
    await tweak(page, "/v1/me/fee-statements", (s) => ({ ...s, vat }));
    const put = await fake(page, "PUT", "/v1/me/vat", () => {
      vat = { ...vat, kind: "reverse_charge", vat_number_checked: true };
      return { status: 200, body: { vat_number: "DE123456789", checked: true, note: null } };
    });
    await signIn(page, "maker");
    await page.goto("/account#fee-statements");
    const section = page.locator("#fee-statements");
    await expect(section).toContainText("The fee includes 19% VAT");
    await expect(section).toContainText("Slop's VAT number: EU372000041.");
    await expect(section).toContainText("Not confirmed yet, so VAT is shown as included in Slop's fee.");
    const box = section.getByRole("textbox", { name: /VAT number/ });
    await expect(box).toHaveValue("DE123456789");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(section).toContainText("Confirmed. Slop's fee is reverse-charged from now on.");
    await expect(section).toContainText("Reverse charge: you account for the VAT on Slop's fee yourself (VAT number DE123456789).");
    await expect(section.getByRole("row").nth(1)).toContainText("Reverse charge");
    expect(put.calls[0].body).toEqual({ vat_number: "DE123456789" });
  });

  test("a number the EU can't confirm comes back with the server's note", async ({ page }) => {
    await tweak(page, "/v1/me/fee-statements", (s) => ({
      ...s,
      vat: { kind: "included", rate_bp: 2100, country: "NL", vat_number: null, vat_number_checked: false, slop_vat_id: null },
    }));
    await fake(page, "PUT", "/v1/me/vat", {
      status: 200,
      body: { vat_number: "NL123456789B01", checked: false, note: "The EU's VIES service says this number isn't valid for cross-border trade." },
    });
    await signIn(page, "maker");
    await page.goto("/account#fee-statements");
    const section = page.locator("#fee-statements");
    await expect(section.getByRole("textbox", { name: /VAT number/ })).toHaveAttribute("placeholder", "NL123456789");
    await section.getByRole("textbox", { name: /VAT number/ }).fill("NL123456789B01");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(section).toContainText("The EU's VIES service says this number isn't valid for cross-border trade.");
  });

  test("a maker with nothing received has no statements", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account");
    await expect(page.getByRole("heading", { name: "Terms and policies" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Fee statements" })).toHaveCount(0);
  });
});
