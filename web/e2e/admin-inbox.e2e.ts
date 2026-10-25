// SPDX-License-Identifier: AGPL-3.0-or-later
// Admin → Review queue as an inbox (web/src/pages/admin/Inbox.tsx): the categories and their
// counts (and the tab's total), one line per item, the order, quick Approve and Deny ("Are you
// sure?" first), opening an item in full, the address and Back, People pages (Ivy's first
// portfolio is approved for real: the seed made it for this test), the phone layout and axe.
// admin-queue.e2e.ts and the others decide each kind of item from its full card.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { call, closePages, inbox, inboxRow, newApp, newPage, newRelease, ok, tag } from "./admin-helpers";
import { expect, ids, signIn, test, type Who } from "./fixtures";

test.afterEach(closePages);

const MAKER: Who = "security";

/** The inbox's categories as the API counts them (pages/admin/queue.ts). */
async function counts(): Promise<Record<string, number>> {
  const q = await ok("admin", "GET", "/v1/admin/queue");
  const actions = await ok("admin", "GET", "/v1/admin/actions");
  const ai = await ok("admin", "GET", "/v1/admin/ai-messages?status=open");
  const legal = await ok("admin", "GET", "/v1/admin/legal-requests");
  const holds = await ok("admin", "GET", "/v1/admin/donation-holds?show=open");
  return {
    "New releases": q.releases.length,
    "Release holds & moves": q.changes.length,
    "Store pictures": q.media.length,
    "Apps to list": q.ready_to_list.length,
    "Re-scan flags": q.rescans.length,
    "People pages": q.portfolios.length,
    "People problems": q.problems.length,
    "Review disputes": q.review_disputes.length,
    Reports: q.reports.length,
    Appeals: q.appeals.length,
    "Abuse alerts": q.abuse.length,
    "Legal requests": legal.filter((r: { status: string }) => r.status === "open").length,
    "Second approvals": actions.actions.filter((a: { status: string }) => a.status === "pending").length,
    "AI messages": ai.length,
    "Donation pause requests": holds.filter(
      (h: { lifted_at: string | null; requests: Array<{ outcome: string | null }> }) =>
        !h.lifted_at && h.requests.some((r) => r.outcome === null),
    ).length,
  };
}

const categories = (page: Page) => page.getByRole("navigation", { name: "Review categories" });

async function axe(page: Page) {
  await page.waitForLoadState("networkidle");
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

test("categories with their counts, the tab's total, one line per item, and the order", async ({ page }) => {
  await signIn(page, "admin");
  const want = await counts();
  await page.goto("/admin?tab=queue");
  // Every category the seed filled, and the empty ones, each with its count.
  for (const [label, n] of Object.entries(want)) {
    await expect(categories(page).getByRole("link", { name: `${label} ${n} waiting`, exact: true })).toBeVisible();
  }
  for (const label of ["New releases", "People pages", "Reports", "Appeals", "Second approvals", "Re-scan flags", "Abuse alerts"]) {
    expect(want[label], label).toBeGreaterThan(0);
  }
  const total = Object.values(want).reduce((a, b) => a + b, 0);
  await expect(page.getByRole("tab", { name: `Review queue ${total} waiting`, exact: true })).toHaveAttribute("aria-selected", "true");
  // With no category in the address, the first with something waiting.
  await expect(categories(page).getByRole("link", { name: /^New releases/ })).toHaveAttribute("aria-current", "page");

  // One line each: what it is, a blurb, its status and how long it has waited.
  const list = page.getByRole("list", { name: "New releases" });
  await expect(list.getByRole("listitem")).toHaveCount(want["New releases"]);
  const golf = list.getByRole("listitem").filter({ hasText: "Pocket Golf 1.1.0" });
  await expect(golf.getByRole("link", { name: "Pocket Golf 1.1.0 by Bo Builder" })).toBeVisible();
  await expect(golf.getByText(/^1 file · /)).toBeVisible();
  await expect(golf.getByText("AWAITING REVIEW")).toBeVisible();
  await expect(golf.locator("time")).toHaveText(/^(just now|\d+ (min|h|days?)) waiting$/);
  await expect(golf.getByRole("button", { name: "Approve Pocket Golf 1.1.0" })).toBeVisible();
  await expect(golf.getByRole("button", { name: "Deny Pocket Golf 1.1.0" })).toBeVisible();
  for (const line of await page.locator(".inbox-line").all()) {
    expect((await line.boundingBox())!.height).toBeLessThan(60);
  }

  // Newest first, in the address.
  const q = await ok("admin", "GET", "/v1/admin/queue");
  const newest = [...q.releases].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))[0];
  await page.getByRole("combobox", { name: "Order" }).selectOption("newest");
  await expect(page).toHaveURL(/[?&]sort=newest/);
  await expect(list.getByRole("listitem").first()).toContainText(`${newest.app.title} ${newest.version}`);

  // Empty categories can be hidden (remembered), and shown again.
  const empty = Object.entries(want).find(([, n]) => n === 0)?.[0];
  expect(empty).toBeTruthy();
  await page.getByRole("button", { name: "Hide empty categories" }).click();
  await expect(categories(page).getByRole("link", { name: new RegExp(`^${empty} `) })).toHaveCount(0);
  await page.reload();
  await expect(categories(page).getByRole("link", { name: new RegExp(`^${empty} `) })).toHaveCount(0);
  await page.getByRole("button", { name: "Show empty categories" }).click();
  await expect(categories(page).getByRole("link", { name: new RegExp(`^${empty} `) })).toBeVisible();

  // Categories are links: the address changes, and Back goes back.
  await categories(page).getByRole("link", { name: /^Appeals/ }).click();
  await expect(page).toHaveURL(/[?&]box=appeals/);
  await expect(page.getByRole("heading", { name: `Appeals (${want.Appeals})` })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole("heading", { name: /^New releases \(/ })).toBeVisible();
});

test("Approve from the list: signed, the line goes, focus moves on, and it's said", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-ib1-${t}`, title: `Beacon ${t}` };
  await newApp(app, MAKER);
  const id = await newRelease(app.slug, "1.0.0", MAKER);
  await signIn(page, "admin");
  await page.goto(inbox("releases"));
  const row = inboxRow(page, `${app.title} 1.0.0`);
  await expect(row.getByText("AWAITING REVIEW")).toBeVisible();
  await row.getByRole("button", { name: `Approve ${app.title} 1.0.0` }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: `Approved and signed “${app.title} 1.0.0”.` })).toBeVisible();
  // Focus is on the line now in its place (or the heading, when none is left).
  await expect(page.locator(".inbox-open:focus, h2:focus")).toHaveCount(1);
  const releases = await ok(MAKER, "GET", `/v1/apps/${app.slug}/releases`);
  expect(releases.find((r: { id: string }) => r.id === id).status).toBe("approved");
});

test("Deny asks “Are you sure?” first: Cancel keeps it; confirming rejects it with the reason", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-ib2-${t}`, title: `Lagoon ${t}` };
  await newApp(app, MAKER);
  const id = await newRelease(app.slug, "0.2.0", MAKER);
  await signIn(page, "admin");
  await page.goto(inbox("releases"));
  const row = inboxRow(page, `${app.title} 0.2.0`);
  const deny = row.getByRole("button", { name: `Deny ${app.title} 0.2.0` });

  await deny.click();
  await expect(deny).toHaveAttribute("aria-expanded", "true");
  const ask = row.getByRole("form", { name: `Are you sure? Deny “${app.title} 0.2.0”.` });
  await expect(ask).toBeVisible();
  const reason = ask.getByRole("textbox", { name: "Why (the maker reads this)" });
  await expect(reason).toBeFocused();
  await expect(ask.getByRole("button", { name: "Yes, deny" })).toBeDisabled();
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect(ask).toHaveCount(0);
  await expect(deny).toBeFocused();
  let status = (await ok(MAKER, "GET", `/v1/apps/${app.slug}/releases`)).find((r: { id: string }) => r.id === id).status;
  expect(status).toBe("awaiting_review");
  await expect(row).toHaveCount(1);

  await deny.click();
  await reason.fill("The zip has no program in it.");
  await ask.getByRole("button", { name: "Yes, deny" }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: `Rejected “${app.title} 0.2.0”.` })).toBeVisible();
  const rejected = (await ok(MAKER, "GET", `/v1/apps/${app.slug}/releases`)).find((r: { id: string }) => r.id === id);
  status = rejected.status;
  expect(status).toBe("rejected");
  expect(rejected.status_reason).toContain("The zip has no program in it.");
});

test("a line opens the item in full; the address says which, and Back goes back", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto(inbox("releases"));
  await categories(page).getByRole("link", { name: /^People pages/ }).click();
  await expect(page).toHaveURL(/[?&]box=people$/);
  const remy = inboxRow(page, "Remy Pending");
  await expect(remy.getByText(`@${ids.pendingCreativeHandle}`)).toBeVisible();
  await expect(remy.getByText("FIRST PORTFOLIO")).toBeVisible();
  await remy.getByRole("link", { name: /^Remy Pending/ }).click();
  await expect(page).toHaveURL(/[?&]box=people&item=[0-9a-f-]{36}/);
  // The full card: every work, notes, the decisions.
  const heading = page.getByRole("heading", { name: "Remy Pending", level: 2 });
  await expect(heading).toBeFocused();
  const card = page.locator(".inbox-detail li.panel");
  await expect(card.getByRole("img")).toHaveCount(5);
  await expect(card.getByRole("button", { name: "Approve portfolio" })).toBeVisible();
  await expect(page.getByRole("link", { name: "← People pages" })).toBeVisible();
  const address = page.url();

  await page.goBack();
  await expect(page).toHaveURL(/[?&]box=people$/);
  await expect(page.getByRole("list", { name: "People pages" })).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/[?&]box=releases$/);

  // The address opens it directly; one that's been decided says so.
  await page.goto(address);
  await expect(page.getByRole("heading", { name: "Remy Pending", level: 2 })).toBeVisible();
  await page.goto(`${inbox("people")}&item=00000000-0000-7000-8000-000000000000`);
  await expect(page.getByText(/This isn't waiting any more/)).toBeVisible();
  await page.getByRole("link", { name: "← People pages" }).click();
  await expect(page.getByRole("list", { name: "People pages" })).toBeVisible();
});

test("People pages: Deny sends the creative's reason; Approve puts Ivy's page live, and she's told", async ({ browser }) => {
  const visitor = await newPage(browser);
  expect((await call(null, "GET", `/v1/people/${ids.inboxCreativeHandle}`)).status).toBe(404);
  const page = await newPage(browser);
  await signIn(page, "admin");
  await page.goto("/admin");
  await page.getByRole("link", { name: /People pages$/ }).click();
  await expect(page).toHaveURL(/[?&]box=people$/);
  const list = page.getByRole("list", { name: "People pages" });

  // Denying Remy's (answered here: his stays waiting for the other tests).
  const sent: Array<{ url: string; body: unknown }> = [];
  await page.route("**/v1/admin/portfolio/*/reject", async (route) => {
    sent.push({ url: route.request().url(), body: route.request().postDataJSON() });
    await route.fulfill({ status: 204 });
  });
  const remy = list.getByRole("listitem").filter({ hasText: "Remy Pending" });
  await remy.getByRole("button", { name: "Deny Remy Pending" }).click();
  await expect(remy.getByText("Are you sure?")).toBeVisible();
  await remy.getByRole("textbox", { name: "What's wrong (the creative reads this)" }).fill("Two tracks are someone else's.");
  await remy.getByRole("button", { name: "Yes, deny" }).click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].body).toEqual({ reason: "Two tracks are someone else's." });

  // Approving Ivy's, for real.
  const ivy = list.getByRole("listitem").filter({ hasText: "Ivy Inbox" });
  await expect(ivy.getByText(`@${ids.inboxCreativeHandle}`)).toBeVisible();
  await expect(ivy.getByText(/^\d+ works? · avatar/)).toBeVisible();
  await expect(ivy.getByText("FIRST PORTFOLIO")).toBeVisible();
  await ivy.getByRole("button", { name: "Approve Ivy Inbox" }).click();
  await expect(ivy).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Approved the People page of “Ivy Inbox”." })).toBeVisible();
  await expect(remy).toHaveCount(1);

  // Her page is live for everyone, and she's told under the bell.
  await visitor.goto(`/people/${ids.inboxCreativeHandle}`);
  await expect(visitor.getByRole("heading", { name: "Ivy Inbox", level: 1 })).toBeVisible();
  const told = await ok("inboxCreative", "GET", "/v1/me/notifications");
  expect(told.items.map((n: { title: string }) => n.title)).toContain("Your People page is live");
});

test("on a phone the categories are a menu and the lines wrap", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signIn(page, "admin");
  await page.goto(inbox("releases"));
  await expect(categories(page)).toBeHidden();
  const menu = page.getByRole("combobox", { name: "Category" });
  await expect(menu).toBeVisible();
  await menu.selectOption("appeals");
  await expect(page).toHaveURL(/[?&]box=appeals/);
  await expect(page.getByRole("heading", { name: /^Appeals \(/ })).toBeVisible();
  // Nothing wider than the screen.
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.goBack();
  // A line's buttons fit on the screen.
  const approve = inboxRow(page, "Pocket Golf 1.1.0").getByRole("button", { name: "Approve Pocket Golf 1.1.0" });
  await approve.scrollIntoViewIfNeeded();
  await expect(approve).toBeInViewport();
  const box = (await approve.boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(390);
  await axe(page);
});

test("the inbox, a list and an item have no serious accessibility problems", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto(inbox("releases"));
  await expect(page.getByRole("list", { name: "New releases" })).toBeVisible();
  await axe(page);
  await inboxRow(page, "Pocket Golf 1.1.0").getByRole("button", { name: "Deny Pocket Golf 1.1.0" }).click();
  await axe(page);
  await page.goto(inbox("people"));
  await inboxRow(page, "Remy Pending").getByRole("link").click();
  await expect(page.locator(".inbox-detail")).toBeVisible();
  await axe(page);
});
