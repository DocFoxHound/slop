// SPDX-License-Identifier: AGPL-3.0-or-later
// A People page from an AI assistant's handoff link (/people/me#fill=…, handoff.ts): filled in,
// with what's the person's own (business or not, 18 or older) left to them; kept through signing
// in, the 18+ question and the People Terms; over a saved page, changes to review. Mo Moderator
// makes a page here (no other test reads one); Olly's and Vera's are only looked at, never saved.
import { expect, server, signIn, test } from "./fixtures";
import { call } from "./people-helpers";

/** The link an assistant builds: the fields as URL-encoded JSON in the fragment. */
const fillLink = (fields: unknown) => `/people/me#fill=${encodeURIComponent(JSON.stringify(fields))}`;

const notice = (page: import("@playwright/test").Page) => page.getByRole("status", { name: "Filled in by your AI assistant" });
/** The page's own name (each service has a Name too). */
const nameBox = (page: import("@playwright/test").Page) => page.getByRole("textbox", { name: "Name", exact: true }).first();

const handle = `mo-${Date.now().toString(36)}`;

test("an assistant's link fills in a new People page; the person answers what's theirs and creates it", async ({ page }) => {
  await signIn(page, "moderator");
  await page.goto(
    fillLink({
      name: "Mo's Pixel Mill",
      handle,
      kind: "studio",
      team_size: 4,
      headline: "Tiny knights, big castles",
      skills: ["pixel-art"],
      ai_use: "heavy",
      ai_note: "Generated, then cleaned up by hand.",
      location: "Leeds, UK",
      turnaround: "2 weeks",
      taking_work: false,
      about_md: "We draw **small** things.",
      // Its skill isn't ticked above: it's ticked for it.
      services: [{ name: "Castle tileset", description: "64 tiles, 16×16.", price_minor: 2500, currency: "gbp", days: 10, skill: "2d-art" }],
      accent: "grape",
      sections: ["about", "portfolio"],
      links: [{ label: "Itch", url: "https://mo.itch.io" }],
      portfolio_repo: "https://github.com/mo/portfolio",
      // The person's own: never filled in.
      trader: true,
      confirm_adult: true,
    }),
  );
  await expect(page.getByRole("heading", { name: "Offer your services", level: 1 })).toBeVisible();
  expect(await page.evaluate(() => location.hash)).toBe("");
  // The intro for someone new still shows.
  await expect(page.getByRole("region", { name: "How People pages work" })).toBeVisible();

  const told = notice(page);
  await expect(told).toContainText("Check everything, especially your page address, your services and their prices, then press Create my page.");
  await expect(told.locator(".handoff-filled")).toHaveText(
    "Filled in: Name, Page address, Who, People in the studio, Headline, What you do, How you use AI, AI use note, Location, Usual turnaround, Taking work now, About, Services, Accent colour, Section order, Links and Portfolio repository.",
  );
  await expect(told.getByRole("listitem")).toHaveText(["I'm 18 or older: only you can say you're 18 or older"]);
  await expect(told).toContainText("Whether you offer services as a business is yours to answer (it's your legal status), so it isn't filled in.");
  await expect(told).toContainText("Your portfolio repository is filled in under Portfolio once your page is created.");

  await expect(nameBox(page)).toHaveValue("Mo's Pixel Mill");
  await expect(page.getByRole("textbox", { name: "Page address" })).toHaveValue(handle);
  await expect(page.getByRole("radio", { name: /A studio of/ })).toBeChecked();
  await expect(page.getByRole("spinbutton", { name: "People in the studio" })).toHaveValue("4");
  await expect(page.getByRole("textbox", { name: "Headline" })).toHaveValue("Tiny knights, big castles");
  const skills = page.locator("fieldset").filter({ hasText: "What you do" });
  await expect(skills.getByRole("checkbox", { name: "Pixel Art" })).toBeChecked();
  await expect(skills.getByRole("checkbox", { name: "2D Art" })).toBeChecked();
  await expect(page.getByRole("radio", { name: /^Mostly AI-generated/ })).toBeChecked();
  await expect(page.getByRole("textbox", { name: "AI use note" })).toHaveValue("Generated, then cleaned up by hand.");
  await expect(page.getByRole("textbox", { name: /Location/ })).toHaveValue("Leeds, UK");
  await expect(page.getByRole("textbox", { name: "Usual turnaround" })).toHaveValue("2 weeks");
  await expect(page.getByRole("checkbox", { name: "Taking work now" })).not.toBeChecked();
  await expect(page.getByRole("textbox", { name: /About/ })).toHaveValue("We draw **small** things.");
  const svc = page.locator(".services-edit > .panel");
  await expect(svc).toHaveCount(1);
  await expect(svc.getByRole("textbox", { name: "Name" })).toHaveValue("Castle tileset");
  await expect(svc.getByRole("textbox", { name: "From" })).toHaveValue("25");
  await expect(svc.getByRole("combobox", { name: "Currency" })).toHaveValue("gbp");
  await expect(svc.getByRole("spinbutton", { name: "Days" })).toHaveValue("10");
  await expect(svc.getByRole("combobox", { name: "Skill" })).toHaveValue("2d-art");
  await expect(svc.getByRole("textbox", { name: "What's included" })).toHaveValue("64 tiles, 16×16.");
  await expect(page.getByRole("radio", { name: "Grape" })).toBeChecked();
  await expect(page.locator("fieldset").filter({ hasText: "Section order" }).locator("li").first()).toHaveText(/About/);
  await expect(page.getByRole("textbox", { name: "Link label" })).toHaveValue("Itch");
  await expect(page.getByRole("textbox", { name: "Link address" })).toHaveValue("https://mo.itch.io");

  // Business or not: the suggestion shows, unanswered, and Create waits for an answer.
  await expect(page.getByText("Your AI assistant suggested “Yes, as a business”, but only you can answer this.")).toBeVisible();
  const business = page.getByRole("radio", { name: /^Yes, as a business/ });
  const individual = page.getByRole("radio", { name: /^No, as a private individual/ });
  await expect(business).not.toBeChecked();
  await expect(individual).not.toBeChecked();
  let saves = 0;
  page.on("request", (r) => {
    if (r.method() === "PUT" && r.url().endsWith("/v1/me/creative")) saves++;
  });
  await page.getByRole("button", { name: "Create my page" }).click();
  expect(await business.evaluate((el: HTMLInputElement) => el.validity.valueMissing)).toBe(true);
  await expect(page).toHaveURL("/people/me");
  expect(saves).toBe(0);

  await individual.check();
  await page.getByRole("button", { name: "Create my page" }).click();
  await expect(page).toHaveURL(`/people/${handle}`);
  const saved = await call("moderator", "GET", "/v1/me/creative");
  expect(saved).toMatchObject({
    handle,
    name: "Mo's Pixel Mill",
    kind: "studio",
    team_size: 4,
    headline: "Tiny knights, big castles",
    skills: ["pixel-art", "2d-art"],
    ai_use: "heavy",
    trader: false,
    taking_work: false,
    accent: "grape",
    sections: ["about", "portfolio", "services", "reviews", "worked_on"],
    links: [{ label: "Itch", url: "https://mo.itch.io/" }],
  });
  expect(saved.services[0]).toMatchObject({ name: "Castle tileset", price_minor: 2500, currency: "gbp", days: 10, skill: "2d-art" });

  // The portfolio repository waits for the Portfolio panel, filled in but not checked.
  await page.getByRole("link", { name: "Add your portfolio" }).click();
  await expect(page).toHaveURL("/people/me#portfolio");
  const repo = page.getByRole("textbox", { name: /^Repository/ });
  await expect(repo).toHaveValue("https://github.com/mo/portfolio");
  await expect(page.getByText("Filled in by your AI assistant: check it's yours, then choose Check my portfolio now.")).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
  // Once.
  await page.reload();
  await expect(page.getByRole("heading", { name: "Your People page", level: 1 })).toBeVisible();
  await expect(page.getByRole("textbox", { name: /^Repository/ })).toHaveValue("");
});

test("signed out, the link survives signing in, the 18+ question and the People Terms", async ({ page }) => {
  const fields = { name: "Olly Draws", handle: "olly-draws", skills: ["2d-art"], headline: "Owls, mostly" };
  await page.goto(fillLink(fields));
  await expect(page.getByText("Sign in to offer your services on People.")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Your AI assistant's draft is ready." })).toBeVisible();
  expect(await page.evaluate(() => location.hash)).toBe("");
  await page.getByRole("main").getByRole("link", { name: "Sign in", exact: true }).click();
  await expect(page).toHaveURL(/\/signin\?return_to=%2Fpeople%2Fme$/);
  // Out to the sign-in provider and back, as signing in goes.
  await page.getByRole("link", { name: "Continue with GitHub" }).click();
  await page.waitForURL(`${server.fake}login/oauth/authorize?**`);
  // Olly hasn't said he's 18 or older yet.
  await signIn(page, "overdue");
  await page.goto("/people/me");
  await expect(notice(page)).toContainText("Saying you're 18 or older is yours to tick, too.");
  await expect(nameBox(page)).toHaveValue("Olly Draws");
  const adult = page.getByRole("checkbox", { name: "I'm 18 or older" });
  await expect(adult).not.toBeChecked();
  await adult.check();
  await page.getByRole("radio", { name: /^No, as a private individual/ }).check();

  // As if the People Terms changed since Olly accepted them: accepted in the dialog, and the
  // page goes again with everything as filled in. Nothing is saved for Olly (the second answer
  // is the test's own), so his account stays as other tests expect it.
  const bodies: Array<Record<string, unknown>> = [];
  await page.route("**/v1/me/creative", async (route) => {
    if (route.request().method() !== "PUT") return route.continue();
    bodies.push(route.request().postDataJSON() as Record<string, unknown>);
    await route.fulfill(
      bodies.length === 1
        ? { status: 409, json: { error: { code: "terms_required", message: "accept the People terms", field: "people" } } }
        : { status: 400, json: { error: { code: "invalid", message: "stopped by the test", field: "handle" } } },
    );
  });
  await page.route("**/v1/me/legal/accept", (route) => route.fulfill({ status: 204 }));
  await page.getByRole("button", { name: "Create my page" }).click();
  const terms = page.getByRole("dialog", { name: /People/ });
  await terms.getByRole("checkbox", { name: /I've read and accept/ }).check();
  await terms.getByRole("button", { name: "Accept and continue" }).click();
  await expect(page.locator("form p.form-error")).toHaveText("stopped by the test");
  expect(bodies).toHaveLength(2);
  expect(bodies[1]).toMatchObject({ ...fields, confirm_adult: true, trader: false });
  await expect(page.getByRole("textbox", { name: "Headline" })).toHaveValue("Owls, mostly");
});

test("over a saved page, the link's changes are there to review, and the saved page can be put back", async ({ page }) => {
  const before = await call("creative", "GET", "/v1/me/creative");
  await signIn(page, "creative");
  await page.goto(
    fillLink({ headline: "Pixel knights, now with horses", taking_work: !before.taking_work, trader: !before.trader, portfolio_repo: "https://github.com/vera/new-portfolio" }),
  );
  await expect(page.getByRole("heading", { name: "Your People page", level: 1 })).toBeVisible();
  const told = notice(page);
  await expect(told).toContainText("Check everything, especially what it changed from your saved page, your services and their prices, then press Save.");
  await expect(told.locator(".handoff-filled")).toHaveText("Filled in: Headline, Taking work now and Portfolio repository.");
  await expect(told).toContainText("so it stays as you saved it.");
  await expect(page.getByRole("textbox", { name: "Headline" })).toHaveValue("Pixel knights, now with horses");
  // The rest is as saved; business or not too, with the suggestion beside it.
  await expect(nameBox(page)).toHaveValue(before.name);
  await expect(page.getByRole("radio", { name: before.trader ? /^Yes, as a business/ : /^No, as a private individual/ })).toBeChecked();
  await expect(page.getByText(/^Your AI assistant suggested .*: your saved answer is kept\.$/)).toBeVisible();
  await expect(page.getByRole("textbox", { name: /^Repository/ })).toHaveValue("https://github.com/vera/new-portfolio");

  await page.getByRole("button", { name: "Put back my saved page" }).click();
  await expect(notice(page)).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Headline" })).toHaveValue(before.headline);
  await expect(page.getByRole("checkbox", { name: "Taking work now" })).toBeChecked({ checked: before.taking_work });
  await expect(page.getByRole("textbox", { name: /^Repository/ })).not.toHaveValue("https://github.com/vera/new-portfolio");
  expect(await call("creative", "GET", "/v1/me/creative")).toEqual(before);
});

test("a link that can't be read says so, and the saved page shows as it is", async ({ page }) => {
  await signIn(page, "creative");
  await page.goto("/people/me#fill=%E0%A4%A");
  await expect(page.getByRole("alert")).toHaveText(
    "The link from your AI assistant couldn't be read: it isn't URL-encoded properly. Your saved page is shown as it is.",
  );
  expect(await page.evaluate(() => location.hash)).toBe("");
  await expect(notice(page)).toHaveCount(0);
});
