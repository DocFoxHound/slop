// SPDX-License-Identifier: AGPL-3.0-or-later
// A maker's donations on the Manage page (web/src/components/DonationsManage.tsx): turning them
// on with Stripe, the settings, shutting the servers down, the totals and refunds. Vera (the
// seeded creative) makes her own game for it; Remy donates to it.
import type { Page, Route } from "@playwright/test";

import { expect, signIn, test } from "./fixtures";
import { api, call, donate, listedApp } from "./money-helpers";

const SLUG = "coin-garden";
const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

const panel = (page: Page) => page.locator("section#donations");

/** Vera's game, and her Stripe account connected for donations (through the API if a test before didn't). */
async function veraReceives() {
  await listedApp("creative", SLUG, "Coin Garden");
  const s = await api<{ account_status: string }>("creative", "GET", `/v1/apps/${SLUG}/donations/settings`);
  if (s.account_status !== "active") await api("creative", "POST", "/v1/me/donation-account", { tax_notice_ack: true });
}

/** The real settings, with some fields changed. */
const settingsAs = (change: Record<string, unknown>) => async (route: Route) => {
  if (route.request().method() !== "GET") return route.fallback();
  const res = await route.fetch();
  return route.fulfill({ response: res, json: { ...(await res.json()), ...change } });
};

test("turning donations on: the tax notice first, then the maker's Stripe account", async ({ page }) => {
  await listedApp("creative", SLUG, "Coin Garden");
  await signIn(page, "creative");
  await page.goto(`/publish/${SLUG}/manage#donations`);
  const p = panel(page);
  await expect(p.getByRole("heading", { name: "Donations" })).toBeVisible();
  await expect(p.getByText(/Slop takes a 5% platform fee/)).toBeVisible();
  // Vera said she's 18 when she joined People, so she isn't asked again.
  await expect(p.getByRole("checkbox", { name: "I'm 18 or older." })).toHaveCount(0);
  const enable = p.getByRole("button", { name: "Enable donations with Stripe" });
  await expect(enable).toBeDisabled();
  await p.getByRole("checkbox", { name: /donations may be taxable income for me/ }).check();
  await enable.click();
  // Her People Stripe account is already finished, so the settings show at once.
  await expect(p.getByRole("checkbox", { name: "Show the Donate button on the store page" })).toBeVisible();
  await expect(p.getByText("No donations yet.")).toBeVisible();
});

test("Stripe still checking, Stripe wanting more, a passkey, an adult tick, or payments off", async ({ page }) => {
  await veraReceives();
  await signIn(page, "creative");
  const p = panel(page);

  // Still signing up on Stripe: say so, and offer "I've finished".
  await page.route(`**/v1/apps/${SLUG}/donations/settings`, settingsAs({ account_status: "pending" }));
  await page.goto(`/publish/${SLUG}/manage#donations`);
  await expect(p.getByText("Stripe is still checking your account. Finish Stripe's sign-up, then come back.")).toBeVisible();
  await expect(p.getByRole("button", { name: "Continue on Stripe" })).toBeDisabled();
  const refreshed = page.waitForRequest((r) => r.url().endsWith("/v1/me/donation-account/refresh") && r.method() === "POST");
  await p.getByRole("button", { name: "I've finished on Stripe" }).click();
  await refreshed;
  await page.unroute(`**/v1/apps/${SLUG}/donations/settings`);

  await page.route(`**/v1/apps/${SLUG}/donations/settings`, settingsAs({ account_status: "restricted" }));
  await page.reload();
  await expect(p.getByText("Stripe needs more from you before donations can arrive. Open Stripe to fix it.")).toBeVisible();
  await page.unroute(`**/v1/apps/${SLUG}/donations/settings`);

  // Not connected yet, for someone who hasn't said they're 18: they're asked, and it's sent.
  await page.route(`**/v1/apps/${SLUG}/donations/settings`, settingsAs({ account_status: "none" }));
  await page.route("**/v1/me", async (route) => {
    const res = await route.fetch();
    return route.fulfill({ response: res, json: { ...(await res.json()), adult_confirmed: false } });
  });
  let sent: Record<string, unknown> | undefined;
  await page.route("**/v1/me/donation-account", (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    sent = route.request().postDataJSON();
    // No passkey or "confirm it's you" on the way (2026-10-25): what Stripe says is what shows.
    return route.fulfill(json(503, { error: { code: "unavailable", message: "Stripe didn't answer: try again in a minute" } }));
  });
  await page.reload();
  await p.getByRole("checkbox", { name: /donations may be taxable income for me/ }).check();
  await p.getByRole("checkbox", { name: "I'm 18 or older." }).check();
  await p.getByRole("button", { name: "Enable donations with Stripe" }).click();
  expect(sent).toEqual({ tax_notice_ack: true, confirm_adult: true, return_to: `/publish/${SLUG}/manage#donations` });
  await expect(p.getByRole("alert")).toHaveText("Stripe didn't answer: try again in a minute");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.unrouteAll();

  // Payments off on this server.
  await page.route(`**/v1/apps/${SLUG}/donations/settings`, settingsAs({ payments: "off" }));
  await page.reload();
  await expect(p.getByText("Payments aren't switched on on this server yet.")).toBeVisible();
});

test("the settings save, refuse bad values, and the servers can be shut down", async ({ browser }) => {
  await veraReceives();
  const page = await browser.newPage();
  await signIn(page, "creative");
  await page.goto(`/publish/${SLUG}/manage#donations`);
  const p = panel(page);
  const on = p.getByRole("checkbox", { name: "Show the Donate button on the store page" });
  const suggested = p.getByRole("textbox", { name: /Suggested amount/ });
  const servers = p.getByRole("checkbox", { name: "This app runs servers (offers a monthly server donation)" });
  const statement = p.getByRole("textbox", { name: /What the servers are/ });
  const save = p.getByRole("button", { name: "Save" });

  await on.check();
  await suggested.fill("0");
  await save.click();
  await expect(p.getByRole("alert")).toHaveText("from 1 to 500");
  await suggested.fill("4");
  await p.getByRole("combobox", { name: "Currency" }).selectOption("eur");
  await expect(statement).toHaveCount(0);
  await servers.check();
  await statement.fill("A server");
  await save.click();
  await expect(p.getByRole("alert")).toHaveText(
    "say what the servers are, and what donations help pay for (10 to 300 characters)",
  );
  await statement.fill("Runs the garden's shared leaderboard.");
  await save.click();
  await expect(p.getByText("Saved.")).toBeVisible();
  await expect(p.getByRole("alert")).toHaveCount(0);
  await page.reload();
  await expect(on).toBeChecked();
  await expect(suggested).toHaveValue("4.00");
  await expect(p.getByRole("combobox", { name: "Currency" })).toHaveValue("eur");
  await expect(statement).toHaveValue("Runs the garden's shared leaderboard.");

  // Players see it, in euros, with the monthly choice.
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${SLUG}`);
  const box = visitor.getByRole("form", { name: "Donate to Coin Garden" });
  await expect(box.getByRole("radio").first()).toContainText("€");
  await box.getByRole("tab", { name: "Server donation · monthly" }).click();
  await expect(box.getByText("“Runs the garden's shared leaderboard.”")).toBeVisible();

  // Turned off, the button goes from the store page.
  await on.uncheck();
  await save.click();
  await expect(p.getByText("Saved.")).toBeVisible();
  await visitor.reload();
  await expect(visitor.getByRole("heading", { name: "Coin Garden", level: 1 })).toBeVisible();
  await expect(box).toHaveCount(0);
  await on.check();
  await save.click();
  await expect(p.getByText("Saved.")).toBeVisible();

  // Someone gives monthly: the servers can't just be unticked now; shutting them down ends it.
  await donate("pendingCreative", SLUG, { amount_minor: 400, monthly: true, consent: true });
  await page.reload();
  await expect(servers).toBeDisabled();
  await p.getByRole("button", { name: "The servers are shut down…" }).click();
  await expect(p.getByText(/This cancels every monthly server donation to this app and emails the donors\./)).toBeVisible();
  await p.getByRole("button", { name: "Cancel" }).click();
  await expect(p.getByRole("button", { name: "Yes, the servers are shut down" })).toHaveCount(0);
  await p.getByRole("button", { name: "The servers are shut down…" }).click();
  const told = page.waitForEvent("dialog");
  await p.getByRole("button", { name: "Yes, the servers are shut down" }).click();
  const alert = await told;
  expect(alert.message()).toBe("Done. 1 server donation was cancelled, and the donors told.");
  await alert.accept();
  await expect(servers).not.toBeChecked();
  await expect(servers).toBeEnabled();
  await expect(p.getByRole("button", { name: "The servers are shut down…" })).toHaveCount(0);
  // Remy's server donation is cancelled, and says why.
  const remy = await browser.newPage();
  await signIn(remy, "pendingCreative");
  await remy.goto("/account#donations");
  await expect(
    remy.getByRole("listitem").filter({ hasText: "Server donation to Coin Garden" }).getByText(/\(the servers shut down\)/),
  ).toBeVisible();
});

test("the totals, the donors who shared their name, and refunding a mistake", async ({ page }) => {
  await veraReceives();
  const s = await api("creative", "GET", `/v1/apps/${SLUG}/donations/settings`);
  if (!s.donations_on) {
    await api("creative", "PUT", `/v1/apps/${SLUG}/donations/settings`, {
      donations_on: true,
      donation_suggested_minor: 400,
      donation_currency: "eur",
      runs_servers: false,
      servers_statement: null,
    });
  }
  await donate("pendingCreative", SLUG, { amount_minor: 600, share_name: true, message: "Lovely garden." });
  await signIn(page, "creative");
  await page.goto(`/publish/${SLUG}/manage#donations`);
  const p = panel(page);
  await expect(p.getByRole("heading", { name: "So far" })).toBeVisible();
  await expect(p.getByText(/to you from \d+ donations?/)).toBeVisible();
  await expect(p.getByText(/Monthly breakdowns and server-donor counts show from 10 donors up/)).toBeVisible();
  const named = p.locator(".shared-list li").filter({ hasText: "Remy Pending" });
  await expect(named.getByText("“Lovely garden.”")).toBeVisible();
  await expect(named).toContainText("€6.00");
  await expect(p.getByText("Only for you. Never show donors publicly, and never give them anything for donating.")).toBeVisible();

  await p.getByText("Refund a mistake").click();
  const refundable = p.locator("details li");
  const before = await refundable.count();
  const item = refundable.filter({ hasText: /^€[\d.]+ on / }).first();
  // Asked first; "Cancel" leaves it.
  page.once("dialog", (d) => {
    expect(d.message()).toBe("Refund this donation in full, from your Stripe account?");
    void d.dismiss();
  });
  await item.getByRole("button", { name: "Refund" }).click();
  await expect(refundable).toHaveCount(before);
  // Stripe or Slop refusing shows why.
  await page.route("**/v1/donations/*/refund", (route) =>
    route.fulfill(json(409, { error: { code: "conflict", message: "Stripe says this charge was already refunded" } })),
  );
  page.once("dialog", (d) => void d.accept());
  await item.getByRole("button", { name: "Refund" }).click();
  await expect(p.getByRole("alert")).toHaveText("Stripe says this charge was already refunded");
  await page.unroute("**/v1/donations/*/refund");
  page.once("dialog", (d) => void d.accept());
  await item.getByRole("button", { name: "Refund" }).click();
  await expect(p.getByText(/€[\d.]+ refunded\)/)).toBeVisible();
  if (before === 1) await expect(p.getByText("Refund a mistake")).toHaveCount(0);
  else await expect(refundable).toHaveCount(before - 1);

  // The refund is the donor's to see too.
  const res = await call("pendingCreative", "GET", "/v1/me/donations");
  expect(res.body.donations.some((d: { app_slug: string; status: string }) => d.app_slug === SLUG && d.status === "refunded")).toBe(
    true,
  );
});
