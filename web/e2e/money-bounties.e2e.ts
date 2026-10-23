// SPDX-License-Identifier: AGPL-3.0-or-later
// Fix bounties (web/src/pages/Bounties.tsx): the list, opening one, pledging (and its refusals),
// delivering, the maker's answer and the backers' vote. The seeded bounties are only read; the
// rest happens on the money tests' own apps (money-helpers.ts).
import { expect, ids, server, signIn, test } from "./fixtures";
import { api, call, moneyApps, openBounty, pledge, startPledge } from "./money-helpers";

const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

test("the bounties list shows each bounty's status, amount and backers, an empty list, and a retry", async ({ page }) => {
  await page.goto("/bounties");
  await expect(page.getByRole("heading", { name: "Fix bounties", level: 1 })).toBeVisible();
  const open = page.getByRole("listitem").filter({ has: page.getByRole("link", { name: "Controller support" }) });
  await expect(open.getByText("OPEN", { exact: true })).toBeVisible();
  await expect(open.getByRole("link", { name: "Lantern Keep" })).toBeVisible();
  await expect(open.getByText(/^\$[\d.]+ from \d+ backers? · open until/)).toBeVisible();
  const claimed = page.getByRole("listitem").filter({ has: page.getByRole("link", { name: "A Linux build" }) });
  await expect(claimed.getByText("DELIVERY WAITING")).toBeVisible();
  await expect(claimed.getByText(/^\$35 from 2 backers/)).toBeVisible();

  // Nothing open anywhere.
  await page.route("**/v1/bounties", (route) => route.fulfill(json(200, [])));
  await page.reload();
  await expect(page.getByText("No open bounties yet. Open one on any app's page.")).toBeVisible();

  // Slop didn't answer: an error with a way to try again.
  await page.unroute("**/v1/bounties");
  let failed = false;
  await page.route("**/v1/bounties", (route) => {
    if (failed) return route.fallback();
    failed = true;
    return route.fulfill(json(500, { error: { code: "internal", message: "Something went wrong on Slop's side." } }));
  });
  await page.reload();
  await expect(page.getByRole("alert")).toBeVisible();
  await page.getByRole("button", { name: /Try again|Retry/ }).click();
  await expect(page.getByRole("link", { name: "Controller support" })).toBeVisible();
});

test("opening a bounty: only signed in, a currency, the server's refusals, and never on mods or Private Slop", async ({ browser }) => {
  const { original } = await moneyApps();
  // Signed out: the bounties are listed, but there's no button.
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${ids.listedApp}`);
  await expect(visitor.locator("#bounties").getByText("Controller support")).toBeVisible();
  await expect(visitor.locator("#bounties").getByRole("button", { name: "Open a bounty" })).toHaveCount(0);
  // A mod (one .zip for every platform) has no bounties.
  await visitor.goto(`/app/${ids.mod}`);
  await expect(visitor.getByRole("heading", { name: "More Lanterns", level: 1 })).toBeVisible();
  await expect(visitor.locator("#bounties")).toHaveCount(0);

  // Private Slop has none either (its owner's view).
  const vera = await browser.newPage();
  await signIn(vera, "creative");
  if ((await call("creative", "GET", "/v1/apps/vera-private-sketches")).status === 404) {
    await api("creative", "POST", "/v1/apps", {
      slug: "vera-private-sketches",
      title: "Vera's Private Sketches",
      summary: "A private game for the money tests.",
      description_md: "Private.",
      kind: "game",
      stage: "released",
      repo_url: `${server.fake}e2e/vera-private-sketches`,
      license_spdx: "MIT",
      min_age: 12,
      tags: ["puzzle"],
      platforms: ["windows"],
      visibility: "private",
    });
  }
  await vera.goto("/app/vera-private-sketches");
  await expect(vera.getByRole("heading", { name: "Vera's Private Sketches", level: 1 })).toBeVisible();
  await expect(vera.locator("#bounties")).toHaveCount(0);
  await expect(vera.getByRole("form", { name: /^Donate to/ })).toHaveCount(0);

  // Signed in: open one in euros.
  const page = await browser.newPage();
  await signIn(page, "security");
  await page.goto(`/app/${original}`);
  const section = page.locator("#bounties");
  await section.getByRole("button", { name: "Open a bounty" }).click();
  const openIt = section.getByRole("button", { name: "Open it" });
  await expect(openIt).toBeDisabled();
  await section.getByRole("textbox", { name: "What you want, in a line" }).fill("A high-contrast tip jar");
  await expect(openIt).toBeDisabled();
  await section.getByRole("textbox", { name: /Details/ }).fill("The coins are hard to see on the dark shelf.");
  await section.getByRole("combobox", { name: "Currency" }).selectOption("eur");

  // What the server says when it refuses is shown in the form, which keeps what was typed.
  await page.route(`**/v1/apps/${original}/bounties`, (route) =>
    route.request().method() === "POST"
      ? route.fulfill(json(409, { error: { code: "conflict", message: "you have 10 open bounties already" } }))
      : route.fallback(),
  );
  await openIt.click();
  await expect(section.getByRole("alert")).toHaveText("you have 10 open bounties already");
  await page.unroute(`**/v1/apps/${original}/bounties`);
  await openIt.click();
  await expect(page.getByRole("heading", { name: "A high-contrast tip jar", level: 1 })).toBeVisible();
  await expect(page.getByText(/^OPEN €0 from 0 backers · open until/)).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Amount (EUR, 1 to 500)" })).toBeVisible();
  await expect(page.getByText("Nobody has delivered it yet.")).toBeVisible();

  // Cancel closes the form.
  await page.goto(`/app/${original}`);
  await section.getByRole("button", { name: "Open a bounty" }).click();
  await section.getByRole("button", { name: "Cancel" }).click();
  await expect(section.getByRole("button", { name: "Open a bounty" })).toBeVisible();
  await expect(section.getByRole("link", { name: "A high-contrast tip jar" })).toBeVisible();
});

test("pledging refuses bad amounts and minors, waits while a delivery is decided, and does the browser check", async ({ browser }) => {
  const { original } = await moneyApps();
  const bounty = await openBounty("finance", original, "Bigger coins");

  const visitor = await browser.newPage();
  await visitor.goto(`/bounties/${bounty}`);
  await expect(visitor.getByText("Sign in to pledge.")).toBeVisible();
  await expect(visitor.getByRole("button", { name: "Pledge" })).toHaveCount(0);

  const page = await browser.newPage();
  await signIn(page, "finance");
  await page.goto(`/bounties/${bounty}`);
  const amount = page.getByRole("textbox", { name: /^Amount/ });
  const pledgeButton = page.getByRole("button", { name: "Pledge" });
  await page.getByRole("checkbox", { name: /Save my card with Stripe/ }).check();
  await amount.fill("abc");
  await pledgeButton.click();
  await expect(page.getByRole("alert")).toHaveText("Enter an amount, like 10 or 12.50.");
  for (const bad of ["0", "501"]) {
    await amount.fill(bad);
    await pledgeButton.click();
    await expect(page.getByRole("alert")).toHaveText("1.00 to 500.00");
  }

  // Someone who hasn't said they're 18 is refused until they do (Olly never has).
  const olly = await browser.newPage();
  await signIn(olly, "overdue");
  await olly.goto(`/bounties/${bounty}`);
  await olly.getByRole("checkbox", { name: /Save my card with Stripe/ }).check();
  await expect(olly.getByRole("checkbox", { name: "I'm 18 or older" })).not.toBeChecked();
  await olly.getByRole("button", { name: "Pledge" }).click();
  await expect(olly.getByRole("alert")).toHaveText("donating is for adults: confirm you're 18 or older");

  // While a delivery waits for a decision, nobody new can pledge (the seeded claimed bounty).
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/bounties/${ids.claimedBounty}`);
  await expect(pat.getByText("DELIVERY WAITING").first()).toBeVisible();
  await expect(pat.getByText("A delivery is waiting for a decision: pledge again once it's settled.")).toBeVisible();
  await expect(pat.getByRole("button", { name: "Pledge" })).toHaveCount(0);

  // Payments switched off on the server: said so, and no form.
  await page.route(`**/v1/bounties/${bounty}`, async (route) => {
    const res = await route.fetch();
    return route.fulfill({ response: res, json: { ...(await res.json()), payments: "off" } });
  });
  await page.reload();
  await expect(page.getByText("Payments aren't set up on this server yet.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Pledge" })).toHaveCount(0);
  // The page asks for the bounty again once it knows who's signed in, so a request may still be
  // in the handler: waiting for it keeps it from being let through (and answered twice) instead.
  await page.unrouteAll({ behavior: "wait" });

  // After three unfinished pledges in a day, the browser does a quick proof of work first; then
  // on to Stripe as usual.
  for (let i = 0; i < 3; i++) {
    await startPledge("finance", bounty, 500);
    await api("finance", "DELETE", `/v1/bounties/${bounty}/pledge`);
  }
  await page.reload();
  const challenged = page.waitForResponse((r) => r.url().endsWith("/v1/donations/challenge"));
  await page.getByRole("checkbox", { name: /Save my card with Stripe/ }).check();
  await amount.fill("7");
  await page.getByRole("button", { name: "Pledge" }).click();
  await challenged;
  await expect(page.getByRole("heading", { name: "Fake Stripe Checkout" })).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Pay" }).click();
  await expect(page).toHaveURL(new RegExp(`/bounties/${bounty}\\?pledged=1$`));
  await expect(page.getByText("$7: your card is saved; charged only if a delivery is confirmed")).toBeVisible();
});

test("a failed browser check gives the Pledge button back (b63)", async ({ page }) => {
  const { original } = await moneyApps();
  const bounty = await openBounty("finance", original, "Coin sounds");
  await signIn(page, "finance");
  await page.route(`**/v1/bounties/${bounty}/pledge`, (route) =>
    route.fulfill(json(409, { error: { code: "challenge_required", message: "one more step before trying again" } })),
  );
  await page.route("**/v1/donations/challenge", (route) =>
    route.fulfill(json(500, { error: { code: "internal", message: "Slop couldn't start the check." } })),
  );
  await page.goto(`/bounties/${bounty}`);
  await page.getByRole("checkbox", { name: /Save my card with Stripe/ }).check();
  await page.getByRole("button", { name: "Pledge" }).click();
  await expect(page.getByRole("alert")).toHaveText("Slop couldn't start the check.");
  await expect(page.getByRole("button", { name: "Pledge" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "One moment…" })).toHaveCount(0);
});

test("a builder delivers, withdraws and delivers again; the maker declines, then pays", async ({ browser }) => {
  const { original, build, nightly } = await moneyApps();
  const bounty = await openBounty("support", original, "Faster coin counting");
  await pledge("support", bounty, 2000);

  // Only makers of the app or a community build of it can deliver.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/bounties/${bounty}`);
  await expect(pat.getByRole("heading", { name: "Faster coin counting", level: 1 })).toBeVisible();
  await expect(pat.getByRole("heading", { name: "Delivered it?" })).toHaveCount(0);

  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/bounties/${bounty}`);
  const picker = bo.getByRole("combobox", { name: "In which of your apps" });
  await expect(picker.getByRole("option")).toHaveCount(2);
  expect((await picker.getByRole("option").allTextContents()).sort()).toEqual(["Tip Jar Tycoon Nightly", "Tip Jar Tycoon Plus"]);
  const claimIt = bo.getByRole("button", { name: "Claim it" });
  await expect(claimIt).toBeDisabled();
  await bo.getByRole("textbox", { name: "What you did, for the backers" }).fill("Counting is instant now.");
  // A build with no approved release can't be a delivery.
  await picker.selectOption(nightly);
  await claimIt.click();
  await expect(bo.getByRole("alert")).toHaveText("release it first: a delivery is an approved release");
  await picker.selectOption(build);
  await claimIt.click();
  const delivery = bo.getByRole("listitem").filter({ hasText: "Counting is instant now." });
  await expect(delivery.getByText("PENDING")).toBeVisible();
  await expect(delivery.getByRole("link", { name: "Tip Jar Tycoon Plus" })).toBeVisible();
  await expect(bo.getByText("DELIVERY WAITING")).toBeVisible();
  await expect(bo.getByRole("heading", { name: "Delivered it?" })).toHaveCount(0);

  // The builder can take it back while it waits (b43).
  await delivery.getByRole("button", { name: "Withdraw my delivery" }).click();
  await expect(delivery.getByText("WITHDRAWN")).toBeVisible();
  await expect(bo.getByText("OPEN", { exact: true })).toBeVisible();
  await bo.getByRole("textbox", { name: "What you did, for the backers" }).fill("Counting is instant now, really.");
  await claimIt.click();
  const again = bo.getByRole("listitem").filter({ hasText: "Counting is instant now, really." });
  await expect(again.getByText("PENDING")).toBeVisible();
  // Nobody else gets the Withdraw button, and only the app's maker answers.
  const sue = await browser.newPage();
  await signIn(sue, "support");
  await sue.goto(`/bounties/${bounty}`);
  await expect(sue.getByRole("listitem").filter({ hasText: "really." }).getByText("PENDING")).toBeVisible();
  await expect(sue.getByRole("button", { name: "Withdraw my delivery" })).toHaveCount(0);
  await expect(sue.getByRole("button", { name: "It's delivered: pay the builder" })).toHaveCount(0);
  // The maker answers first; backers vote only if they don't.
  await expect(sue.getByRole("group", { name: "Was it delivered?" })).toHaveCount(0);
  await expect(bo.getByRole("button", { name: "It's delivered: pay the builder" })).toHaveCount(0);

  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/bounties/${bounty}`);
  await expect(mia.getByRole("button", { name: "Withdraw my delivery" })).toHaveCount(0);
  await mia.getByRole("button", { name: "Not delivered" }).click();
  await expect(mia.getByRole("listitem").filter({ hasText: "really." }).getByText("DECLINED")).toBeVisible();
  await expect(mia.getByText("OPEN", { exact: true })).toBeVisible();
  await expect(mia.getByRole("button", { name: "Not delivered" })).toHaveCount(0);

  await bo.reload();
  await picker.selectOption(build);
  await bo.getByRole("textbox", { name: "What you did, for the backers" }).fill("Third time: counting in one frame.");
  await claimIt.click();
  await expect(bo.getByRole("listitem").filter({ hasText: "one frame" }).getByText("PENDING")).toBeVisible();

  // A refusal from the server is shown; then the maker accepts and the builder is paid.
  await mia.reload();
  await mia.route("**/answer", (route) =>
    route.fulfill(json(409, { error: { code: "conflict", message: "that delivery was just withdrawn" } })),
  );
  await mia.getByRole("button", { name: "It's delivered: pay the builder" }).click();
  await expect(mia.getByRole("alert")).toHaveText("that delivery was just withdrawn");
  await mia.unroute("**/answer");
  await mia.getByRole("button", { name: "It's delivered: pay the builder" }).click();
  await expect(mia.getByRole("listitem").filter({ hasText: "one frame" }).getByText("ACCEPTED")).toBeVisible();
  await expect(mia.getByText(/^(PAYING|PAID)$/)).toBeVisible();
  await expect(mia.getByRole("alert")).toHaveCount(0);
});

test("backers vote on the maker's own delivery", async ({ browser }) => {
  const { original } = await moneyApps();
  const bounty = await openBounty("moderator", original, "A coin counter");
  await pledge("moderator", bounty, 1000);
  await pledge("reviewer", bounty, 500);

  // The maker delivers in her own app: her backers decide, not her.
  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/bounties/${bounty}`);
  await expect(mia.getByRole("combobox", { name: "In which of your apps" }).getByRole("option")).toHaveText(["Tip Jar Tycoon"]);
  await mia.getByRole("textbox", { name: "What you did, for the backers" }).fill("A counter in the corner.");
  await mia.getByRole("button", { name: "Claim it" }).click();
  await expect(mia.getByText("PENDING")).toBeVisible();
  await expect(mia.getByRole("button", { name: "It's delivered: pay the builder" })).toHaveCount(0);
  await expect(mia.getByRole("group", { name: "Was it delivered?" })).toHaveCount(0);

  // Not a backer: no vote.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/bounties/${bounty}`);
  await expect(pat.getByText("PENDING")).toBeVisible();
  await expect(pat.getByRole("group", { name: "Was it delivered?" })).toHaveCount(0);

  const mo = await browser.newPage();
  await signIn(mo, "moderator");
  await mo.goto(`/bounties/${bounty}`);
  const vote = mo.getByRole("group", { name: "Was it delivered?" });
  const yes = vote.getByRole("button", { name: "Yes, it's delivered" });
  const no = vote.getByRole("button", { name: "No" });
  await expect(yes).toHaveAttribute("aria-pressed", "false");
  await expect(no).toHaveAttribute("aria-pressed", "false");
  await yes.click();
  await expect(yes).toHaveAttribute("aria-pressed", "true");
  await expect(no).toHaveAttribute("aria-pressed", "false");
  await expect(mo.getByText("Backers so far: $10 yes, $0 no")).toBeVisible();
  // A vote can change until it's settled.
  await no.click();
  await expect(no).toHaveAttribute("aria-pressed", "true");
  await expect(yes).toHaveAttribute("aria-pressed", "false");
  await expect(mo.getByText("Backers so far: $0 yes, $10 no")).toBeVisible();

  // The last backer votes: more than half the pledged amount says no, so it's declined.
  const rae = await browser.newPage();
  await signIn(rae, "reviewer");
  await rae.goto(`/bounties/${bounty}`);
  await rae.getByRole("button", { name: "Yes, it's delivered" }).click();
  await expect(rae.getByText("DECLINED")).toBeVisible();
  await expect(rae.getByText("OPEN", { exact: true })).toBeVisible();
  await expect(rae.getByRole("group", { name: "Was it delivered?" })).toHaveCount(0);
});
