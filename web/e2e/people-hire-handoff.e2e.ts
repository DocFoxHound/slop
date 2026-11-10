// SPDX-License-Identifier: AGPL-3.0-or-later
// A request to hire someone from an AI assistant's handoff link (/people/{handle}#fill=…,
// handoff.ts): the hire form opens filled in, what commits the person is left to them, and it's
// kept through signing in. Cleo asks Vera, for Cleo's Tavern; the one request sent here is ended
// again, since Cleo may have only three open with Vera at once (people-hire.e2e.ts).
import { expect, ids, signIn, test } from "./fixtures";
import { call, jobStatus } from "./people-helpers";

/** The link an assistant builds: the fields as URL-encoded JSON in the fragment. */
const fillLink = (fields: unknown) => `/people/${ids.creativeHandle}#fill=${encodeURIComponent(JSON.stringify(fields))}`;

type Page = import("@playwright/test").Page;
const hireForm = (page: Page) => page.getByRole("form", { name: "Hire Vera Okafor" });
const notice = (page: Page) => page.getByRole("status", { name: "Filled in by your AI assistant" });
const requestBox = (page: Page) => hireForm(page).getByRole("textbox", { name: "What do you need, exactly?" });
const understand = (page: Page) => hireForm(page).getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ });
const kept = (page: Page) => page.evaluate(() => sessionStorage.getItem("slop.handoff.hire"));

test("an assistant's link opens the hire form filled in; the person ticks what's theirs and sends it", async ({ page }) => {
  const vera = await call<{ services: Array<{ id: string; name: string }> }>("client", "GET", `/v1/people/${ids.creativeHandle}`);
  const sprites = vera.services.find((s) => s.name === "Sprite sheet")!;
  await signIn(page, "client");
  await page.goto(
    fillLink({
      app_slug: ids.clientApp,
      service_id: sprites.id,
      request_md: "1 tavern keeper, 4 directions, walk cycle of 8 frames, 32×32.\nTransparent PNG sheets.",
      wanted_by: "2030-01-31",
      // The person's own: never filled in.
      exact_wording_ack: true,
      client_is_business: true,
      inquiry: true,
      // Not part of the form.
      budget: 50,
    }),
  );
  const form = hireForm(page);
  await expect(form.getByRole("heading", { name: "Hire Vera Okafor" })).toBeVisible();
  expect(await page.evaluate(() => location.hash)).toBe("");

  const told = notice(page);
  await expect(told).toContainText("Check everything, especially what you're asking for, word for word, then press Send request. Nothing is sent until you do.");
  await expect(told.locator(".handoff-filled")).toHaveText("Filled in: Project, Package, What you need and Hoping to have it by.");
  await expect(told.getByRole("listitem")).toHaveText([
    "I understand: only you can say you've understood it",
    "I'm hiring as a business: whether you're hiring as a business is yours to say",
    "Ask about it first: you choose between Send request and Ask about it first",
  ]);
  await expect(told).toContainText("One other thing in the link isn't part of this form, so it was left out.");
  await expect(told).toContainText("The boxes under the request are yours to tick");

  await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue(ids.clientApp);
  await expect(form.getByRole("combobox", { name: "Package" })).toHaveValue(sprites.id);
  // The package brings its skill.
  await expect(form.getByRole("combobox", { name: "Skill" })).toHaveValue("pixel-art");
  await expect(requestBox(page)).toHaveValue("1 tavern keeper, 4 directions, walk cycle of 8 frames, 32×32.\nTransparent PNG sheets.");
  await expect(form.getByLabel(/Hoping to have it by/)).toHaveValue("2030-01-31");
  await expect(understand(page)).not.toBeChecked();
  await expect(form.getByRole("checkbox", { name: /hiring as a business/ })).not.toBeChecked();
  // In the form now: it isn't kept to be filled in again.
  expect(await kept(page)).toBeNull();

  // Nothing goes until the person has ticked that they understand.
  await form.getByRole("button", { name: "Send request" }).click();
  await expect(page).toHaveURL(`/people/${ids.creativeHandle}`);
  await understand(page).check();
  await form.getByRole("button", { name: "Send request" }).click();
  await expect(page).toHaveURL(/\/jobs\/[0-9a-f-]{36}$/);
  const job = page.url().split("/").pop()!;
  await expect(jobStatus(page)).toContainText("Waiting for Vera Okafor to send an offer.");
  await expect(page.getByText("1 tavern keeper, 4 directions")).toBeVisible();
  await expect(page.getByText("Hoped for by 2030-01-31")).toBeVisible();
  await expect(page.locator(".timeline")).toContainText("Package: Sprite sheet");
  const sent = await call<{ client_is_business: boolean; inquiry: boolean }>("client", "GET", `/v1/jobs/${job}`);
  expect(sent.client_is_business).toBe(false);
  expect(sent.inquiry).toBe(false);
  await call("client", "POST", `/v1/jobs/${job}/decline`, { note: "" });
});

test("what the form wouldn't take is left out and named; clearing, closing and reloading finish with the link", async ({ page }) => {
  await signIn(page, "client");
  await page.goto(
    fillLink({
      // Mia's, not Cleo's.
      app_slug: ids.listedApp,
      skill: "voice",
      request_md: "A fireplace, animated, 4 frames.",
      wanted_by: "next week",
    }),
  );
  const form = hireForm(page);
  const told = notice(page);
  await expect(told.locator(".handoff-filled")).toHaveText("Filled in: What you need.");
  await expect(told.getByRole("listitem")).toHaveText([
    "Project: isn't one of your projects",
    "Skill: isn't one of their skills",
    "Hoping to have it by: should be a date, like 2030-01-31",
  ]);
  await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue(ids.clientApp);
  await expect(requestBox(page)).toHaveValue("A fireplace, animated, 4 frames.");
  await expect(form.getByLabel(/Hoping to have it by/)).toHaveValue("");

  await told.getByRole("button", { name: "Clear the form" }).click();
  await expect(told).toHaveCount(0);
  await expect(requestBox(page)).toHaveValue("");

  // A new link opened in the same tab only changes the fragment: the form starts again from it.
  await requestBox(page).fill("Typed by hand.");
  await page.goto(fillLink({ request_md: "Two barrels and a crate, 32×32.", skill: "music" }));
  await expect(requestBox(page)).toHaveValue("Two barrels and a crate, 32×32.");
  await expect(form.getByRole("combobox", { name: "Skill" })).toHaveValue("music");
  await expect(told.locator(".handoff-filled")).toHaveText("Filled in: Skill and What you need.");
  expect(await page.evaluate(() => location.hash)).toBe("");

  // Cancel finishes with it: the form opened again is empty.
  await form.getByRole("button", { name: "Cancel" }).click();
  await expect(form).toHaveCount(0);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  await expect(requestBox(page)).toHaveValue("");
  await expect(told).toHaveCount(0);

  // And so does a reload: the page is as it is for anyone.
  await page.goto(fillLink({ request_md: "A shop sign." }));
  await expect(requestBox(page)).toHaveValue("A shop sign.");
  await page.reload();
  await expect(page.getByRole("button", { name: "Hire Vera" })).toBeVisible();
  await expect(form).toHaveCount(0);
});

test("signed out, the request waits while the person signs in, then fills in the form", async ({ page }) => {
  await page.goto(fillLink({ request_md: "A tavern sign, hand-painted look, 64×32.", skill: "pixel-art" }));
  await expect(page.getByText("Sign in to hire Vera Okafor.")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Your AI assistant's draft is ready." })).toContainText(
    "Sign in, or make an account, and the form opens with it filled in for you to check.",
  );
  expect(await page.evaluate(() => location.hash)).toBe("");
  await page.locator(".hire-form").getByRole("link", { name: "Sign in" }).click();
  await expect(page).toHaveURL(`/signin?return_to=${encodeURIComponent(`/people/${ids.creativeHandle}`)}`);

  // Signed in and back, as the sign-in pages bring them: the fragment didn't make that trip, the
  // tab's storage did. It's for Vera's page only: someone else's shows no form.
  await signIn(page, "client");
  await page.goto(`/people/${ids.previewerHandle}`);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.locator(".hire-form")).toHaveCount(0);
  await page.goto(`/people/${ids.creativeHandle}`);
  await expect(notice(page).locator(".handoff-filled")).toHaveText("Filled in: Skill and What you need.");
  await expect(requestBox(page)).toHaveValue("A tavern sign, hand-painted look, 64×32.");
  expect(await kept(page)).toBeNull();
});

test("with no project yet, the request is kept for when there is one", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(fillLink({ request_md: "A title screen, 320×180." }));
  const form = hireForm(page);
  await expect(form.getByText(/Jobs belong to one of your projects/)).toBeVisible();
  await expect(form.getByRole("status").filter({ hasText: "Your AI assistant's request is ready." })).toContainText(
    "Come back to this page once you have a project, and the form opens with it filled in. It's kept in this browser tab only, for two hours.",
  );
  await expect(notice(page)).toHaveCount(0);
  expect(await kept(page)).toContain("A title screen, 320×180.");
});

test("a link that can't be read says so over an empty form; someone who can't be hired gets no form", async ({ page }) => {
  await signIn(page, "client");
  await page.goto(`/people/${ids.creativeHandle}#fill=%7Bnot-json`);
  const form = hireForm(page);
  await expect(form.getByRole("alert")).toHaveText(
    "The link from your AI assistant couldn't be read: what follows #fill= isn't JSON. The form is empty: fill it in yourself, or ask your assistant for a new link.",
  );
  await expect(requestBox(page)).toHaveValue("");
  await expect(notice(page)).toHaveCount(0);
  expect(await page.evaluate(() => location.hash)).toBe("");

  // Fully booked: it says why nothing opened, and the request isn't kept.
  await page.route(`**/v1/people/${ids.creativeHandle}`, async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), can_hire: false, taking_work: false } });
  });
  await page.goto("/people");
  await page.goto(fillLink({ request_md: "A fireplace." }));
  await expect(page.getByRole("status").filter({ hasText: "Your AI assistant prepared a request" })).toHaveText(
    "Your AI assistant prepared a request for Vera Okafor, but they're fully booked, so it can't be sent.",
  );
  await expect(page.getByRole("button", { name: "Fully booked" })).toBeDisabled();
  await expect(form).toHaveCount(0);
  expect(await kept(page)).toBeNull();
});

test("a link on your own page does nothing", async ({ page }) => {
  await signIn(page, "creative");
  await page.goto(fillLink({ request_md: "A fireplace." }));
  await expect(page.getByRole("region", { name: "Your page's status" })).toBeVisible();
  await expect(page.locator(".hire-form")).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "AI assistant" })).toHaveCount(0);
  expect(await kept(page)).toBeNull();
});
