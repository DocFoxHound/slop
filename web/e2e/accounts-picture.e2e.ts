// SPDX-License-Identifier: AGPL-3.0-or-later
// The profile picture (crates/slop-api/src/avatars.rs; Account → Picture in
// components/AccountSections.tsx, components/Avatar.tsx everywhere a name is shown). Pia Picture
// is the account these tests change (her GitHub sign-in's picture comes from the fake GitHub);
// each test puts her back to the letter.
import type { Locator, Page } from "@playwright/test";

import { apiError, call, fake, tweak } from "./accounts-helpers";
import { expect, ids, server, signIn, test } from "./fixtures";

const avatarUrl = new RegExp(`^${server.url}/v1/users/${ids.pictured}/avatar\\?v=[0-9a-f]+$`);

/** SLAPorSLOP's copy loaded: a 128-pixel square. */
async function loaded(img: Locator) {
  await expect(img).toHaveAttribute("src", avatarUrl);
  await expect.poll(() => img.evaluate((i: HTMLImageElement) => (i.complete ? i.naturalWidth : 0))).toBe(128);
}

const menu = (page: Page) => page.getByRole("link", { name: "Pia Picture" }).filter({ has: page.locator(".avatar") });

test.afterEach(async () => {
  await call("pictured", "PUT", "/v1/me/avatar", { provider: null });
});

test("choosing the GitHub picture shows SLAPorSLOP's copy beside her name, and the letter comes back", async ({ page }) => {
  await signIn(page, "pictured");
  await page.goto("/account#picture");
  const section = page.locator("#picture");
  await expect(section.getByRole("heading", { name: "Picture" })).toBeVisible();
  const letter = section.getByRole("radio", { name: /The letter/ });
  const github = section.getByRole("radio", { name: /Your GitHub picture/ });
  await expect(letter).toBeChecked();
  await expect(section.getByText("pia-gh")).toBeVisible();
  await expect(section.getByText("Kept up to date: SLAPorSLOP checks it every day.")).toBeVisible();
  await expect(section.locator(".picture-current")).toContainText("P");
  await expect(menu(page).locator("img")).toHaveCount(0);

  await github.check();
  await expect(github).toBeChecked();
  await loaded(section.locator(".picture-current img"));
  // The menu shows it too, at its own fixed size.
  await loaded(menu(page).locator("img"));
  const box = await menu(page).locator(".avatar").boundingBox();
  expect(box?.width).toBe(32);
  expect(box?.height).toBe(32);

  // Her profile, as anyone sees it, signed out too.
  await page.context().clearCookies();
  await page.goto(`/u/${ids.pictured}`);
  await loaded(page.locator(".profile-head img"));

  // Back to the letter: gone from the profile.
  await signIn(page, "pictured");
  await page.goto("/account#picture");
  // Her GitHub picture shows as chosen once loaded (never the letter first, which would be wrong).
  await expect(github).toBeChecked();
  await letter.check();
  await expect(letter).toBeChecked();
  await expect(section.locator(".picture-current img")).toHaveCount(0);
  await expect(section.locator(".picture-current")).toContainText("P");
  await page.goto(`/u/${ids.pictured}`);
  await expect(page.locator(".profile-head .avatar")).toHaveText("P");
});

test("a sign-in that shares its picture only at sign-in offers Refresh, and a refusal says why", async ({ page }) => {
  // The test server can't sign anyone in to Discord or Microsoft, so Pia's are stood in for.
  await tweak(page, "/v1/me/avatar", (p) => ({
    ...p,
    choices: [
      ...p.choices,
      { provider: "discord", subject: "80351110224678912", login: "pia", available: false, updates: "sign_in", can_refresh: true },
      { provider: "microsoft", subject: "ms-pia", login: null, available: false, updates: "never", can_refresh: false },
    ],
  }));
  await signIn(page, "pictured");
  await page.goto("/account#picture");
  const section = page.locator("#picture");
  await expect(section.getByRole("radio", { name: /Your Discord picture/ })).toBeDisabled();
  await expect(
    section.getByText("SLAPorSLOP gets this picture when you sign in with Discord: press Refresh picture to fetch it now (a quick trip to Discord and back)."),
  ).toBeVisible();
  const discord = section.locator(".picture-choice").filter({ hasText: "Your Discord picture" });
  await expect(discord.getByRole("link", { name: "Refresh picture" })).toHaveAttribute(
    "href",
    "/v1/auth/discord/link?return_to=%2Faccount%3Fpicture%3Drefresh%23picture",
  );
  await expect(section.getByRole("radio", { name: /Your Microsoft picture/ })).toBeDisabled();
  await expect(section.getByText("Microsoft doesn't share a picture with SLAPorSLOP.")).toBeVisible();

  // Back from the refresh, through the link flow.
  await page.goto("/account?picture=refresh&linked=already#picture");
  await expect(section.getByRole("status")).toHaveText("Signed in again: SLAPorSLOP has your newest picture from there.");
  await expect(page.getByText("That account was already linked.")).toHaveCount(0);

  // SLAPorSLOP couldn't read it: the choice stays as it was.
  await fake(page, "PUT", "/v1/me/avatar", {
    status: 422,
    body: apiError("invalid", "couldn't get your picture from GitHub: try again later", "provider"),
  });
  await section.getByRole("radio", { name: /Your GitHub picture/ }).check();
  await expect(section.getByRole("alert")).toHaveText("couldn't get your picture from GitHub: try again later");
  await expect(section.getByRole("radio", { name: /The letter/ })).toBeChecked();
});

test("Refresh picture for a sign-in with no picture yet goes to it and straight back, and the picture is there", async ({ page }) => {
  // Pia's Google sign-in has no picture yet; the test server's stand-in Google says yes at once
  // (crates/slop-api/examples/e2e_server/main.rs). No "confirm it's you" on the way.
  await signIn(page, "pictured");
  await page.goto("/account#picture");
  const section = page.locator("#picture");
  const google = section.getByRole("radio", { name: /Your Google picture/ });
  await expect(google).toBeDisabled();
  await expect(
    section.getByText("SLAPorSLOP gets this picture when you sign in with Google: press Refresh picture to fetch it now (a quick trip to Google and back)."),
  ).toBeVisible();
  const choice = section.locator(".picture-choice").filter({ hasText: "Your Google picture" });
  await choice.getByRole("link", { name: "Refresh picture" }).click();
  // Through the link flow and back to the account page, signed in as before.
  await expect(page).toHaveURL(new RegExp(`^${server.url}/account\\?picture=refresh&linked=already#picture$`));
  await expect(section.getByRole("status")).toHaveText("Signed in again: SLAPorSLOP has your newest picture from there.");
  await expect(page.getByText("Page not found")).toHaveCount(0);
  await expect(google).toBeEnabled();
  await expect(section.getByText("Updated each time you sign in with Google.")).toBeVisible();
  await google.check();
  await expect(google).toBeChecked();
  await loaded(section.locator(".picture-current img"));
});

test("staff remove a picture from the account's record, saying why", async ({ browser }) => {
  await call("pictured", "PUT", "/v1/me/avatar", { provider: "github" });
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=users&user=${ids.pictured}`);
  await admin.getByText("Fix their profile").click();
  const profile = admin.locator("details").filter({ hasText: "Fix their profile" });
  await loaded(profile.locator(".picture-current img"));
  await profile.getByRole("checkbox", { name: /Remove their picture/ }).check();
  await profile.getByRole("textbox", { name: /Why, as they'll read it/ }).fill("That photo is of someone else.");
  await profile.getByRole("button", { name: "Save the fix" }).click();
  await expect(profile.getByRole("status")).toHaveText("Changed picture. They've been told why.");
  await expect(profile.getByText("No picture: they show the letter.")).toBeVisible();
  expect((await call("pictured", "GET", "/v1/me")).avatar_url).toBeNull();
});
