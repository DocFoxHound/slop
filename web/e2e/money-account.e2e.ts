// SPDX-License-Identifier: AGPL-3.0-or-later
// Money on the account page (web/src/components/AccountSections.tsx): fee statements and the VAT
// number, and switching or disconnecting the Stripe account donations are paid to. (Cancelling a
// server donation is in money-donate.e2e.ts, with the one it makes.) Mia's real Stripe account is
// never changed: requests that would start its 72-hour switch are answered in the browser.
import type { Route } from "@playwright/test";

import { expect, signIn, test } from "./fixtures";

const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

test("fee statements show SLAPorSLOP's fee by month, and the VAT in it where there is VAT", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/account#fee-statements");
  const fees = page.locator("section#fee-statements");
  await expect(fees.getByRole("heading", { name: "Fee statements" })).toBeVisible();
  // Mia is in the US: no VAT, so no VAT column or number.
  await expect(fees.getByText(/There's no VAT on SLAPorSLOP's fee where you are\./)).toBeVisible();
  const row = fees.getByRole("row").filter({ hasText: "Donations" }).first();
  await expect(row.getByRole("cell").nth(0)).toHaveText(/^\d{4}-\d{2}$/);
  await expect(row.getByRole("cell").nth(4)).toHaveText(/^\$\d+\.\d\d$/);
  await expect(fees.getByRole("columnheader", { name: "VAT in the fee" })).toHaveCount(0);
  await expect(fees.getByRole("textbox", { name: /VAT number/ })).toHaveCount(0);

  // Somewhere with VAT (as a German maker would see it): the VAT inside the fee, and a VAT number.
  let saved: string | null = null;
  await page.route("**/v1/me/fee-statements", async (route: Route) => {
    const res = await route.fetch();
    const real = await res.json();
    return route.fulfill({
      response: res,
      json: {
        statements: real.statements.map((s: { slop_fee_minor: number }) => ({ ...s, vat_minor: Math.round(s.slop_fee_minor / 6) })),
        vat: { kind: "included", rate_bp: 1900, country: "DE", vat_number: saved, vat_number_checked: false, slop_vat_id: null },
      },
    });
  });
  await page.reload();
  await expect(fees.getByText(/The fee includes 19% VAT, which SLAPorSLOP pays out of its 5%; it's never added on top\./)).toBeVisible();
  await expect(fees.getByText(/SLAPorSLOP's VAT registration is still being set up\./)).toBeVisible();
  await expect(fees.getByRole("columnheader", { name: "VAT in the fee" })).toBeVisible();
  const vat = fees.getByRole("textbox", { name: /VAT number, if you're a VAT-registered business/ });
  await expect(vat).toHaveAttribute("placeholder", "DE123456789");
  // SLAPorSLOP checks the number against the account's country (Mia's account is really in the US).
  await vat.fill("DE123456789");
  await fees.getByRole("button", { name: "Save" }).click();
  await expect(fees.getByRole("alert")).toHaveText("the VAT number has to be from the country on your account");
  // Saved but not confirmed yet: said so.
  await page.route("**/v1/me/vat", (route) =>
    route.fulfill(
      json(200, {
        vat_number: "DE123456789",
        checked: false,
        note: "Saved, but the EU's VIES service didn't answer. SLAPorSLOP will check it again later; until then VAT stays included in SLAPorSLOP's fee.",
      }),
    ),
  );
  await fees.getByRole("button", { name: "Save" }).click();
  await expect(fees.getByText(/Saved, but the EU's VIES service didn't answer\./)).toBeVisible();
  await expect(fees.getByRole("alert")).toHaveCount(0);
  // Later visits say it isn't confirmed yet.
  saved = "DE123456789";
  await page.reload();
  await expect(vat).toHaveValue("DE123456789");
  await expect(fees.getByText("Not confirmed yet, so VAT is shown as included in SLAPorSLOP's fee.")).toBeVisible();
});

test("switching or disconnecting the Stripe account asks first, needs a fresh sign-in, and can be cancelled", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/account#receiving");
  const section = page.locator("section#receiving");
  await expect(section.getByRole("heading", { name: "Receiving donations" })).toBeVisible();
  await expect(section.getByText(/Donations to your apps go to your own Stripe account\. Bank details are changed in Stripe/)).toBeVisible();

  // Each asks first, and "Never mind" backs out.
  await section.getByRole("button", { name: "Use a different Stripe account" }).click();
  await expect(section.getByText(/new donations start going to it 72 hours from now/)).toBeVisible();
  await section.getByRole("button", { name: "Never mind" }).click();
  await section.getByRole("button", { name: "Disconnect Stripe" }).click();
  await expect(section.getByText(/In 72 hours donations turn off on all your apps and monthly server donations end\./)).toBeVisible();

  // A refusal says why (there's no "confirm it's you" any more: SECURITY §4.3).
  let asked: unknown;
  await page.route("**/v1/me/donation-account/change", (route) => {
    asked = route.request().postDataJSON();
    return route.fulfill(json(409, { error: { code: "conflict", message: "a change is already waiting: cancel it first" } }));
  });
  await section.getByRole("button", { name: "Disconnect in 72 hours" }).click();
  expect(asked).toEqual({ kind: "disconnect", return_to: "/account/money#receiving" });
  await expect(section.getByRole("alert")).toHaveText("a change is already waiting: cancel it first");

  // Once asked for, the change waits, says when it happens, and can be cancelled.
  const change = {
    kind: "switch",
    requested_at: "2026-10-20T10:00:00Z",
    effective_at: "2026-10-23T10:00:00Z",
    new_account_ready: false,
  };
  let pending: typeof change | null = change;
  await page.route("**/v1/me/donation-account", (route) =>
    route.request().method() === "GET"
      ? route.fulfill(json(200, { status: "active", payments: "stripe", change: pending }))
      : route.fallback(),
  );
  let cancelled = false;
  await page.route("**/v1/me/donation-account/change/cancel", (route) => {
    cancelled = true;
    pending = null;
    return route.fulfill({ status: 204 });
  });
  await page.reload();
  const notice = section.getByRole("status");
  await expect(notice).toContainText("Switching to a new Stripe account, asked");
  await expect(notice).toContainText("once you've finished the new account's sign-up on Stripe");
  await expect(notice.getByRole("button", { name: "I've finished on Stripe" })).toBeVisible();
  await notice.getByRole("button", { name: "Cancel the change" }).click();
  expect(cancelled).toBe(true);
  await expect(notice).toHaveCount(0);
  await expect(section.getByRole("button", { name: "Use a different Stripe account" })).toBeVisible();

  pending = { ...change, kind: "disconnect", new_account_ready: null as unknown as boolean };
  await page.reload();
  await expect(notice).toContainText("Disconnecting your Stripe account");
  await expect(notice).toContainText("Donations then turn off on all your apps, and monthly server donations end.");
  await expect(notice.getByRole("button", { name: "I've finished on Stripe" })).toHaveCount(0);
});

test("someone who receives nothing has no receiving or fee sections", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/account/money");
  await expect(page.getByRole("heading", { name: "Your donations" })).toBeVisible();
  await expect(page.locator("section#receiving")).toHaveCount(0);
  await expect(page.locator("section#fee-statements")).toHaveCount(0);
  // No apps: nothing to set donations up for.
  await expect(page.locator("section#donations-setup")).toHaveCount(0);
});

test("a maker with apps but no Stripe account is shown where to turn donations on", async ({ page }) => {
  await signIn(page, "maker");
  // Mia's real Stripe account is connected: no pointer.
  await page.goto("/account/money");
  await expect(page.getByRole("heading", { name: "Receiving donations" })).toBeVisible();
  await expect(page.locator("section#donations-setup")).toHaveCount(0);

  // As before she connected one, with two of her apps (Lantern Keep already takes donations).
  await page.route("**/v1/me/donation-account", (route) =>
    route.request().method() === "GET" ? route.fulfill(json(200, { status: "none", payments: "stripe", change: null })) : route.fallback(),
  );
  await page.route("**/v1/me/apps", async (route) => {
    const res = await route.fetch();
    const apps = (await res.json()) as Array<{ slug: string }>;
    return route.fulfill({ response: res, json: apps.filter((a) => a.slug === "mias-first" || a.slug === "lantern-keep") });
  });
  await page.reload();
  const hint = page.getByRole("region", { name: "Want donations?" });
  await expect(hint).toContainText("Turn them on per app: Publish → your app → Manage → Donations.");
  await expect(hint).toContainText("Stripe's own sign-up (you need to be 18 or older)");
  await expect(hint).toContainText("SLAPorSLOP's fee is 5%, and donations never unlock anything.");
  await expect(hint.getByRole("listitem")).toHaveCount(1);
  await expect(page.locator("section#receiving")).toHaveCount(0);
  const link = hint.getByRole("link", { name: "Set up donations for Mia's First Game" });
  await expect(link).toHaveAttribute("href", "/publish/mias-first/manage#donations");
  // The link lands on the app's Donations panel.
  await link.click();
  await expect(page).toHaveURL(/\/publish\/mias-first\/manage#donations$/);
  const panel = page.locator("section#donations");
  await expect(panel.getByRole("heading", { name: "Donations" })).toBeVisible();
  await expect(panel).toBeInViewport();
});
