// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page editor (/people/me): making a People page, its services, the portfolio
// panel and getting paid. Sec Security makes a new page (no other test reads it); Remy's waiting
// portfolio is only looked at and put back in order; Vera's live page is only looked at.
import { expect, ids, signIn, test } from "./fixtures";
import { call } from "./people-helpers";

test("signed out, and not yet 18-confirmed, the editor says what's needed", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/people/me");
  await expect(visitor.getByText("Sign in to offer your services on People.")).toBeVisible();

  const olly = await browser.newPage();
  await signIn(olly, "overdue");
  await olly.goto("/people/me");
  await expect(olly.getByRole("heading", { name: "Offer your services", level: 1 })).toBeVisible();
  const adult = olly.getByRole("checkbox", { name: "I'm 18 or older" });
  await expect(adult).toBeVisible();
  await expect(adult).toHaveAttribute("required", "");
  await expect(olly.getByRole("heading", { name: "Get paid" })).toHaveCount(0);
  await expect(olly.getByText("Pick what you do first.")).toBeVisible();
});

test("a new creative makes their page: details, services, look, links; then gets paid through Stripe", async ({ page }) => {
  test.slow();
  await signIn(page, "security");
  await page.goto("/people/me");
  await expect(page.getByRole("heading", { name: "Offer your services", level: 1 })).toBeVisible();
  // Adults only, and Sec already said so: no 18+ box.
  await expect(page.getByRole("checkbox", { name: "I'm 18 or older" })).toHaveCount(0);

  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Sec Sound Studio");
  // The address keeps only lower-case letters, digits and dashes.
  await page.getByRole("textbox", { name: "Page address" }).fill("Sec Sounds!");
  await expect(page.getByRole("textbox", { name: "Page address" })).toHaveValue("secsounds");
  // The seed's Vera has vera-pixels: taken.
  await page.getByRole("textbox", { name: "Page address" }).fill("vera-pixels");
  await page.getByRole("radio", { name: /A studio of/ }).check();
  await page.getByRole("spinbutton", { name: "People in the studio" }).fill("3");
  await page.getByRole("textbox", { name: "Headline" }).fill("Footsteps, doors and creaky floors");

  // Services need skills first.
  await expect(page.getByText("Pick what you do first.")).toBeVisible();
  // Skills are chips: clicking one ticks it.
  const skills = page.locator("fieldset").filter({ hasText: "What you do" });
  await skills.getByText("Sound Design", { exact: true }).click();
  await skills.getByText("Music", { exact: true }).click();
  await expect(skills.getByRole("checkbox", { name: "Sound Design" })).toBeChecked();
  await expect(skills.getByRole("checkbox", { name: "Music" })).toBeChecked();
  await page.getByRole("radio", { name: /^AI-assisted/ }).check();
  await page.getByRole("textbox", { name: "AI use note" }).fill("AI for reference mood boards only.");
  await page.getByRole("radio", { name: /^Yes, as a business/ }).check();
  await page.getByRole("textbox", { name: /Location/ }).fill("Lisbon, PT");
  await page.getByRole("textbox", { name: "Usual turnaround" }).fill("1 week");
  await page.getByRole("textbox", { name: /About/ }).fill("We record **real** foley.");

  // Two services, then one removed; prices are checked by the server.
  await page.getByRole("button", { name: "+ Add a service" }).click();
  await page.getByRole("button", { name: "+ Add a service" }).click();
  const services = page.locator(".services-edit > .panel");
  await expect(services).toHaveCount(2);
  await services.nth(1).getByRole("button", { name: "Remove" }).click();
  await expect(services).toHaveCount(1);
  const svc = services.first();
  await svc.getByRole("textbox", { name: "Name" }).fill("Footstep pack");
  await svc.getByRole("textbox", { name: "From" }).fill("20000");
  await svc.getByRole("combobox", { name: "Currency" }).selectOption("eur");
  await svc.getByRole("spinbutton", { name: "Days" }).fill("5");
  await svc.getByRole("combobox", { name: "Skill" }).selectOption("sound");
  await svc.getByRole("textbox", { name: "What's included" }).fill("40 footsteps on 5 surfaces, WAV 48 kHz.");
  await expect(svc.getByText('Shown as "from €20,000".')).toBeVisible();

  // The look: an accent colour and the order of the sections.
  await page.locator("label.swatch").filter({ hasText: "Ocean" }).click();
  await expect(page.getByRole("radio", { name: "Ocean" })).toBeChecked();
  const order = page.locator("fieldset").filter({ hasText: "Section order" }).locator("li");
  await expect(order.first()).toHaveText(/Portfolio/);
  await page.getByRole("button", { name: "Move About up" }).click();
  await page.getByRole("button", { name: "Move Services up" }).click();
  await expect(order.first()).toHaveText(/Services/);
  await expect(order.nth(3)).toHaveText(/About/);

  // Links: up to five; one added and one taken away.
  await page.getByRole("button", { name: "+ Add a link" }).click();
  await page.getByRole("button", { name: "+ Add a link" }).click();
  await page.getByRole("textbox", { name: "Link label" }).first().fill("Showreel");
  await page.getByRole("textbox", { name: "Link address" }).first().fill("https://soundcloud.com/sec-sounds");
  await page.locator("fieldset").filter({ hasText: "Links (up to 5)" }).getByRole("button", { name: "Remove" }).nth(1).click();
  await expect(page.getByRole("textbox", { name: "Link address" })).toHaveCount(1);

  // Errors come back from the server and nothing is lost: first the price, then the address.
  const create = page.getByRole("button", { name: "Create my page" });
  await create.click();
  await expect(page.getByRole("alert")).toContainText(/prices are from 10 to 10,000|taken/);
  await svc.getByRole("textbox", { name: "From" }).fill("35");
  await expect(svc.getByText('Shown as "from €35".')).toBeVisible();
  await create.click();
  await expect(page.getByRole("alert")).toContainText(/taken|already/);
  await expect(page.getByRole("textbox", { name: "Headline" })).toHaveValue("Footsteps, doors and creaky floors");
  await page.getByRole("textbox", { name: "Page address" }).fill("sec-sounds");

  // As if the People terms changed since Sec accepted them: accept, and it goes again.
  let asked = false;
  await page.route("**/v1/me/creative", async (route) => {
    if (route.request().method() === "PUT" && !asked) {
      asked = true;
      await route.fulfill({ status: 409, json: { error: { code: "terms_required", message: "accept the People terms", field: "people" } } });
    } else await route.continue();
  });
  await create.click();
  const terms = page.getByRole("dialog", { name: /People/ });
  await terms.getByRole("checkbox", { name: /I've read and accept/ }).check();
  await terms.getByRole("button", { name: "Accept and continue" }).click();

  // Created: Sec lands on the page itself, told it's saved and that it's a draft until a
  // portfolio is approved (people-status.e2e.ts follows every state).
  await expect(page).toHaveURL("/people/sec-sounds");
  await expect(page.getByRole("status").filter({ hasText: "Your People page is created." })).toContainText(
    "Only you can see it for now: add your portfolio next",
  );
  await expect(page.getByText("Draft: only you can see your page")).toBeVisible();
  await page.getByRole("link", { name: "Add your portfolio" }).click();
  await expect(page).toHaveURL("/people/me#portfolio");
  await expect(page.getByRole("heading", { name: "Your People page", level: 1 })).toBeVisible();
  await expect(page.locator(".step-track li.done")).toHaveText(["Describe yourself"]);
  await expect(page.getByRole("heading", { name: "Portfolio" })).toBeInViewport();
  const saved = await call("security", "GET", "/v1/me/creative");
  expect(saved).toMatchObject({
    handle: "sec-sounds",
    kind: "studio",
    team_size: 3,
    ai_use: "assisted",
    trader: true,
    accent: "ocean",
    sections: ["services", "portfolio", "reviews", "about", "worked_on"],
  });
  expect(saved.services[0]).toMatchObject({ name: "Footstep pack", price_minor: 3500, currency: "eur", days: 5, skill: "sound" });
  expect(saved.links).toHaveLength(1);

  // Edits save too, and show the page with them.
  await page.getByRole("textbox", { name: "Headline" }).fill("Footsteps, doors, creaky floors and rain");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page).toHaveURL("/people/sec-sounds");
  await expect(page.getByRole("status").filter({ hasText: "Your People page is saved." })).toBeVisible();
  // Said once: not again after a reload.
  await page.reload();
  await expect(page.getByRole("heading", { name: "Sec Sound Studio", level: 1 })).toBeVisible();
  await expect(page.getByText("Your People page is saved.")).toHaveCount(0);
  await page.goto("/people/me");
  await expect(page.getByRole("link", { name: "View my page" })).toHaveAttribute("href", "/people/sec-sounds");

  // The page, only for Sec until a portfolio is approved.
  await page.getByRole("link", { name: "View my page" }).click();
  await expect(page).toHaveURL("/people/sec-sounds");
  await expect(page.getByRole("heading", { name: "Sec Sound Studio", level: 1 })).toBeVisible();
  await expect(page.getByText(/STUDIO · 3 PEOPLE/)).toBeVisible();
  await expect(page.getByText("Footsteps, doors, creaky floors and rain")).toBeVisible();
  await expect(page.locator(".service").filter({ hasText: "Footstep pack" })).toContainText("from €35");
  await expect(page.getByText("AI for reference mood boards only.")).toBeVisible();

  // Get paid: Stripe's sign-up (the fake finishes at once) and back.
  await page.goto("/people/me");
  const paid = page.locator("section").filter({ has: page.getByRole("heading", { name: "Get paid" }) });
  await expect(paid.getByText(/Slop's fee is 5% of each payment/)).toBeVisible();
  const refreshed = page.waitForRequest((r) => r.url().endsWith("/v1/me/creative/stripe/refresh"));
  await paid.getByRole("button", { name: "Set up payments with Stripe" }).click();
  await refreshed;
  await expect(page).toHaveURL("/people/me");
  await expect(paid.getByText(/Payments go straight to your own Stripe account/)).toBeVisible();
  await expect(page.locator(".step-track li.done")).toHaveText(["Describe yourself", "Get paid"]);
});

test("the portfolio panel: waiting vs live, checking a repository, reordering, and errors (b68)", async ({ browser }) => {
  // Vera's portfolio is live.
  const vera = await browser.newPage();
  await signIn(vera, "creative");
  await vera.goto("/people/me");
  await expect(vera.getByText("Live: everyone can see your page")).toBeVisible();
  await expect(vera.getByRole("link", { name: `/people/${ids.creativeHandle}` })).toBeVisible();
  await expect(vera.getByText(/New portfolio waiting for review/)).toHaveCount(0);
  await expect(vera.locator(".step-track li.done")).toHaveCount(3);

  // Remy's first one waits for staff.
  const remy = await browser.newPage();
  await signIn(remy, "pendingCreative");
  await remy.goto("/people/me");
  const panel = remy.locator("section").filter({ has: remy.getByRole("heading", { name: "Portfolio" }) });
  await expect(panel.getByText("New portfolio waiting for review (3 works).")).toBeVisible();
  await expect(remy.getByText("Waiting for Slop's review")).toBeVisible();
  await expect(remy.getByText("Live: everyone can see your page")).toHaveCount(0);
  await panel.getByText("Example .slop/portfolio.json").click();
  await expect(panel.locator("pre.code-block")).toContainText('"works"');

  // Checking a repository: it must be a GitHub link, and one of his own.
  const repo = panel.getByRole("textbox", { name: "Repository" });
  await repo.fill("https://example.com/remy/portfolio");
  await panel.getByRole("button", { name: "Check my portfolio now" }).click();
  await expect(panel.getByRole("alert")).toContainText("use a public GitHub repository link");
  await repo.fill("https://github.com/someone-else/portfolio");
  await panel.getByRole("button", { name: "Check my portfolio now" }).click();
  await expect(panel.getByRole("alert")).toBeVisible();
  await expect(panel.getByText("New portfolio waiting for review (3 works).")).toBeVisible();

  // Reordering moves works; a failed save says so.
  const works = panel.locator("ol.media-order li");
  await expect(works).toHaveText([/Knight/, /Tavern/, /Dragon/]);
  await expect(works.first().getByRole("button", { name: "Move up" })).toBeDisabled();
  await works.first().getByRole("button", { name: "Move down" }).click();
  await expect(works).toHaveText([/Tavern/, /Knight/, /Dragon/]);
  await remy.route("**/v1/me/creative/portfolio/order", (route) =>
    route.fulfill({ status: 500, json: { error: { code: "internal", message: "something went wrong" } } }),
  );
  await works.nth(2).getByRole("button", { name: "Move up" }).click();
  await expect(panel.getByRole("alert")).toContainText("something went wrong");
  await expect(works).toHaveText([/Tavern/, /Knight/, /Dragon/]);
  await remy.unroute("**/v1/me/creative/portfolio/order");
  // Put back as it was.
  await works.nth(1).getByRole("button", { name: "Move up" }).click();
  await expect(works).toHaveText([/Knight/, /Tavern/, /Dragon/]);
  await remy.reload();
  await expect(works).toHaveText([/Knight/, /Tavern/, /Dragon/]);

  // A portfolio that couldn't load says so and can be tried again, rather than loading forever.
  let fail = true;
  await remy.route("**/v1/me/creative/portfolio", (route) =>
    fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy" } } }) : route.continue(),
  );
  await remy.reload();
  const failed = remy.getByRole("alert").filter({ hasText: "Slop is busy" });
  await expect(failed).toBeVisible();
  fail = false;
  await failed.getByRole("button", { name: "Try again" }).click();
  await expect(panel.getByText("New portfolio waiting for review (3 works).")).toBeVisible();
});

test("a turned-down portfolio shows why; Stripe pending, Check again, and payments off", async ({ page }) => {
  await signIn(page, "pendingCreative");
  // Shown as if staff had turned the portfolio down, and Stripe were still checking.
  await page.route("**/v1/me/creative/portfolio", async (route) => {
    const res = await route.fetch();
    const ov = await res.json();
    await route.fulfill({
      response: res,
      json: { ...ov, pending: null, rejected: { ...ov.pending, status: "rejected", status_reason: "Two works are someone else's art." } },
    });
  });
  let payments = "stripe";
  await page.route("**/v1/me/creative", async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), stripe_status: "pending", payments } });
  });
  await page.route("**/v1/me/creative/stripe/refresh", (route) => route.fulfill({ json: { url: null, status: "active" } }));
  await page.goto("/people/me");
  await expect(page.getByText("Your last portfolio was turned down: Two works are someone else's art.")).toBeVisible();

  const paid = page.locator("section").filter({ has: page.getByRole("heading", { name: "Get paid" }) });
  await expect(paid.getByText("Stripe hasn't finished checking your details yet.")).toBeVisible();
  await expect(paid.getByRole("button", { name: "Continue on Stripe" })).toBeEnabled();
  await paid.getByRole("button", { name: "Check again" }).click();
  await expect(paid.getByText(/Payments go straight to your own Stripe account/)).toBeVisible();

  payments = "off";
  await page.reload();
  await expect(paid.getByRole("button", { name: "Continue on Stripe" })).toBeDisabled();
  await expect(paid.getByText("Payments aren't switched on yet on this server.")).toBeVisible();
});
