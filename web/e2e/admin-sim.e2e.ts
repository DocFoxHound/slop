// SPDX-License-Identifier: AGPL-3.0-or-later
// /sim, "view as a simulated user": only on a server running the simulation sandbox. The test
// server isn't one, so the page is missing there; the sandbox's answers are then played to the
// page to check what it does with them.
import type { Page } from "@playwright/test";

import { expect, test } from "./fixtures";

const users = [
  { id: "00000000-0000-7000-8000-00000000aa01", display_name: "Rowan", personas: "roguelike fan:1,developer:0.35", country_code: "US", library: 14, votes: 31 },
  { id: "00000000-0000-7000-8000-00000000aa02", display_name: "Isla", personas: "cozy:0.7", country_code: "GB", library: 6, votes: 9 },
  { id: "00000000-0000-7000-8000-00000000aa03", display_name: "Hana", personas: "People: maker", country_code: "GB", library: 4, votes: 0 },
];

/** Plays the simulation sandbox: the server says so, lists simulated users and signs in as one. */
async function sandbox(page: Page, login: { status: number; json?: unknown } = { status: 204 }) {
  const logins: string[] = [];
  await page.route("**/v1/server", (route) =>
    route.fulfill({ json: { scanning: false, maker_signatures_required: false, simulation: true } }),
  );
  await page.route("**/v1/sim/users", (route) => route.fulfill({ json: users }));
  await page.route("**/v1/sim/login/*", (route) => {
    logins.push(route.request().url().split("/").pop()!);
    return route.fulfill(login);
  });
  return logins;
}

test("the page isn't there unless the server is a simulation sandbox", async ({ page }) => {
  await page.goto("/sim");
  await expect(page.getByText("Page not found.")).toBeVisible();
  await expect(page.getByRole("heading", { name: "View as a simulated user" })).toHaveCount(0);
  // Nor is the banner that points to it.
  await expect(page.getByText("Simulation sandbox.")).toHaveCount(0);
});

test("on the sandbox: every page says so; filter the simulated users and view as one", async ({ page }) => {
  const logins = await sandbox(page);
  await page.goto("/");
  const banner = page.getByRole("note").filter({ hasText: "Simulation sandbox." });
  await expect(banner).toBeVisible();
  await banner.getByRole("link", { name: "View as a simulated user" }).click();
  await expect(page.getByRole("heading", { name: "View as a simulated user", level: 1 })).toBeVisible();

  // Each with their taste in words.
  await expect(page.getByText("roguelike fan + a little developer")).toBeVisible();
  await expect(page.getByText("Rowan · US · 14 in library · 31 votes")).toBeVisible();
  await expect(page.getByText("some cozy")).toBeVisible();
  await expect(page.getByText("Hana · GB · 4 jobs")).toBeVisible();

  // The filter matches personas, names and nothing else.
  const filter = page.getByRole("textbox", { name: "Filter" });
  await filter.fill("cozy");
  await expect(page.getByRole("button", { name: "View as" })).toHaveCount(1);
  await expect(page.getByText("Isla · GB · 6 in library · 9 votes")).toBeVisible();
  await filter.fill("people");
  await expect(page.getByRole("button", { name: "View as" })).toHaveCount(1);

  // A People persona lands on their jobs; anyone else on the store.
  await page.getByRole("button", { name: "View as" }).click();
  await expect(page).toHaveURL(/\/people\/jobs$/);
  expect(logins).toEqual([users[2].id]);
  await page.goto("/sim");
  await page.getByRole("listitem").filter({ hasText: "Rowan" }).getByRole("button", { name: "View as" }).click();
  await expect(page).toHaveURL(new RegExp(`${new URL(page.url()).origin}/$`));
  expect(logins).toEqual([users[2].id, users[0].id]);
});

test("on the sandbox: signing in as a simulated user can fail, and says why", async ({ page }) => {
  await sandbox(page, { status: 404, json: { error: { code: "not_found", message: "not found" } } });
  await page.goto("/sim");
  await page.getByRole("listitem").filter({ hasText: "Isla" }).getByRole("button", { name: "View as" }).click();
  await expect(page.getByText("not found", { exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\/sim$/);
});
