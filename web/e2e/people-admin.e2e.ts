// SPDX-License-Identifier: AGPL-3.0-or-later
// Staff and People: the portfolio queue, and fixing what someone wrote on their People page or
// team. (Problems and review disputes are decided in people-job-steps and people-jobs, on those
// tests' own jobs.) The seed's one waiting portfolio (Remy's) must stay waiting for seed.e2e, and
// the fake GitHub serves no portfolio files to make another, so its approve and turn-down
// requests are answered here instead of by the server.
import { expect, ids, signIn, test } from "./fixtures";
import { call } from "./people-helpers";

test("staff look at a first portfolio's works, then approve or turn it down", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin");
  await page.getByRole("link", { name: /People portfolios to review$/ }).click();
  const row = page.locator("li.panel").filter({ hasText: "Remy Pending" });
  await expect(row).toContainText(`@${ids.pendingCreativeHandle}`);
  await expect(row).toContainText("AI: NONE");
  await expect(row).toContainText("FIRST PORTFOLIO");
  await expect(row.getByRole("link", { name: "Remy Pending" })).toHaveAttribute("href", `/people/${ids.pendingCreativeHandle}`);
  await expect(row.getByRole("link", { name: "ccccccc" })).toHaveAttribute(
    "href",
    `https://github.com/e2e/${ids.pendingCreativeHandle}-portfolio/tree/${"c".repeat(40)}/.slop`,
  );
  // The avatar, the banner and three works.
  await expect(row.getByRole("img")).toHaveCount(5);

  const asked: Array<{ url: string; body: unknown }> = [];
  await page.route("**/v1/admin/portfolio/*/*", async (route) => {
    asked.push({ url: route.request().url(), body: route.request().postDataJSON() });
    await route.fulfill({ json: {} });
  });
  await row.getByRole("button", { name: "Approve portfolio" }).click();
  await expect.poll(() => asked.length).toBe(1);
  expect(asked[0].url).toMatch(/\/v1\/admin\/portfolio\/[0-9a-f-]{36}\/approve$/);

  await row.getByRole("button", { name: "Turn down" }).click();
  const reason = row.getByRole("textbox", { name: "Turn down: reason" });
  await expect(row.getByRole("button", { name: "Turn down" })).toBeDisabled();
  await reason.fill("Two works are someone else's art.");
  await row.getByRole("button", { name: "Turn down" }).click();
  await expect.poll(() => asked.length).toBe(2);
  expect(asked[1].url).toMatch(/\/reject$/);
  expect(JSON.stringify(asked[1].body)).toContain("Two works are someone else's art.");
});

test("staff fix someone's People page and team, with a reason they're told", async ({ page }) => {
  // Fin has a People page and runs a team of his own.
  await call("finance", "PUT", "/v1/me/creative", {
    handle: "fin-tunes",
    name: "Fin Tunes",
    kind: "individual",
    team_size: null,
    headline: "Buy cheap gold at example.com",
    about_md: "Chiptune.",
    location: "",
    skills: ["music"],
    ai_use: "none",
    ai_note: "",
    trader: false,
    taking_work: true,
    turnaround: "",
    services: [],
    accent: "sun",
    sections: ["portfolio", "services", "reviews", "worked_on", "about"],
    links: [{ label: "Shop", url: "https://example.com/gold" }],
    confirm_adult: true,
  });
  await call("finance", "POST", "/v1/teams", { slug: "fin-club", name: "Fin Club", kind: "player", about: "Cheap gold here", join_policy: "invite" });

  await signIn(page, "admin");
  await page.goto("/admin?tab=users&q=Fin%20Finance");
  await page.getByRole("link", { name: "Fin Finance" }).click();
  await page.getByText("Fix their profile").click();
  const profile = page.locator("details").filter({ hasText: "Fix their profile" });
  await expect(profile.getByRole("link", { name: "@fin-tunes" })).toHaveAttribute("href", "/people/fin-tunes");
  const save = profile.getByRole("button", { name: "Save the fix" });
  await expect(save).toBeDisabled();
  await profile.getByRole("textbox", { name: "Headline" }).fill("Chiptune for small games");
  await profile.getByRole("checkbox", { name: "Remove every link" }).check();
  await expect(save).toBeDisabled();
  await profile.getByRole("textbox", { name: "Why, as they'll read it" }).fill("The headline and link advertised a gold seller (spam).");
  await save.click();
  await expect(profile.getByRole("status")).toContainText("They've been told why.");
  const fixed = await call("finance", "GET", "/v1/me/creative");
  expect(fixed.headline).toBe("Chiptune for small games");
  expect(fixed.links).toEqual([]);

  await page.getByText("Fix their team: Fin Club").click();
  const team = page.locator("details").filter({ hasText: "Fix their team: Fin Club" });
  await team.getByRole("textbox", { name: "About" }).fill("A chiptune listening club.");
  await team.getByRole("textbox", { name: "Why, as the team's owners will read it" }).fill("The description advertised a gold seller (spam).");
  await team.getByRole("button", { name: "Save the fix" }).click();
  // Saved: the reason is cleared and there's nothing left to save.
  await expect(team.getByRole("textbox", { name: "Why, as the team's owners will read it" })).toHaveValue("");
  await expect(team.getByRole("button", { name: "Save the fix" })).toBeDisabled();
  await expect(team.getByRole("alert")).toHaveCount(0);
  expect((await call("finance", "GET", "/v1/teams/fin-club")).about).toBe("A chiptune listening club.");
});
