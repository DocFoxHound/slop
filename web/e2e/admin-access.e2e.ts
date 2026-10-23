// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel's door and its tabs: who gets in, what each role may see, the Home page's
// counts, alerts and server checks, the audit log, AI reviewer tokens, staff roles and the
// actions waiting for a second approval.
//
// Making tokens, changing staff and approving actions need a sign-in from the last ten minutes
// (SECURITY §4.3), and the seeded sessions are as old as the test server: these run first.
import { adminToken, call, closePages, newPage, ok, tag, withToken } from "./admin-helpers";
import type { Locator } from "@playwright/test";

import { expect, ids, server, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("AI reviewer tokens: made with a scope and an expiry, shown once, listed, and revoked", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin?tab=ai");
  const name = `Picture checks ${tag()}`;
  await page.getByRole("textbox", { name: "Name" }).fill(name);
  await page.getByRole("combobox", { name: "Can" }).selectOption({ label: "Also approve and reject (review)" });
  await page.getByRole("combobox", { name: "Expires after" }).selectOption({ label: "30 days" });
  await page.getByRole("button", { name: "Make token" }).click();

  // The secret, once, with the prompt and the MCP command that carry it.
  await expect(page.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
  const secret = (await page.locator("pre.code-block").first().innerText()).trim();
  expect(secret).toMatch(/^slopadm_/);
  await expect(page.getByText(`${server.url}/v1/admin/review/guide`)).toBeVisible();
  await expect(page.getByText(`Authorization: Bearer ${secret}`).first()).toBeVisible();
  await expect(
    page.getByText(`claude mcp add --transport http slop-admin ${server.url}/v1/admin/mcp --header "Authorization: Bearer ${secret}"`),
  ).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Name" })).toHaveValue("");
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText("Copy it now: Slop won't show this token again.")).toHaveCount(0);

  // It works, it's listed with its scope and whose it is, and revoking ends it.
  expect((await withToken(secret, "GET", "/v1/admin/status")).status).toBe(200);
  const row = page.getByRole("listitem").filter({ hasText: name });
  await expect(row.getByText("REVIEW", { exact: true })).toBeVisible();
  await expect(row.getByText(/for Ada Admin · made .* · expires/)).toBeVisible();
  await row.getByRole("button", { name: "Revoke" }).click();
  await expect(row.getByText("REVOKED", { exact: true })).toBeVisible();
  await expect(row.getByRole("button", { name: "Revoke" })).toHaveCount(0);
  expect((await withToken(secret, "GET", "/v1/admin/status")).status).toBe(404);
});

test("AI reviewer tokens: expired ones say so, a refusal is shown, and only security staff see them", async ({ browser }) => {
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  // An expired token (none can be made: the shortest lasts a week).
  await admin.route("**/v1/admin/tokens", async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const res = await route.fetch();
    const list = await res.json();
    list.unshift({
      id: "00000000-0000-7000-8000-000000000001",
      name: "Old picture checker",
      scope: "read",
      admin: "Ada Admin",
      created_at: "2025-01-01T00:00:00Z",
      expires_at: "2025-01-08T00:00:00Z",
      last_used_at: null,
      revoked_at: null,
    });
    await route.fulfill({ response: res, json: list });
  });
  await admin.goto("/admin?tab=ai");
  const old = admin.getByRole("listitem").filter({ hasText: "Old picture checker" });
  await expect(old.getByText("EXPIRED", { exact: true })).toBeVisible();
  await expect(old.getByText(/never used/)).toBeVisible();
  await expect(old.getByRole("button", { name: "Revoke" })).toHaveCount(0);

  // Making one needs a recent sign-in: the refusal shows, and no secret does.
  await admin.route("**/v1/admin/tokens", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 403,
          json: { error: { code: "step_up_required", message: "confirm it's you first: sign in again, or use a passkey" } },
        })
      : route.fallback(),
  );
  await admin.getByRole("textbox", { name: "Name" }).fill("Late night checker");
  await admin.getByRole("button", { name: "Make token" }).click();
  await expect(admin.getByRole("alert")).toHaveText("confirm it's you first: sign in again, or use a passkey");
  await expect(admin.getByText("Copy it now")).toHaveCount(0);

  // A reviewer can't see or make them.
  const reviewer = await newPage(browser);
  await signIn(reviewer, "reviewer");
  await reviewer.goto("/admin?tab=ai");
  await expect(reviewer.getByRole("alert")).toContainText("you can't do this");
  await reviewer.getByRole("textbox", { name: "Name" }).fill("Not allowed");
  await reviewer.getByRole("button", { name: "Make token" }).click();
  await expect(reviewer.getByRole("alert").filter({ hasText: "you can't do this" })).toHaveCount(2);

  // Security staff can.
  const security = await newPage(browser);
  await signIn(security, "security");
  await security.goto("/admin?tab=ai");
  await security.getByRole("textbox", { name: "Name" }).fill(`Security's reader ${tag()}`);
  await security.getByRole("button", { name: "Make token" }).click();
  await expect(security.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
  await security.getByRole("button", { name: "Done" }).click();
  await expect(security.getByText(/for Sec Security/).first()).toBeVisible();
});

test("an AI agent's staff change waits for a person, who approves one and turns down another", async ({ page }) => {
  const olly = ids.overdue;
  // A moderate token asks for two staff changes for Olly: each waits for a person.
  const { token } = await adminToken(`Staffing agent ${tag()}`, "moderate");
  const asked = await withToken(token, "POST", `/v1/admin/staff/${olly}/roles`, { role: "support", grant: true });
  expect(asked.status).toBe(202);
  expect((await withToken(token, "POST", `/v1/admin/staff/${olly}/roles`, { role: "finance", grant: true })).status).toBe(202);

  await signIn(page, "admin");
  await page.goto("/admin");
  await expect(page.getByRole("link", { name: /^\d+ Waiting for a second approval$/ })).toBeVisible();
  await page.getByRole("link", { name: /Waiting for a second approval$/ }).click();
  const waiting = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Waiting for approval/ }) });
  await expect(waiting.getByText("Sensitive actions need 1 different staff members")).toBeVisible();
  const support = waiting.getByRole("listitem").filter({ hasText: "Give the support role to Olly Overdue" });
  const finance = waiting.getByRole("listitem").filter({ hasText: "Give the finance role to Olly Overdue" });
  await expect(support.getByText(/Asked by AI reviewer "Staffing agent .*" for Ada Admin .* · 0 of 1 approvals/)).toBeVisible();
  // The seeded request (Sam) stays where it is.
  await expect(waiting.getByRole("listitem").filter({ hasText: "Sam Seed" })).toHaveCount(1);

  await finance.getByRole("button", { name: "Turn down" }).click();
  await expect(finance).toHaveCount(0);
  await support.getByRole("button", { name: "Approve" }).click();
  await expect(support).toHaveCount(0);

  // Olly is support staff now, and the history says what happened.
  await page.getByRole("tab", { name: "Staff" }).click();
  await expect(page.getByRole("checkbox", { name: "Olly Overdue: support" })).toBeChecked();
  await expect(page.getByRole("checkbox", { name: "Olly Overdue: finance" })).not.toBeChecked();
  const history = page.getByRole("listitem").filter({ hasText: "Olly Overdue" });
  await expect(history.filter({ hasText: "Give the support role to Olly Overdue" }).getByText("DONE", { exact: true })).toBeVisible();
  await expect(history.filter({ hasText: "Give the finance role to Olly Overdue" }).getByText("CANCELLED", { exact: true })).toBeVisible();

  // Back as he was.
  await ok("admin", "POST", `/v1/admin/staff/${olly}/remove`, { reason: "Only for the admin browser tests." });
});

test("waiting for approval: a refusal and a recorded request are both shown", async ({ page }) => {
  await signIn(page, "admin");
  // With two approvals required, asking for one records it and says so (this server needs one).
  await page.route("**/v1/admin/staff/*/roles", (route) =>
    route.fulfill({
      status: 202,
      json: {
        pending_action: "00000000-0000-7000-8000-000000000002",
        approvals_still_needed: 1,
        message: "Recorded. It happens once another staff member approves it in the admin queue.",
      },
    }),
  );
  await page.goto("/admin?tab=staff");
  await page.getByRole("checkbox", { name: "Sue Support: reviewer" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Recorded. It happens once another staff member approves it under Waiting for approval." }),
  ).toBeVisible();

  // Whoever asked can't be the second approval.
  await page.route("**/v1/admin/actions/*/approve", (route) =>
    route.fulfill({ status: 409, json: { error: { code: "conflict", message: "you already approved this: it needs someone else" } } }),
  );
  await page.goto("/admin?tab=queue");
  const waiting = page.locator("section").filter({ has: page.getByRole("heading", { name: /^Waiting for approval/ }) });
  await waiting.getByRole("listitem").filter({ hasText: "Sam Seed" }).getByRole("button", { name: "Approve" }).click();
  await expect(waiting.getByRole("alert")).toHaveText("you already approved this: it needs someone else");
  await expect(waiting.getByRole("listitem").filter({ hasText: "Sam Seed" })).toHaveCount(1);
});

test("the Staff tab: owners give and take roles, add and remove staff; others can't", async ({ browser }) => {
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=staff");
  await expect(admin.getByText(/need 1 different staff member: they happen at once/)).toBeVisible();

  // Add Olly by his profile link, give him a second role, take the first away.
  const add = admin.getByRole("textbox", { name: "Account ID or profile link" });
  await add.fill("Olly Overdue");
  await admin.getByRole("combobox", { name: "Role" }).selectOption("reviewer");
  await admin.getByRole("button", { name: "Add to staff" }).click();
  await expect(admin.getByRole("alert")).toHaveText("Paste the account's ID, or the link to their profile (…/u/…).");
  await add.fill(`${server.url}/u/${ids.overdue}`);
  await admin.getByRole("button", { name: "Add to staff" }).click();
  await expect(admin.getByRole("checkbox", { name: "Olly Overdue: reviewer" })).toBeChecked();
  await expect(add).toHaveValue("");
  await admin.getByRole("checkbox", { name: "Olly Overdue: support" }).click();
  await expect(admin.getByRole("checkbox", { name: "Olly Overdue: support" })).toBeChecked();
  await admin.getByRole("checkbox", { name: "Olly Overdue: reviewer" }).click();
  await expect(admin.getByRole("checkbox", { name: "Olly Overdue: reviewer" })).not.toBeChecked();
  const taken = admin.getByRole("listitem").filter({ hasText: "Take away the reviewer role from Olly Overdue" });
  await expect(taken.getByText("DONE", { exact: true })).toBeVisible();

  // Removing needs a real reason.
  await admin.getByRole("button", { name: "Remove Olly Overdue" }).click();
  const reason = admin.getByRole("textbox", { name: "Remove Olly Overdue: reason" });
  await reason.fill("Done");
  await admin.getByRole("button", { name: "Remove Olly Overdue" }).click();
  await expect(admin.getByRole("alert")).toHaveText("say why in 10 to 2000 characters");
  await reason.fill("Only staff for the admin browser tests.");
  await admin.getByRole("button", { name: "Remove Olly Overdue" }).click();
  await expect(admin.getByRole("link", { name: "Olly Overdue" })).toHaveCount(0);

  // Security staff manage roles, but only an owner gives the owner role.
  const security = await newPage(browser);
  await signIn(security, "security");
  await security.goto("/admin?tab=staff");
  await expect(security.getByRole("checkbox", { name: "Rae Reviewer: owner" })).toBeDisabled();
  await expect(security.getByRole("checkbox", { name: "Rae Reviewer: support" })).toBeEnabled();
  await expect(security.getByRole("button", { name: "Remove Rae Reviewer" })).toBeVisible();

  // A reviewer only looks.
  const reviewer = await newPage(browser);
  await signIn(reviewer, "reviewer");
  await reviewer.goto("/admin?tab=staff");
  await expect(reviewer.getByRole("checkbox", { name: "Sue Support: support" })).toBeDisabled();
  await expect(reviewer.getByRole("checkbox", { name: "Sue Support: support" })).toBeChecked();
  await expect(reviewer.getByRole("button", { name: /^Remove / })).toHaveCount(0);
  await expect(reviewer.getByRole("button", { name: "Add to staff" })).toHaveCount(0);
  // Nor can it through the API.
  expect((await call("reviewer", "POST", `/v1/admin/staff/${ids.overdue}/roles`, { role: "reviewer", grant: true })).status).toBe(403);
});

test("players and signed-out visitors find no admin panel", async ({ browser }) => {
  const visitor = await newPage(browser);
  await visitor.goto("/admin");
  await expect(visitor.getByText("Page not found.")).toBeVisible();
  await expect(visitor.getByRole("heading", { name: "Admin" })).toHaveCount(0);

  const player = await newPage(browser);
  await signIn(player, "player");
  await player.goto("/admin?tab=queue");
  await expect(player.getByText("Page not found.")).toBeVisible();
  // The API doesn't let on that it's there.
  expect((await call("player", "GET", "/v1/admin/queue")).status).toBe(404);
  expect((await call(null, "GET", "/v1/admin/queue")).status).toBe(404);
});

test("every tab opens for an owner", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin");
  const tabs: Array<[string, Locator]> = [
    ["Review queue", page.getByRole("heading", { name: /^Reports \(/ })],
    ["Apps", page.getByRole("link", { name: "Lantern Keep", exact: true })],
    ["Accounts", page.getByRole("textbox", { name: /Find an account/ })],
    ["Front page", page.getByText("Featured on the front page")],
    ["Money", page.getByRole("heading", { name: /^Donation pauses/ })],
    ["Audit log", page.getByRole("columnheader", { name: "When" })],
    ["AI messages", page.getByRole("link", { name: "Release upload times out" })],
    ["AI reviewers", page.getByRole("heading", { name: "New token" })],
    ["Staff", page.getByRole("heading", { name: "Sensitive actions" })],
    ["Terms & policies", page.getByRole("heading", { name: "Terms of Service" })],
    ["Legal requests", page.getByRole("button", { name: "Log a request" })],
    ["Home", page.getByRole("heading", { name: "Needs you" })],
  ];
  for (const [tab, shows] of tabs) {
    await page.getByRole("tab", { name: tab, exact: true }).click();
    await expect(page.getByRole("tab", { name: tab, exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(shows).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
  }
});

test("staff with one role see what their role allows, and refusals for the rest", async ({ browser }) => {
  const reviewer = await newPage(browser);
  await signIn(reviewer, "reviewer");
  // Finding accounts is open to every staff member; opening one is for moderation.
  await reviewer.goto("/admin?tab=users&q=Mia%20Maker");
  await reviewer.getByRole("link", { name: "Mia Maker" }).click();
  await expect(reviewer.getByRole("alert")).toContainText("you can't do this");
  await reviewer.goto("/admin?tab=legal");
  await expect(reviewer.getByRole("alert")).toContainText("you can't do this");
  await reviewer.goto("/admin?tab=money");
  await expect(reviewer.getByText("The payments record is for staff with the finance role.")).toBeVisible();
  // Their own work is there.
  await reviewer.goto("/admin?tab=queue");
  await expect(reviewer.getByRole("heading", { name: /^Releases to review/ })).toBeVisible();

  const finance = await newPage(browser);
  await signIn(finance, "finance");
  await finance.goto("/admin?tab=money");
  await expect(finance.getByText(/Payments are live through Stripe/)).toBeVisible();
  await expect(finance.getByRole("heading", { name: "The last 12 months" })).toBeVisible();
  await expect.poll(() => finance.locator("table.audit").first().locator("tbody tr").count()).toBeGreaterThan(0);

  const support = await newPage(browser);
  await signIn(support, "support");
  await support.goto("/admin?tab=ai");
  await expect(support.getByRole("alert")).toContainText("you can't do this");
});

test("Home counts what's waiting, links to it, and finds apps and accounts", async ({ page }) => {
  await signIn(page, "admin");
  const queue = await ok("admin", "GET", "/v1/admin/queue");
  const actions = await ok("admin", "GET", "/v1/admin/actions");
  const ops = await ok("admin", "GET", "/v1/admin/ops");
  await page.goto("/admin");
  const counts: Array<[number, string]> = [
    [ops.length, "Server alerts"],
    [actions.actions.length, "Waiting for a second approval"],
    [queue.reports.length, "Reports"],
    [queue.appeals.length, "Appeals"],
    [queue.rescans.length, "Live releases a re-scan flagged"],
    [queue.releases.length, "Releases to review"],
    [queue.changes.length, "Changes to review"],
    [queue.media.length, "Pictures to review"],
    [queue.portfolios.length, "People portfolios to review"],
    [queue.problems.length, "People problems"],
    [queue.review_disputes.length, "Review disputes"],
    [queue.abuse.length, "Possible abuse"],
    [queue.ready_to_list.length, "Ready for the store"],
  ];
  for (const [n, label] of counts) {
    expect(n, label).toBeGreaterThan(0);
    await expect(page.getByRole("link", { name: `${n} ${label}`, exact: true })).toBeVisible();
  }
  // Nothing paused, so nothing about pauses.
  await expect(page.getByRole("link", { name: /Makers asking for donations back/ })).toHaveCount(0);

  // Each goes to its part of the queue.
  await page.getByRole("link", { name: /^\d+ Appeals$/ }).click();
  await expect(page).toHaveURL(/tab=queue#appeals$/);
  await expect(page.getByRole("heading", { name: /^Appeals \(/ })).toBeInViewport();
  await page.goBack();
  await page.getByRole("link", { name: /^\d+ Reports$/ }).click();
  await expect(page.getByRole("heading", { name: /^Reports \(/ })).toBeInViewport();

  // Find: an account.
  await page.getByRole("tab", { name: "Home" }).click();
  await page.getByRole("textbox", { name: /Find/ }).fill("Bo Builder");
  await page.getByRole("button", { name: "Accounts" }).click();
  await expect(page).toHaveURL(/tab=users&q=Bo(\+|%20)Builder/);
  await expect(page.getByRole("link", { name: "Bo Builder" })).toBeVisible();
  // And an app, with Enter.
  await page.getByRole("tab", { name: "Home" }).click();
  await page.getByRole("textbox", { name: /Find/ }).fill("pocket");
  await page.getByRole("textbox", { name: /Find/ }).press("Enter");
  await expect(page.getByRole("link", { name: "Pocket Golf" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Lantern Keep" })).toHaveCount(0);
});

test("the queue shows server alerts and what the server isn't set up for", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin?tab=queue");
  const alerts = page.getByRole("status").filter({ hasText: "Needs attention" });
  await expect(alerts.getByText("The disk is 91% full")).toBeVisible();
  await expect(alerts.getByText(/Old release files could be cleaned up\./)).toBeVisible();
  // The test server scans nothing, needs no maker signatures and has no GitHub App or token.
  await expect(page.getByText("Scanning is off: files are not scanned.")).toBeVisible();
  await expect(page.getByText("Maker signatures aren't required on this server.")).toBeVisible();
  await expect(page.getByText(/Slop's GitHub App isn't set up/)).toBeVisible();
  await expect(page.getByText(/No GITHUB_TOKEN either/)).toBeVisible();
  // It has a signing key, so approving works.
  await expect(page.getByText(/No signing key is set/)).toHaveCount(0);
});

test("the audit log: search, and older entries a page at a time", async ({ page }) => {
  // Enough entries for more than one page: each save of Featured is one.
  let total = 0;
  for (let before: number | undefined; ; ) {
    const batch = await ok("admin", "GET", `/v1/admin/audit${before ? `?before=${before}` : ""}`);
    total += batch.length;
    if (batch.length < 100) break;
    before = batch[batch.length - 1].id;
  }
  for (let i = total; i < 130; i++) await ok("admin", "PUT", "/v1/admin/featured", { apps: [] });
  await signIn(page, "admin");
  await page.goto("/admin?tab=audit");
  const rows = page.locator("table.audit tbody tr");
  await expect(rows).toHaveCount(100);
  const older = page.getByRole("button", { name: "Older entries" });
  for (let i = 0; i < 20 && (await older.count()) > 0; i++) {
    const shown = await rows.count();
    await older.click();
    // More rows, or (when the last page was exactly full) no button any more.
    await expect(async () => expect((await rows.count()) > shown || (await older.count()) === 0).toBe(true)).toPass();
  }
  await expect(older).toHaveCount(0);
  expect(await rows.count()).toBeGreaterThan(100);

  // Search by what was done.
  await page.getByRole("textbox", { name: /Find/ }).fill("staff_action");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(rows.first()).toContainText("staff_action");
  for (const text of await rows.locator("td:nth-child(3)").allInnerTexts()) expect(text).toContain("staff_action");
  await page.getByRole("textbox", { name: /Find/ }).fill("nothing-has-this-name");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByText("Nothing recorded that matches.")).toBeVisible();
});

test.fixme("the audit log is for the security role (as the Staff tab says), not every staff member", async ({ page }) => {
  // The Staff tab describes the security role as "tokens, staff roles and the audit log", but
  // GET /v1/admin/audit only asks for a staff member (admin_web.rs audit_log). Whether a
  // reviewer may read it is a product decision.
  await signIn(page, "reviewer");
  await page.goto("/admin?tab=audit");
  await expect(page.getByRole("alert")).toContainText("you can't do this");
});
