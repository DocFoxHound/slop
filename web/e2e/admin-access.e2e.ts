// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel's door and its tabs: who gets in, what each role may see, the Home page's
// counts, alerts and server checks, the audit log, AI reviewer tokens, staff roles and the
// actions waiting for a second approval.
//
// Changing staff and approving actions need a sign-in from the last ten minutes (SECURITY §4.3),
// and the seeded sessions are as old as the test server: these run first. Making an AI reviewer
// token doesn't (decided 2026-10-25).
import { adminToken, call, closePages, inbox, inboxRow, newPage, ok, tag, withToken } from "./admin-helpers";
import type { Locator } from "@playwright/test";

import { expect, ids, server, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("AI reviewer tokens: made with a scope and an expiry, set up without the token in any command, listed, and revoked", async ({
  page,
}) => {
  await signIn(page, "admin");
  // The seeded session is old: making a token doesn't ask to sign in again.
  await page.goto("/admin?tab=ai");
  const name = `Picture checks ${tag()}`;
  await page.getByRole("textbox", { name: "Name" }).fill(name);
  // The same lifetimes as makers' tokens; moderate ones up to 3 months.
  const expires = page.getByRole("combobox", { name: "Expires after" });
  await expect(expires.locator("option")).toHaveText(["1 day", "7 days", "30 days", "3 months", "6 months", "12 months"]);
  await page.getByRole("combobox", { name: "Can" }).selectOption({ label: "Also suspend accounts and withdraw releases (moderate)" });
  await expect(expires.locator("option")).toHaveText(["1 day", "7 days", "30 days", "3 months"]);
  await page.getByRole("combobox", { name: "Can" }).selectOption({ label: "Also approve and reject (review)" });
  await expires.selectOption({ label: "30 days" });
  await page.getByRole("button", { name: "Make token" }).click();

  // The setup makers get: a warning, the token once, then steps that only refer to it.
  const setup = page.getByRole("region", { name: `Set up “${name}” on your computer` });
  await expect(setup.getByText("Never paste this token into a chat with Claude, ChatGPT, Codex, Grok or any AI.")).toBeVisible();
  await expect(setup.getByText(/It's a password for your Slop staff account/)).toBeVisible();
  await expect(setup.getByText(/Copy it now: Slop won't show this token again\./)).toBeVisible();
  const secret = (await setup.locator("pre.copy-row-secret").innerText()).trim();
  expect(secret).toMatch(/^slopadm_/);
  // Only the token itself and the step that saves it have it.
  const withSecret = setup.locator("pre").filter({ hasText: secret });
  await expect(withSecret).toHaveCount(2);
  await expect(withSecret.nth(1)).toHaveText(/SLOP_ADMIN_TOKEN/);
  await expect(page.getByText(/claude mcp add .*slopadm_/)).toHaveCount(0);
  await expect(setup.getByText("Save the token on your computer")).toBeVisible();
  const tools = setup.getByRole("tablist", { name: "Your AI tool" });
  await tools.getByRole("tab", { name: "Claude Code" }).click();
  await expect(
    setup.getByText(
      `claude mcp add --transport http --scope user slop-admin ${server.url}/v1/admin/mcp --header 'Authorization: Bearer \${SLOP_ADMIN_TOKEN}'`,
    ),
  ).toBeVisible();
  await tools.getByRole("tab", { name: "Codex (OpenAI)" }).click();
  await expect(
    setup.getByText(`codex mcp add slop-admin --url ${server.url}/v1/admin/mcp --bearer-token-env-var SLOP_ADMIN_TOKEN`),
  ).toBeVisible();
  for (const tool of ["Grok Build (xAI)", "Other AI tools"]) {
    await tools.getByRole("tab", { name: tool }).click();
    await expect(setup.locator("pre").filter({ hasText: secret })).toHaveCount(2);
  }
  await expect(setup.getByText(/^Read review_guide, then /)).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Name" })).toHaveValue("");
  await setup.getByRole("button", { name: "Done" }).click();
  await expect(setup).toHaveCount(0);

  // It works, it's listed with its scope, whose it is and when it expires, and revoking ends it.
  expect((await withToken(secret, "GET", "/v1/admin/status")).status).toBe(200);
  const row = page.getByRole("listitem").filter({ hasText: name });
  await expect(row.getByText("REVIEW", { exact: true })).toBeVisible();
  await expect(row.getByText(/for Ada Admin · made .* · never used · expires in 30 days/)).toBeVisible();
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

  // An expired token of your own offers a replacement with its settings.
  await old.getByRole("button", { name: "Make a replacement for Old picture checker" }).click();
  await expect(admin.getByText(/Making a replacement for “Old picture checker”, with the same settings/)).toBeVisible();
  await expect(admin.getByRole("textbox", { name: "Name" })).toHaveValue("Old picture checker");
  await expect(admin.getByRole("combobox", { name: "Expires after" })).toHaveValue("7");

  // A refusal shows, and no secret does.
  await admin.route("**/v1/admin/tokens", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 422,
          json: { error: { code: "invalid", message: "name: give it a name of up to 60 characters" } },
        })
      : route.fallback(),
  );
  await admin.getByRole("textbox", { name: "Name" }).fill("Late night checker");
  await admin.getByRole("button", { name: "Make token" }).click();
  await expect(admin.getByRole("alert")).toHaveText("name: give it a name of up to 60 characters");
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
  await expect(page.getByRole("link", { name: /^\d+ Second approvals$/ })).toBeVisible();
  await page.getByRole("link", { name: /Second approvals$/ }).click();
  const waiting = page.getByRole("list", { name: "Second approvals" });
  const support = waiting.getByRole("listitem").filter({ hasText: "Give the support role to Olly Overdue" });
  const finance = waiting.getByRole("listitem").filter({ hasText: "Give the finance role to Olly Overdue" });
  await expect(support.getByText(/Asked by AI reviewer "Staffing agent .*" for Ada Admin/)).toBeVisible();
  await expect(support.getByText("0 OF 1 APPROVALS")).toBeVisible();
  // The seeded request (Sam) stays where it is.
  await expect(waiting.getByRole("listitem").filter({ hasText: "Sam Seed" })).toHaveCount(1);
  // Opened, it says how many approvals a sensitive action needs.
  await support.getByRole("link").click();
  await expect(page.getByText("Sensitive actions need 1 different staff members")).toBeVisible();
  await page.goBack();

  await finance.getByRole("button", { name: /^Deny / }).click();
  await finance.getByRole("button", { name: "Yes, deny" }).click();
  await expect(finance).toHaveCount(0);
  await support.getByRole("button", { name: /^Approve / }).click();
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
    page.getByRole("status").filter({ hasText: "Recorded. It happens once another staff member approves it under Review queue → Second approvals." }),
  ).toBeVisible();

  // Whoever asked can't be the second approval.
  await page.route("**/v1/admin/actions/*/approve", (route) =>
    route.fulfill({ status: 409, json: { error: { code: "conflict", message: "you already approved this: it needs someone else" } } }),
  );
  await page.goto(inbox("approvals"));
  const sam = inboxRow(page, "Sam Seed");
  await sam.getByRole("button", { name: /^Approve / }).click();
  await expect(page.getByRole("alert")).toHaveText("you already approved this: it needs someone else");
  await expect(sam).toHaveCount(1);
  // Focus stays on the button that was refused.
  await expect(sam.getByRole("button", { name: /^Approve / })).toBeFocused();
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
    ["Review queue", page.getByRole("navigation", { name: "Review categories" })],
    ["Apps", page.getByRole("list", { name: "Apps" }).getByRole("button", { name: /^Actions for / }).first()],
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
    // The queue's tab also says how much is waiting.
    const name = tab === "Review queue" ? /^Review queue \d+ waiting$/ : tab;
    await page.getByRole("tab", { name, exact: true }).click();
    await expect(page.getByRole("tab", { name, exact: true })).toHaveAttribute("aria-selected", "true");
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
  const categories = reviewer.getByRole("navigation", { name: "Review categories" });
  await expect(categories.getByRole("link", { name: /^New releases \d+ waiting$/ })).toBeVisible();
  // Legal requests are for staff who take sensitive actions: not a category for them.
  await expect(categories.getByRole("link", { name: /^Legal requests/ })).toHaveCount(0);

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
  // What the API counts, read again on each try: earlier files' background work (scans, alerts)
  // can still add to the queue while this one starts, on a slow machine.
  const waiting = async () => {
    const queue = await ok("admin", "GET", "/v1/admin/queue");
    const actions = await ok("admin", "GET", "/v1/admin/actions");
    const ops = await ok("admin", "GET", "/v1/admin/ops");
    const ai = await ok("admin", "GET", "/v1/admin/ai-messages?status=open");
    const counts: Array<[number, string]> = [
      [ops.length, "Server alerts"],
      [actions.actions.length, "Second approvals"],
      [queue.reports.length, "Reports"],
      [queue.appeals.length, "Appeals"],
      [queue.rescans.length, "Re-scan flags"],
      [queue.releases.length, "New releases"],
      [queue.changes.length, "Release holds & moves"],
      [queue.media.length, "Store pictures"],
      [queue.portfolios.length, "People pages"],
      [queue.problems.length, "People problems"],
      [queue.review_disputes.length, "Review disputes"],
      [queue.abuse.length, "Abuse alerts"],
      [queue.ready_to_list.length, "Apps to list"],
      [ai.length, "AI messages"],
    ];
    return { queue, counts };
  };
  let queue = (await waiting()).queue;
  await page.goto("/admin");
  let first = true;
  await expect(async () => {
    if (!first) await page.reload();
    first = false;
    const now = await waiting();
    queue = now.queue;
    for (const [n, label] of now.counts) {
      expect(n, label).toBeGreaterThan(0);
      await expect(page.getByRole("link", { name: `${n} ${label}`, exact: true })).toBeVisible({ timeout: 2_000 });
    }
  }).toPass({ timeout: 20_000 });
  // Nothing paused, so nothing about pauses.
  await expect(page.getByRole("link", { name: /Donation pause requests/ })).toHaveCount(0);

  // Each goes to its category in the queue.
  await page.getByRole("link", { name: /^\d+ Appeals$/ }).click();
  await expect(page).toHaveURL(/tab=queue&box=appeals$/);
  await expect(page.getByRole("heading", { name: `Appeals (${queue.appeals.length})` })).toBeVisible();
  await page.goBack();
  await page.getByRole("link", { name: /^\d+ Reports$/ }).click();
  await expect(page.getByRole("heading", { name: /^Reports \(/ })).toBeVisible();

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
