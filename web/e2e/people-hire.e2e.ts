// SPDX-License-Identifier: AGPL-3.0-or-later
// Hiring a creative from their page (POST /v1/people/{handle}/hire). Cleo hires Vera for Cleo's
// Tavern; each test ends the jobs it starts, since Cleo may have only three open requests with
// Vera at once (the seed's requestedJob is one).
import { expect, ids, signIn, test } from "./fixtures";
import { call, hireVera, jobStatus } from "./people-helpers";

test("the hire form opened while the session loads becomes the form once it has (b62)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await signIn(page, "client");
  // Hold the session back until the form is open.
  let release!: () => void;
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/v1/me", async (route) => {
    await held;
    await route.continue();
  });
  await page.goto(`/people/${ids.creativeHandle}`);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  await expect(page.getByText("Sign in to hire Vera Okafor.")).toBeVisible();
  release();
  const form = page.getByRole("form", { name: "Hire Vera Okafor" });
  await expect(form.getByRole("heading", { name: "Hire Vera Okafor" })).toBeVisible();
  await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue("cleos-tavern");
  expect(errors).toEqual([]);
});

test("a maker with no projects is asked to create one first", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(`/people/${ids.creativeHandle}`);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  const form = page.getByRole("form", { name: "Hire Vera Okafor" });
  await expect(form.getByText(/Jobs belong to one of your projects/)).toBeVisible();
  await expect(form.getByRole("link", { name: "Create a project first" })).toHaveAttribute("href", "/publish/new");
  await expect(form.getByRole("button", { name: "Send request" })).toHaveCount(0);
});

test("asking about it first sends an inquiry and opens the job, which the maker can withdraw", async ({ page }) => {
  await signIn(page, "client");
  await page.goto(`/people/${ids.creativeHandle}`);
  // Hire for a package: the package sets the skill.
  await page.locator(".service").filter({ hasText: "Sprite sheet" }).getByRole("button", { name: "Hire for this" }).click();
  const form = page.getByRole("form", { name: "Hire Vera Okafor" });
  const pkg = form.getByRole("combobox", { name: "Package" });
  const skill = form.getByRole("combobox", { name: "Skill" });
  await expect(pkg).toHaveValue(/.+/);
  await expect(skill).toHaveValue("pixel-art");
  await pkg.selectOption({ label: "Something else (describe it below)" });
  await skill.selectOption("music");
  await pkg.selectOption({ label: "Sprite sheet · from $120" });
  await expect(skill).toHaveValue("pixel-art");

  // Asking first needs the words and the "exactly what it says" box; there's no 18+ box for Cleo.
  await expect(form.getByRole("checkbox", { name: "I'm 18 or older" })).toHaveCount(0);
  const ask = form.getByRole("button", { name: "Ask about it first" });
  await expect(ask).toBeDisabled();
  await form.getByRole("textbox", { name: "What do you need, exactly?" }).fill("A tavern cat, idle and sleeping, 32×32. Is that one sheet?");
  await expect(ask).toBeDisabled();
  await form.getByRole("checkbox", { name: /I understand: Vera Okafor will deliver exactly what the agreement says/ }).check();
  await expect(ask).toBeEnabled();
  await ask.click();

  await expect(page).toHaveURL(/\/jobs\/[0-9a-f-]{36}$/);
  await expect(jobStatus(page)).toContainText("You asked Vera Okafor to look at your request.");
  await expect(page.getByText("An inquiry: they asked for a review first")).toBeVisible();
  await expect(page.getByText("A tavern cat, idle and sleeping")).toBeVisible();
  await expect(page.locator(".timeline")).toContainText("Package: Sprite sheet (from $120, about 7 days)");
  await expect(page.getByText(/You asked for a conversation\. Vera Okafor decides when to open it\./)).toBeVisible();

  // The maker withdraws the request with a note.
  await page.locator(".job-secondary").getByRole("button", { name: "Decline" }).click();
  await page.getByRole("textbox", { name: "Decline" }).fill("Found a cat sprite already, thanks!");
  await page.locator(".job-secondary").getByRole("button", { name: "Decline" }).click();
  await expect(jobStatus(page)).toContainText("Declined.");
  await expect(page.locator(".timeline")).toContainText("Found a cat sprite already, thanks!");
  await expect(page.locator(".job-secondary").getByRole("button", { name: "Decline" })).toHaveCount(0);
});

test("a request as a business with a date, and the refusal when too many requests are open", async ({ page }) => {
  // One more open request makes three with the seed's: a fourth is refused.
  const open = await hireVera("Two barrels and a crate, 32×32.");
  await signIn(page, "client");
  await page.goto(`/people/${ids.creativeHandle}`);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  const form = page.getByRole("form", { name: "Hire Vera Okafor" });
  await expect(form.getByRole("combobox", { name: "Package" })).toHaveValue("");
  await form.getByRole("textbox", { name: "What do you need, exactly?" }).fill("A wooden bar counter, 3 tiles wide.");
  await form.getByLabel(/Hoping to have it by/).fill("2030-01-31");
  await form.getByRole("checkbox", { name: /hiring as a business/ }).check();
  await form.getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ }).check();
  await form.getByRole("button", { name: "Send request" }).click();
  await expect(page).toHaveURL(/\/jobs\/[0-9a-f-]{36}$/);
  const job = page.url().split("/").pop()!;
  await expect(jobStatus(page)).toContainText("Waiting for Vera Okafor to send an offer.");
  await expect(page.getByText("Hoped for by 2030-01-31")).toBeVisible();
  expect((await call("client", "GET", `/v1/jobs/${job}`)).client_is_business).toBe(true);

  // Now there are three: the form says why it can't send a fourth, and keeps what was written.
  await page.goto(`/people/${ids.creativeHandle}`);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  await form.getByRole("textbox", { name: "What do you need, exactly?" }).fill("A fireplace, animated.");
  await form.getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ }).check();
  await form.getByRole("button", { name: "Send request" }).click();
  await expect(form.getByRole("alert")).toContainText("you already have 3 open requests with them");
  await expect(form.getByRole("textbox", { name: "What do you need, exactly?" })).toHaveValue("A fireplace, animated.");
  await expect(page).toHaveURL(`/people/${ids.creativeHandle}`);
  await form.getByRole("button", { name: "Cancel" }).click();
  await expect(form).toHaveCount(0);

  for (const id of [open, job]) await call("client", "POST", `/v1/jobs/${id}/decline`, { note: "" });
});

test("hiring asks for 18+ when not confirmed, and for the People terms when not accepted", async ({ page }) => {
  await signIn(page, "client");
  // As an account that hasn't said it's 18 or older yet.
  await page.route("**/v1/me", async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), adult_confirmed: false } });
  });
  // And as if the People terms changed since Cleo accepted them: the first try is refused.
  let refused = false;
  const sent: unknown[] = [];
  await page.route("**/v1/people/vera-pixels/hire", async (route) => {
    sent.push(route.request().postDataJSON());
    if (!refused) {
      refused = true;
      await route.fulfill({
        status: 409,
        json: { error: { code: "terms_required", message: "accept the People terms first", field: "people" } },
      });
    } else await route.continue();
  });
  await page.goto(`/people/${ids.creativeHandle}`);
  await page.getByRole("button", { name: "Hire Vera" }).click();
  const form = page.getByRole("form", { name: "Hire Vera Okafor" });
  await form.getByRole("textbox", { name: "What do you need, exactly?" }).fill("A tavern menu board with six dishes.");
  await form.getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ }).check();
  const ask = form.getByRole("button", { name: "Ask about it first" });
  await expect(ask).toBeDisabled();
  await form.getByRole("checkbox", { name: "I'm 18 or older" }).check();
  await expect(ask).toBeEnabled();
  await form.getByRole("button", { name: "Send request" }).click();

  const dialog = page.getByRole("dialog", { name: /People/ });
  await expect(dialog.getByRole("heading", { name: /Before you continue/ })).toBeVisible();
  const accept = dialog.getByRole("button", { name: "Accept and continue" });
  await expect(accept).toBeDisabled();
  await dialog.getByRole("checkbox", { name: /I've read and accept/ }).check();
  await accept.click();

  // Accepted: the request goes again, and opens the job.
  await expect(page).toHaveURL(/\/jobs\/[0-9a-f-]{36}$/);
  expect(sent).toHaveLength(2);
  expect(sent[1]).toMatchObject({ confirm_adult: true, exact_wording_ack: true, inquiry: false });
  const job = page.url().split("/").pop()!;
  await call("client", "POST", `/v1/jobs/${job}/decline`, { note: "" });
});
