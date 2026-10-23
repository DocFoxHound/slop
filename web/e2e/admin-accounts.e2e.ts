// SPDX-License-Identifier: AGPL-3.0-or-later
// Accounts in the admin panel: finding one, its record, warning, suspending, reinstating,
// passkey grace, fixing a profile or a team, and making someone staff from their record.
// What changes here is Sam's (who never signs in), Olly's (put back as he was) or made here.
import { call, closePages, newPage, newTeam, ok, tag } from "./admin-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("finding accounts by name or People handle, and an account's record", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin?tab=users");
  const find = page.getByRole("textbox", { name: /Find an account/ });
  await find.fill("nobody-is-called-this");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByText("No accounts match.")).toBeVisible();

  await find.fill("vera-pixels");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("listitem").filter({ hasText: "Vera Okafor" }).getByText("@vera-pixels")).toBeVisible();
  await find.fill("Sid");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("listitem").filter({ hasText: "Sid Suspended" }).getByText("SUSPENDED", { exact: true })).toBeVisible();
  await find.fill("Rae Reviewer");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("listitem").filter({ hasText: "Rae Reviewer" }).getByText("STAFF", { exact: true })).toBeVisible();
  // By ID, too.
  await find.fill(ids.maker2);
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByRole("link", { name: "Bo Builder" }).click();

  // Bo's record: his apps, the report about Pocket Golf and the decision about Delisted Dash.
  await expect(page.getByRole("heading", { name: "Bo Builder", level: 2 })).toBeVisible();
  await expect(page.getByText(new RegExp(`${ids.maker2} · joined .* · US · signs in with nothing linked`))).toBeVisible();
  const apps = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Apps \(/ }) });
  await expect(apps.getByRole("link", { name: "Pocket Golf" })).toBeVisible();
  await expect(apps.getByText("delisted").first()).toBeVisible();
  const reports = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Reports about them/ }) });
  await expect(reports.getByText(/· app · broken · open ·/)).toBeVisible();
  const decisions = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Decisions \(/ }) });
  await expect(decisions.getByText(/delist_app \(misleading\) about App: Delisted Dash/)).toBeVisible();
  // An app opens its own record.
  await apps.getByRole("link", { name: "Pocket Golf" }).click();
  await expect(page.getByRole("heading", { name: "Pocket Golf", level: 2 })).toBeVisible();
  await page.goBack();
  await page.getByRole("link", { name: "← Find another account" }).click();
  await expect(find).toBeVisible();

  // A suspended account offers to reinstate it instead.
  await page.goto(`/admin?tab=users&user=${ids.suspended}`);
  await expect(page.getByText("SUSPENDED", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reinstate" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Suspend" })).toHaveCount(0);
});

test("warning, suspending and reinstating an account, and passkey grace", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto(`/admin?tab=users&user=${ids.sam}`);
  await expect(page.getByRole("heading", { name: "Sam Seed", level: 2 })).toBeVisible();
  const decisions = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Decisions \(/ }) });

  // A warning: a rule and the facts.
  await page.getByRole("button", { name: "Warn" }).click();
  const warn = page.getByRole("form", { name: "Warn: warn" });
  await expect(warn.getByRole("button", { name: "Warn" })).toBeDisabled();
  await warn.getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Spam" });
  await warn.getByRole("textbox", { name: /The facts/ }).fill("Posted the same link in forty reviews.");
  await warn.getByRole("button", { name: "Warn" }).click();
  await expect(decisions.getByText(/warn \(spam\) about Sam Seed/)).toBeVisible();

  // A suspension, then reinstating (it needs a real reason).
  await page.getByRole("button", { name: "Suspend" }).click();
  const suspend = page.getByRole("form", { name: "Suspend: suspend_account" });
  await suspend.getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Harassment, hate and bullying" });
  await suspend.getByRole("textbox", { name: /The facts/ }).fill("Kept messaging a player who asked them to stop.");
  await suspend.getByRole("button", { name: "Suspend" }).click();
  await expect(page.getByText("SUSPENDED", { exact: true })).toBeVisible();
  await expect(decisions.getByText(/suspend_account \(harassment\) about Sam Seed/)).toBeVisible();
  await page.getByRole("button", { name: "Reinstate" }).click();
  const why = page.getByRole("textbox", { name: "Reinstate: reason" });
  await why.fill("Sorry");
  await page.getByRole("button", { name: "Reinstate" }).click();
  await expect(page.getByRole("alert")).toHaveText("say why in 10 to 1000 characters");
  await why.fill("They explained; it was a misunderstanding.");
  await page.getByRole("button", { name: "Reinstate" }).click();
  await expect(page.getByText("SUSPENDED", { exact: true })).toHaveCount(0);
  await expect(decisions.getByText(/suspend_account \(harassment\) about Sam Seed .* · reversed/)).toBeVisible();

  // Passkey grace: how staff checked it's them.
  await page.getByRole("button", { name: "Passkey grace" }).click();
  const grace = page.getByRole("textbox", { name: "Passkey grace: reason" });
  await grace.fill("Asked");
  await page.getByRole("button", { name: "Passkey grace" }).click();
  await expect(page.getByRole("alert")).toHaveText("say how you checked it's them, in 10 to 1000 characters");
  await grace.fill("Video call; matched the ID on their payout account.");
  await page.getByRole("button", { name: "Passkey grace" }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  const audit = await ok("admin", "GET", "/v1/admin/audit?q=account.passkey_grace");
  expect(audit.some((e: { subject: string }) => e.subject === ids.sam)).toBe(true);
});

test("staff can't act on their own account, and a suspension may wait for a second person", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto(`/admin?tab=users&user=${ids.admin}`);
  await expect(page.getByText("STAFF", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Warn" }).click();
  await page.getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Spam" });
  await page.getByRole("textbox", { name: /The facts/ }).fill("Testing on myself.");
  await page.getByRole("form", { name: "Warn: warn" }).getByRole("button", { name: "Warn" }).click();
  await expect(page.getByRole("alert")).toHaveText("not on your own account");

  // Where Slop requires two people, asking records it and says so.
  await page.route("**/v1/admin/users/*/decide", (route) =>
    route.fulfill({ status: 202, json: { pending_action: "00000000-0000-7000-8000-000000000003", approvals_still_needed: 1 } }),
  );
  await page.goto(`/admin?tab=users&user=${ids.sam}`);
  await page.getByRole("button", { name: "Suspend" }).click();
  const suspend = page.getByRole("form", { name: "Suspend: suspend_account" });
  await suspend.getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Spam" });
  await suspend.getByRole("textbox", { name: /The facts/ }).fill("Spam in every review.");
  await suspend.getByRole("button", { name: "Suspend" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Recorded. It happens once another staff member approves it under Waiting for approval." }),
  ).toBeVisible();
  await expect(page.getByText("SUSPENDED", { exact: true })).toHaveCount(0);
});

test("fixing a display name, a People page and a team, each with a reason", async ({ browser }) => {
  const team = `adm-fix-${tag()}`;
  await newTeam("overdue", team, "Olly's Crew");
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=users&user=${ids.overdue}`);

  // The display name: nothing changes without a change and a reason.
  await admin.getByText("Fix their profile").click();
  const profile = admin.locator("details").filter({ hasText: "Fix their profile" });
  const save = profile.getByRole("button", { name: "Save the fix" });
  await expect(save).toBeDisabled();
  await expect(profile.getByText("They have no People page.")).toBeVisible();
  const name = profile.getByRole("textbox", { name: "Display name" });
  await name.fill("Olly ✨ Overdue ✨");
  await profile.getByRole("textbox", { name: /Why, as they'll read it/ }).fill("Emoji in names are for People pages.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(profile.getByRole("status")).toHaveText("Changed display name. They've been told why.");
  await expect(admin.getByRole("heading", { name: "Olly ✨ Overdue ✨", level: 2 })).toBeVisible();
  // A name the API refuses says why.
  await name.fill("   ");
  await profile.getByRole("textbox", { name: /Why, as they'll read it/ }).fill("Trying a blank name.");
  await save.click();
  await expect(profile.getByRole("alert")).toBeVisible();
  // Back as it was.
  await name.fill("Olly Overdue");
  await profile.getByRole("textbox", { name: /Why, as they'll read it/ }).fill("Put back after the test.");
  await save.click();
  await expect(admin.getByRole("heading", { name: "Olly Overdue", level: 2 })).toBeVisible();
  const olly = await newPage(browser);
  await signIn(olly, "overdue");
  await olly.goto("/notifications");
  await expect(olly.getByText("Emoji in names are for People pages.")).toBeVisible();

  // A team he runs.
  await admin.getByText("Fix their team: Olly's Crew").click();
  const fix = admin.locator("details").filter({ hasText: "Fix their team" });
  await fix.getByRole("textbox", { name: "Team name" }).fill("Olly's Friendly Crew");
  await fix.getByRole("textbox", { name: "About", exact: true }).fill("Friendly games, every weekend.");
  await fix.getByRole("textbox", { name: /Why, as the team's owners will read it/ }).fill("The old name copied another team's.");
  await fix.getByRole("button", { name: "Save the fix" }).click();
  await expect(admin.getByText("Fix their team: Olly's Friendly Crew")).toBeVisible();
  await olly.goto(`/teams/${team}`);
  await expect(olly.getByRole("heading", { name: /Olly's Friendly Crew/, level: 1 })).toBeVisible();
  await expect(olly.getByText("Friendly games, every weekend.")).toBeVisible();
});

test("fixing a creative's People page", async ({ page }) => {
  await signIn(page, "admin");
  const before = await ok("admin", "GET", `/v1/admin/users/${ids.pendingCreative}/profile`);
  await page.goto(`/admin?tab=users&user=${ids.pendingCreative}`);
  await page.getByText("Fix their profile").click();
  const profile = page.locator("details").filter({ hasText: "Fix their profile" });
  await expect(profile.getByRole("link", { name: "@remy-sounds" })).toBeVisible();
  await profile.getByRole("textbox", { name: "Headline" }).fill("Sound design for small games");
  await profile.getByRole("textbox", { name: "Location" }).fill("Somewhere quiet");
  await profile.getByRole("textbox", { name: /Why, as they'll read it/ }).fill("The headline named another studio.");
  await profile.getByRole("button", { name: "Save the fix" }).click();
  await expect(profile.getByRole("status")).toHaveText("Changed headline, location. They've been told why.");
  // Back as it was.
  await ok("admin", "PATCH", `/v1/admin/users/${ids.pendingCreative}/profile`, {
    creative: { headline: before.creative.headline, location: before.creative.location },
    reason: "Put back after the test.",
  });
});

test("making someone staff from their record, and taking them off again", async ({ browser }) => {
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=users&user=${ids.overdue}`);
  const staff = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "Staff", exact: true }) });
  await expect(staff.getByText("Not staff.")).toBeVisible();
  // An owner may give the owner role.
  await expect(staff.getByRole("combobox", { name: "Role to give" }).locator("option")).toHaveText([
    "reviewer",
    "support",
    "moderator",
    "finance",
    "security",
    "owner",
  ]);
  await staff.getByRole("combobox", { name: "Role to give" }).selectOption("support");
  await staff.getByRole("button", { name: "Make staff" }).click();
  await expect(staff.getByText(/Olly Overdue is staff: support\./)).toBeVisible();
  await expect(admin.getByText("STAFF", { exact: true })).toBeVisible();
  // A role he has can't be given again; another can.
  await expect(staff.getByRole("button", { name: "Give this role" })).toBeDisabled();
  await staff.getByRole("combobox", { name: "Role to give" }).selectOption("reviewer");
  await staff.getByRole("button", { name: "Give this role" }).click();
  await expect(staff.getByText(/Olly Overdue is staff: (reviewer, support|support, reviewer)\./)).toBeVisible();

  // Security staff change staff under Staff: reading an account's details is moderation.
  const security = await newPage(browser);
  await signIn(security, "security");
  await security.goto(`/admin?tab=users&user=${ids.overdue}`);
  await expect(security.getByRole("alert")).toContainText("you can't do this");
  // Support staff can open the account but not make staff.
  const support = await newPage(browser);
  await signIn(support, "support");
  await support.goto(`/admin?tab=users&user=${ids.overdue}`);
  await expect(support.getByRole("heading", { name: "Olly Overdue", level: 2 })).toBeVisible();
  await expect(support.getByRole("heading", { name: "Staff", exact: true })).toHaveCount(0);
  expect((await call("support", "POST", `/v1/admin/staff/${ids.overdue}/remove`, { reason: "Support tries to remove." })).status).toBe(403);

  // Taking him off staff needs a reason of ten characters or more.
  await staff.getByRole("button", { name: "Remove from staff" }).click();
  const reason = staff.getByRole("textbox", { name: "Remove from staff: reason" });
  await reason.fill("Done.");
  await staff.getByRole("button", { name: "Remove from staff" }).click();
  await expect(staff.getByRole("alert")).toHaveText("say why in 10 to 2000 characters");
  await reason.fill("He was only staff for the browser test.");
  await staff.getByRole("button", { name: "Remove from staff" }).click();
  await expect(staff.getByText("Not staff.")).toBeVisible();
  await expect(admin.getByText("STAFF", { exact: true })).toHaveCount(0);
});
