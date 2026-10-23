// SPDX-License-Identifier: AGPL-3.0-or-later
// The seed (fixtures.ts lists it) is there: every account signs in to its main page without an
// error, and the main seeded things show where they should.
import { expect, ids, signIn, test, type Who } from "./fixtures";

const homes: Array<[Who, string, string | RegExp]> = [
  ["maker", "/publish", "Your apps"],
  ["player", "/library", "Library"],
  ["admin", "/admin", "Admin"],
  ["overdue", "/publish", "Your apps"],
  ["teen", "/friends", /Friends/],
  ["maker2", "/publish", "Your apps"],
  ["creative", "/people/me", "Your People page"],
  ["client", "/people/jobs", "Your jobs"],
  ["pendingCreative", "/people/me", "Your People page"],
  ["moderator", "/admin", "Admin"],
  ["reviewer", "/admin", "Admin"],
  ["support", "/admin", "Admin"],
  ["finance", "/admin", "Admin"],
  ["security", "/admin", "Admin"],
  ["suspended", "/account", "Your account"],
];

for (const [who, path, heading] of homes) {
  test(`${who} signs in and ${path} loads`, async ({ page }) => {
    await signIn(page, who);
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
    await page.waitForLoadState("networkidle");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await page.goto("/notifications");
    await expect(page.getByRole("heading", { name: "Notifications", level: 1 })).toBeVisible();
    await page.waitForLoadState("networkidle");
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
}

test("the teen is in the under-18 view, and the suspended account is told", async ({ browser }) => {
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto("/friends");
  await expect(teen.getByText(/In the under-18 view nobody else can message you/)).toBeVisible();
  await expect(teen.getByText("Pat Player").first()).toBeVisible();

  const sid = await browser.newPage();
  await signIn(sid, "suspended");
  await sid.goto("/");
  await expect(sid.getByText("Your account is suspended.")).toBeVisible();
});

test("the listed app is in the store with its pictures, reviews, bugs and bounties", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Lantern Keep").first()).toBeVisible();
  await page.goto(`/app/${ids.listedApp}`);
  await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();
  await expect(page.getByRole("region", { name: /Lantern Keep: screenshots and videos/ })).toBeVisible();
  await expect(page.getByText(/Forty hours in and still finding secrets/)).toBeVisible();
  await expect(page.getByText(/1\.0\.1 fixes the level 3 crash/)).toBeVisible();
  await expect(page.locator("#bounties").getByText("Controller support")).toBeVisible();
  await expect(page.getByText("Lantern Keep for Linux").first()).toBeVisible();
  await expect(page.getByText("More Lanterns").first()).toBeVisible();

  await signIn(page, "player");
  await page.goto(`/app/${ids.listedApp}`);
  await expect(page.locator("#bug-reports").getByText("Fog covers the menu after alt-tab")).toBeVisible();
  await page.goto(`/bounties/${ids.claimedBounty}`);
  await expect(page.getByRole("heading", { name: "A Linux build", level: 1 })).toBeVisible();
  await expect(page.getByText("Lantern Keep for Linux").first()).toBeVisible();
});

test("the teams show their members, and the job its conversation", async ({ browser }) => {
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/teams/${ids.inviteTeam}`);
  await expect(pat.getByRole("heading", { name: /Night Owls/, level: 1 })).toBeVisible();
  await expect(pat.getByText("Bo Builder").first()).toBeVisible();
  await pat.goto("/teams");
  await expect(pat.getByText("Night Owls").first()).toBeVisible();

  const cleo = await browser.newPage();
  await signIn(cleo, "client");
  await cleo.goto(`/jobs/${ids.deliveredJob}`);
  await expect(cleo.getByRole("heading", { name: "Knight sprite sheet", level: 1 })).toBeVisible();
  await expect(cleo.getByText(/Godot 4\. Idle would be lovely/)).toBeVisible();
  await cleo.goto("/people/jobs");
  await expect(cleo.getByText("Tavern floor tiles").first()).toBeVisible();

  const visitor = await browser.newPage();
  await visitor.goto(`/people/${ids.creativeHandle}`);
  await expect(visitor.getByRole("heading", { name: "Vera Okafor", level: 1 })).toBeVisible();
});

test("staff see each queue the seed filled", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin");
  await expect(page.getByRole("heading", { name: "Needs you" })).toBeVisible();
  for (const queue of [
    /Reports$/,
    /Appeals$/,
    /Releases to review$/,
    /Pictures to review$/,
    /Changes to review$/,
    /portfolios to review$/i,
    /People problems$/,
    /Review disputes$/,
    /Waiting for a second approval$/,
    /Server alerts$/,
    /Possible abuse$/,
    /Ready for the store$/,
    /re-scan flagged$/,
  ]) {
    await expect(page.getByRole("link", { name: queue }).first()).toBeVisible();
  }
});
