// SPDX-License-Identifier: AGPL-3.0-or-later
// The Donate button next to Add to library / Play on an app's page (web/src/components/Donate.tsx)
// and the form it opens: green for someone who can donate, grey once they have, a monthly server
// donation with Cancel right there; quotes, who can't donate, one-off and monthly server donations
// through (fake) Stripe Checkout, which opens in a new tab, the shared two-payment donation to a
// community build, and the proof-of-work check. Each donor is a staff account that donates
// nowhere else, so their per-hour limits and checks are theirs.
import type { Page } from "@playwright/test";

import { fake } from "./accounts-helpers";
import { donateButton, donatePanel, expectSameRow, noSeriousAxeViolations, openDonateForm, SLIME } from "./donate-helpers";
import { expect, ids, signIn, test } from "./fixtures";
import { api, call, moneyApps } from "./money-helpers";

interface Quote {
  donation: number;
  total: number;
  slop_fee: number;
  processing: number;
  maker_receives: number;
  donor_covers_fees: boolean;
}

/** As the donate form writes amounts: always with cents. */
const amount = (minor: number, _currency: "usd") => `$${(minor / 100).toFixed(2)}`;

const donateBox = (page: Page) => page.getByRole("form", { name: /^Donate to/ });

/** Clicks the Donate button and pays on the fake Stripe page it opens in a new tab. */
async function payInNewTab(page: Page, button: string | RegExp): Promise<Page> {
  const popup = page.waitForEvent("popup");
  await page.getByRole("button", { name: button }).click();
  const stripe = await popup;
  await expect(stripe.getByRole("heading", { name: "Fake Stripe Checkout" })).toBeVisible({ timeout: 20_000 });
  await stripe.getByRole("button", { name: "Pay" }).click();
  return stripe;
}

test("signed out, on your own app, coming soon or with donations off, there's no way to donate, and it says why", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${ids.listedApp}`);
  // Green like Play, in the Get it box; it opens the amounts, with Sign in to donate.
  await expect(donateButton(visitor)).toHaveAccessibleName("Donate");
  await expect(donateButton(visitor)).toHaveCSS("background-color", SLIME);
  const box = await openDonateForm(visitor);
  await expect(box.getByRole("heading", { name: "♥ Donate" })).toBeVisible();
  await expect(box.getByRole("radiogroup", { name: "Amount" }).getByRole("radio")).not.toHaveCount(0);
  await expect(box.getByRole("link", { name: "Sign in to donate" })).toHaveAttribute(
    "href",
    `/signin?return_to=/app/${ids.listedApp}`,
  );
  await expect(box.getByRole("button", { name: /^Donate|Choose an amount/ })).toHaveCount(0);

  // The maker's own: quiet, not green, and the form says why.
  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/app/${ids.listedApp}`);
  await expect(donateButton(mia)).not.toHaveCSS("background-color", SLIME);
  await expect(mia.locator(".install-box").getByText("Donations on")).toBeVisible();
  await openDonateForm(mia);
  await expect(donateBox(mia).getByText("It's your own app.")).toBeVisible();
  await expect(donateBox(mia).getByRole("button", { name: /^Donate|Choose an amount/ })).toHaveCount(0);
  // Coming soon: no Donate at all.
  await mia.goto(`/app/${ids.draftApp}`);
  await expect(mia.getByRole("heading", { name: "Starship Draft", level: 1 })).toBeVisible();
  await expect(donateButton(mia)).toHaveCount(0);
  // The maker hasn't turned donations on: the button stays, quiet, and says why beside it, on
  // hover or focus, and in its panel.
  await visitor.goto("/app/castle-rock");
  await expect(visitor.getByRole("heading", { name: "Castle Rock", level: 1 })).toBeVisible();
  const quiet = donateButton(visitor);
  await expect(quiet).toHaveAccessibleName("Donate");
  await expect(quiet).not.toHaveCSS("background-color", SLIME);
  await expect(visitor.locator(".install-box").getByText("Donations off")).toBeVisible();
  await expect(quiet).toHaveAccessibleDescription(/^Donations are off\. (The maker hasn't turned on donations\.|The maker's Stripe account isn't ready)/);
  await quiet.focus();
  await expect(visitor.getByRole("tooltip")).toBeVisible();
  await quiet.click();
  const off = donatePanel(visitor);
  await expect(off.getByText("Donations are off.")).toBeVisible();
  await expect(off).toContainText(/The maker hasn't turned on donations\.|The maker's Stripe account isn't ready/);
  await expect(donateBox(visitor)).toHaveCount(0);
  await expect(off.getByRole("button")).toHaveAccessibleName("Close");
});

test("next to Add to library: green for someone who can donate, grey once they have, and a monthly one with Cancel", async ({ browser }) => {
  // Rae never donates to Lantern Keep: green, normal size, beside the library button.
  const rae = await (await browser.newContext()).newPage();
  await signIn(rae, "reviewer");
  await rae.goto(`/app/${ids.listedApp}`);
  const green = donateButton(rae);
  await expect(green).toHaveAccessibleName("Donate");
  await expect(green).toHaveCSS("background-color", SLIME);
  await expect(green).not.toHaveClass(/\bbig\b/);
  await expectSameRow(green, rae.locator(".install-box").getByRole("button", { name: /library/ }));
  await green.click();
  await expect(green).toHaveAttribute("aria-expanded", "true");
  await expect(donatePanel(rae)).toBeFocused();
  await expect(donateBox(rae)).toBeVisible();
  await noSeriousAxeViolations(rae);
  await rae.keyboard.press("Escape");
  await expect(donatePanel(rae)).toHaveCount(0);
  await expect(green).toBeFocused();

  // Pat has donated: grey "Donated", which says what and when, with Donate again.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${ids.listedApp}`);
  const given = donateButton(pat);
  await expect(given).toHaveAccessibleName("Donated");
  await expect(given).not.toHaveCSS("background-color", SLIME);
  await given.click();
  await expect(donatePanel(pat).getByText(/^You donated \$[\d.]+ on /)).toBeVisible();
  await expect(donatePanel(pat).getByRole("button", { name: "Donate again" })).toBeVisible();

  // Cleo gives monthly: the amount, grey, and Cancel right beside it, which asks first. (Her real
  // server donation stays: the cancel is answered here.)
  const cleo = await (await browser.newContext()).newPage();
  await signIn(cleo, "client");
  let cancelled = false;
  await cleo.route(
    (u) => u.pathname === `/v1/apps/${ids.listedApp}/donate`,
    async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      const res = await route.fetch();
      const body = await res.json();
      return route.fulfill({ response: res, json: cancelled ? { ...body, mine: { ...body.mine, monthly: null } } : body });
    },
  );
  const cancel = await fake(cleo, "POST", `/v1/me/server-donations/${ids.serverDonation}/cancel`, () => {
    cancelled = true;
    return { status: 204 };
  });
  await cleo.goto(`/app/${ids.listedApp}`);
  const monthly = cleo.locator(".install-box").getByRole("button", { name: /^\$[\d.]+\/month server donation$/ });
  await expect(monthly).toBeVisible();
  await expect(monthly).not.toHaveCSS("background-color", SLIME);
  const stop = cleo.locator(".install-box").getByRole("button", { name: "Cancel server donation" });
  await expectSameRow(monthly, stop);
  await stop.click();
  const sure = cleo.getByRole("group", { name: "Cancel your server donation" });
  await expect(sure).toContainText(/^Cancel your server donation of \$[\d.]+ a month to Lantern Keep\? It ends now: no more payments are taken/);
  await expect(sure).toBeFocused();
  await noSeriousAxeViolations(cleo);
  await sure.getByRole("button", { name: "Keep it" }).click();
  await expect(sure).toHaveCount(0);
  await expect(stop).toBeFocused();
  expect(cancel.calls).toHaveLength(0);
  await stop.click();
  await sure.getByRole("button", { name: "Yes, cancel it" }).click();
  await expect(cleo.getByRole("status").filter({ hasText: "Your server donation to Lantern Keep is cancelled." })).toBeVisible();
  expect(cancel.calls).toHaveLength(1);
  await expect(stop).toHaveCount(0);
  await expect(monthly).toHaveCount(0);
});

test("presets and your own amount show what you pay, what the maker gets and the fees", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(`/app/${ids.listedApp}`);
  const box = await openDonateForm(page);
  const presets = box.getByRole("radiogroup", { name: "Amount" }).getByRole("radio");
  await expect(box.getByRole("button", { name: "Choose an amount" })).toBeDisabled();

  // A preset shows the total first, then what Mia gets; choosing it quotes the rest.
  const info = await api<{ presets: Quote[] }>("player", "GET", `/v1/apps/${ids.listedApp}/donate`);
  const second = info.presets[1];
  await expect(presets.nth(1)).toHaveText(`${amount(second.total, "usd")}Mia Maker gets ${amount(second.maker_receives, "usd")}`);
  await presets.nth(1).click();
  await expect(presets.nth(1)).toHaveAttribute("aria-checked", "true");
  const quoteLine = (q: Quote) =>
    `You pay ${amount(q.total, "usd")} · Mia Maker receives ${amount(q.maker_receives, "usd")} · ${amount(q.slop_fee, "usd")} Slop fee · ` +
    (q.donor_covers_fees
      ? `${amount(q.processing, "usd")} card processing`
      : `about ${amount(q.processing, "usd")} card processing comes out of what they receive`);
  await expect(box.getByText(/^You pay/)).toHaveText(quoteLine(second));
  await expect(box.getByRole("button", { name: `Donate ${amount(second.total, "usd")}` })).toBeEnabled();

  // Your own amount replaces the preset.
  const custom = box.getByRole("textbox", { name: "Or your own amount for Mia Maker ($1.00 to $500.00)" });
  await custom.fill("7.50");
  await expect(presets.nth(1)).toHaveAttribute("aria-checked", "false");
  const q750 = await api<Quote>("player", "GET", `/v1/apps/${ids.listedApp}/donate/quote?amount_minor=750`);
  await expect(box.getByText(/^You pay/)).toHaveText(quoteLine(q750));
  await expect(box.getByRole("button", { name: `Donate ${amount(q750.total, "usd")}` })).toBeEnabled();
  // Not an amount, or outside $1 to $500: no quote, and nothing to press.
  for (const bad of ["abc", "0.50", "600"]) {
    await custom.fill(bad);
    await expect(box.getByText(/^You pay/)).toHaveCount(0);
    await expect(box.getByRole("button", { name: "Choose an amount" })).toBeDisabled();
  }

  // The name (and a message) reach the maker only when ticked.
  const message = box.getByRole("textbox", { name: /A message for them/ });
  await expect(message).toHaveCount(0);
  await box.getByRole("checkbox", { name: /Tell Mia Maker my name/ }).check();
  await expect(message).toBeVisible();
  await box.getByRole("checkbox", { name: /Tell Mia Maker my name/ }).uncheck();
  await expect(message).toHaveCount(0);
  // A one-off app without servers has no monthly choice.
  await expect(box.getByRole("tablist", { name: "How often" })).toBeVisible();
  const { original } = await moneyApps();
  await page.goto(`/app/${original}`);
  await openDonateForm(page);
  await expect(donateBox(page).getByRole("heading", { name: "♥ Donate" })).toBeVisible();
  await expect(donateBox(page).getByRole("tablist", { name: "How often" })).toHaveCount(0);
});

test("someone who hasn't said they're 18 has to tick it first", async ({ page }) => {
  // Olly never has; the box asks, and the button waits for the tick (it isn't sent here, so he
  // stays unconfirmed for the other tests).
  await signIn(page, "overdue");
  await page.goto(`/app/${ids.listedApp}`);
  const box = await openDonateForm(page);
  await box.getByRole("radiogroup", { name: "Amount" }).getByRole("radio").first().click();
  const donate = box.getByRole("button", { name: /^Donate \$/ });
  await expect(donate).toBeDisabled();
  await box.getByRole("checkbox", { name: "I'm 18 or older." }).check();
  await expect(donate).toBeEnabled();
  // The under-18 view (Tia, in the UK) is asked the same: the box takes the donor's word.
  const teen = await page.context().browser()!.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${ids.listedApp}`);
  await openDonateForm(teen);
  await donateBox(teen).getByRole("radiogroup", { name: "Amount" }).getByRole("radio").first().click();
  await expect(donateBox(teen).getByRole("checkbox", { name: "I'm 18 or older." })).not.toBeChecked();
  await expect(donateBox(teen).getByRole("button", { name: /^Donate \$/ })).toBeDisabled();
  // The server refuses without the tick too.
  const res = await call("overdue", "POST", `/v1/apps/${ids.listedApp}/donate`, { amount_minor: 500 });
  expect(res.status).toBe(422);
  expect(res.body.error.message).toBe("donating is for adults: confirm you're 18 or older");
});

test("a one-off donation goes through Stripe in a new tab, says thanks, and is on the donor's account page", async ({ page }) => {
  const { original } = await moneyApps();
  await signIn(page, "support");
  await page.goto(`/app/${original}`);
  const box = await openDonateForm(page);
  await box.getByRole("textbox", { name: /Or your own amount/ }).fill("4");
  await box.getByRole("checkbox", { name: /Tell Mia Maker my name/ }).check();
  await box.getByRole("textbox", { name: /A message for them/ }).fill("For the shiny coins.");
  const q = await api<Quote>("support", "GET", `/v1/apps/${original}/donate/quote?amount_minor=400`);
  const stripe = await payInNewTab(page, `Donate ${amount(q.total, "usd")}`);
  // Stripe sends the new tab back to the app, which checks the payment and says thanks.
  await expect(stripe.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();
  await expect(stripe.getByText("Your donation goes straight to Mia Maker.")).toBeVisible();
  await expect(stripe).toHaveURL(new RegExp(`/app/${original}$`));
  // The button beside Add to library goes grey.
  await expect(donateButton(stripe)).toHaveAccessibleName("Donated");
  await expect(donateButton(stripe)).not.toHaveCSS("background-color", SLIME);

  await stripe.goto("/account#donations");
  const row = stripe.getByRole("row").filter({ has: stripe.getByRole("link", { name: "Tip Jar Tycoon" }) });
  await expect(row.getByRole("cell", { name: amount(q.total, "usd") })).toBeVisible();
  await expect(row.getByRole("cell", { name: "Paid" })).toBeVisible();

  // The maker sees the name and the message.
  await signIn(page, "maker");
  await page.goto(`/publish/${original}/manage#donations`);
  const shared = page.locator(".shared-list li").filter({ hasText: "Sue Support" });
  await expect(shared.getByText("“For the shiny coins.”")).toBeVisible();
});

test("back from Stripe, the thanks show even when checking the payment fails", async ({ page }) => {
  await signIn(page, "player");
  await page.route("**/v1/me/donations/*/check", (route) =>
    route.fulfill({ status: 500, contentType: "application/json", body: '{"error":{"code":"internal","message":"no"}}' }),
  );
  await page.goto(`/app/${ids.listedApp}?donated=${ids.donation}`);
  await expect(donateBox(page)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/app/${ids.listedApp}$`));
  await page.getByRole("link", { name: "account page" }).click();
  await expect(page).toHaveURL(/\/account\/money#donations$/);
});

/** Ends Mo's monthly server donation to Lantern Keep, if a run before left one going. */
async function endMonthly() {
  const mine = await api<{ server_donations: Array<{ id: string; app_slug: string; status: string }> }>("moderator", "GET", "/v1/me/donations");
  for (const s of mine.server_donations) {
    if (s.app_slug === ids.listedApp && s.status !== "cancelled") {
      await call("moderator", "POST", `/v1/me/server-donations/${s.id}/cancel`);
    }
  }
}

test("a monthly server donation states its terms, needs a fresh tick for each amount, and is cancelled next to the button in two clicks", async ({ page }) => {
  await endMonthly();
  try {
    await signIn(page, "moderator");
    await page.goto(`/app/${ids.listedApp}`);
    const box = await openDonateForm(page);
    await expect(box.getByRole("tab", { name: "Once" })).toHaveAttribute("aria-selected", "true");
    await expect(box.getByRole("checkbox", { name: /Tell Mia Maker my name/ })).toBeVisible();
    await box.getByRole("tab", { name: "Server donation · monthly" }).click();
    await expect(box.getByText("“Pays for two game servers and the leaderboard.”")).toBeVisible();
    // Names aren't shared on monthly donations.
    await expect(box.getByRole("checkbox", { name: /Tell Mia Maker my name/ })).toHaveCount(0);
    const presets = box.getByRole("radiogroup", { name: "Amount" }).getByRole("radio");
    await presets.first().click();
    await expect(box.getByText(/^You pay .* every month/)).toBeVisible();
    const consent = box.locator("label.consent").getByRole("checkbox");
    await expect(box.locator("label.consent")).toContainText("Lantern Keep");
    await expect(box.locator("label.consent")).toContainText("I can cancel at any time in two clicks from my library or account page");
    const donate = box.getByRole("button", { name: /^Donate \$[\d.]+ a month$/ });
    await expect(donate).toBeDisabled();
    await consent.check();
    await expect(donate).toBeEnabled();
    // Other terms, another tick.
    await presets.nth(1).click();
    await expect(consent).not.toBeChecked();
    await expect(donate).toBeDisabled();
    await consent.check();
    const stripe = await payInNewTab(page, /^Donate \$[\d.]+ a month$/);
    await expect(stripe.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();

    // Beside Add to library: the amount a month, grey, and Cancel. Two clicks; Keep it backs out.
    const monthly = stripe.locator(".install-box").getByRole("button", { name: /^\$[\d.]+\/month server donation$/ });
    await expect(monthly).toBeVisible();
    await expect(monthly).not.toHaveCSS("background-color", SLIME);
    const stop = stripe.locator(".install-box").getByRole("button", { name: "Cancel server donation" });
    await stop.click();
    const sure = stripe.getByRole("group", { name: "Cancel your server donation" });
    await expect(sure).toContainText(/It ends now: no more payments are taken, including the one due /);
    await sure.getByRole("button", { name: "Keep it" }).click();
    await expect(sure.getByRole("button", { name: "Yes, cancel it" })).toHaveCount(0);
    await stop.click();
    await sure.getByRole("button", { name: "Yes, cancel it" }).click();
    await expect(stripe.getByRole("status").filter({ hasText: "Your server donation to Lantern Keep is cancelled." })).toBeVisible();
    await expect(stop).toHaveCount(0);
    // The month paid counts as given.
    await expect(donateButton(stripe)).toHaveAccessibleName("Donated");

    // The account page has it, with what was agreed to, cancelled.
    await stripe.goto("/account#donations");
    const item = stripe.getByRole("listitem").filter({ hasText: "Server donation to Lantern Keep" }).first();
    await item.getByText("What you agreed to").click();
    await expect(item.getByText(/Lantern Keep/).last()).toBeVisible();
    await expect(item.getByText(/^Cancelled /)).toBeVisible();
    await expect(item.getByRole("button", { name: "Cancel" })).toHaveCount(0);
  } finally {
    await endMonthly();
  }
});

test("a donation to a community build is two payments: the original's share first, then the build's", async ({ page }) => {
  const { original, build } = await moneyApps();
  await signIn(page, "reviewer");
  await page.goto(`/app/${build}`);
  const box = await openDonateForm(page);
  const custom = box.getByRole("textbox", { name: /Or your own amount for Bo Builder/ });
  // Too small: the original's 30% would be under the $1 minimum.
  await custom.fill("1");
  await expect(box.getByText("Shared donations start at $2.34, so Mia Maker's share is at least $1.00.")).toBeVisible();
  await expect(box.getByRole("button", { name: /^Donate/ })).toBeDisabled();
  await custom.fill("5");
  const share = await api<Quote>("reviewer", "GET", `/v1/apps/${original}/donate/quote?amount_minor=215`);
  const rest = await api<Quote>("reviewer", "GET", `/v1/apps/${build}/donate/quote?amount_minor=500`);
  await expect(box.getByText(/^Shared with the original:/)).toHaveText(
    `Shared with the original: ${amount(share.total, "usd")} goes first to Mia Maker for Tip Jar Tycoon (30% of your donations), then the amount below to Bo Builder. That's two payments, each straight to its maker.`,
  );
  const stripe = await payInNewTab(page, `Donate ${amount(share.total + rest.total, "usd")} in two payments`);
  // Back from the first payment: step 2.
  await expect(stripe.getByRole("heading", { name: "♥ Step 2 of 2" })).toBeVisible();
  await expect(stripe.getByText("Thank you: your share for Tip Jar Tycoon is paid. Now your donation of $5.00 to Bo Builder.")).toBeVisible();
  await expect(stripe).toHaveURL(new RegExp(`/app/${build}$`));
  const second = await payInNewTab(stripe, "Donate $5.00 to Bo Builder");
  await expect(second.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();

  await second.goto("/account#donations");
  for (const title of ["Tip Jar Tycoon", "Tip Jar Tycoon Plus"]) {
    const row = second.getByRole("row").filter({ has: second.getByRole("link", { name: title, exact: true }) });
    await expect(row.getByRole("cell", { name: "Paid" })).toBeVisible();
  }
});

test("where payments are simulated, both payments of a shared donation happen at once and it says thanks", async ({ page }) => {
  const { original, build } = await moneyApps();
  await signIn(page, "reviewer");
  // This server runs on (fake) Stripe; a simulating one answers like this.
  await page.route(`**/v1/apps/${build}/donate`, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const res = await route.fetch();
    return route.fulfill({ response: res, json: { ...(await res.json()), simulated: true } });
  });
  const started: Array<{ url: string; body: Record<string, unknown> }> = [];
  await page.route("**/v1/apps/*/donate", (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    started.push({ url: route.request().url(), body: route.request().postDataJSON() });
    const id = `00000000-0000-4000-8000-00000000000${started.length}`;
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ url: null, id, done: true }) });
  });
  await page.goto(`/app/${build}`);
  const box = await openDonateForm(page);
  await expect(box.getByText("This server simulates payments: no money moves.")).toBeVisible();
  await box.getByRole("textbox", { name: /Or your own amount/ }).fill("5");
  // Once it's done, the button reloads what Rae gave (through the stand-ins above).
  const reloaded = page.waitForResponse((r) => new URL(r.url()).pathname === `/v1/apps/${build}/donate` && r.request().method() === "GET");
  await box.getByRole("button", { name: /in two payments$/ }).click();
  await expect(page.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();
  await reloaded;
  expect(started.map((s) => new URL(s.url).pathname)).toEqual([`/v1/apps/${original}/donate`, `/v1/apps/${build}/donate`]);
  expect(started[0].body).toMatchObject({ amount_minor: 215, share_for: build, rest_minor: 500, monthly: false });
  expect(started[1].body).toMatchObject({ amount_minor: 500, share_donation: "00000000-0000-4000-8000-000000000001" });
});

test("after a few unfinished checkouts the browser does a quick check first", async ({ page }) => {
  // Three donations started and never paid, today.
  for (let i = 0; i < 3; i++) {
    await api("security", "POST", `/v1/apps/${ids.listedApp}/donate`, { amount_minor: 300 });
  }
  await signIn(page, "security");
  await page.goto(`/app/${ids.listedApp}`);
  const box = await openDonateForm(page);
  await box.getByRole("radiogroup", { name: "Amount" }).getByRole("radio").first().click();

  // When the check can't start, the error shows and the button comes back.
  await page.route("**/v1/donations/challenge", (route) =>
    route.fulfill({ status: 500, contentType: "application/json", body: '{"error":{"code":"internal","message":"The check failed to start."}}' }),
  );
  await box.getByRole("button", { name: /^Donate \$/ }).click();
  await expect(box.getByRole("alert")).toHaveText("The check failed to start.");
  await expect(box.getByRole("button", { name: /^Donate \$/ })).toBeEnabled();
  await page.unroute("**/v1/donations/challenge");

  const challenged = page.waitForResponse((r) => r.url().endsWith("/v1/donations/challenge") && r.ok());
  const stripe = await payInNewTab(page, /^Donate \$/);
  await challenged;
  await expect(stripe.getByRole("heading", { name: "♥ Thank you" })).toBeVisible();
});
