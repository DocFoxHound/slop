// SPDX-License-Identifier: AGPL-3.0-or-later
// The notifications inbox (web/src/pages/Notifications.tsx, crates/slop-api/src/notify.rs): the
// kinds with their unread counts, day groups, Unread only, Mark all as read for one kind, marking
// one read or unread, dismissing, older pages, the kind in the address, phones and accessibility.
// Most tests answer for the server (a small in-page inbox) so the times, kinds and counts are
// exact; the last one does it all against the real server.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { call, tweak } from "./accounts-helpers";
import { expect, ids, signIn, test } from "./fixtures";

// A Wednesday afternoon, in UTC, so the day groups are the same wherever the tests run.
const NOW = new Date("2026-10-21T15:00:00Z");
test.use({ timezoneId: "UTC" });

interface N {
  id: string;
  category: string;
  title: string;
  body: string;
  link: string;
  count: number;
  created_at: string;
  read: boolean;
}

const at = (iso: string) => new Date(iso).toISOString();
const n = (id: number, category: string, title: string, created: string, read = false, extra: Partial<N> = {}): N => ({
  id: `00000000-0000-7000-8000-${String(id).padStart(12, "0")}`,
  category,
  title,
  body: "",
  link: "/library",
  count: 1,
  created_at: at(created),
  read,
  ...extra,
});

function seeded(): N[] {
  return [
    n(7, "chat", "New messages from Bo Builder", "2026-10-21T14:59:00Z", false, { count: 3, link: "/chat" }),
    n(1, "friends", "Sue Support wants to be friends", "2026-10-21T14:50:00Z", false, { body: "Friends can message each other in the SLAPorSLOP app.", link: "/friends" }),
    n(3, "jobs", "Cleo paid for Pixel art", "2026-10-21T13:00:00Z", true),
    n(2, "apps", "Lantern Keep 1.1.0 was approved", "2026-10-21T12:00:00Z", false, { link: "/app/lantern-keep" }),
    n(4, "apps", "Pocket Golf 1.0.0 was received", "2026-10-20T09:00:00Z", true),
    n(5, "moderation", "SLAPorSLOP made a decision about your review", "2026-10-18T09:00:00Z"),
    n(6, "security", "A passkey was added to your account", "2026-10-01T09:00:00Z", true),
  ];
}

/** Answers the inbox's requests from `items` (changed by what the page does), as the server would. */
async function fakeInbox(page: Page, items: N[]) {
  const calls = { list: [] as URLSearchParams[], read: [] as unknown[], unread: [] as unknown[], dismissed: [] as string[] };
  const unread = () => items.filter((i) => !i.read).length;
  await page.route(
    (url) => url.pathname.startsWith("/v1/me/notifications"),
    async (route) => {
      const req = route.request();
      const url = new URL(req.url());
      if (req.method() === "GET" && url.pathname === "/v1/me/notifications") {
        const q = url.searchParams;
        calls.list.push(q);
        let rows = items.filter((i) => (!q.get("category") || i.category === q.get("category")) && (q.get("unread") !== "true" || !i.read));
        const before = q.get("before");
        if (before) rows = rows.slice(rows.findIndex((i) => i.id === before) + 1);
        const limit = Number(q.get("limit") ?? 60);
        const counts: Record<string, number> = {};
        for (const i of items) if (!i.read) counts[i.category] = (counts[i.category] ?? 0) + 1;
        return route.fulfill({ json: { unread: unread(), counts, items: rows.slice(0, limit), more: rows.length > limit } });
      }
      const body = req.postDataJSON() as { ids?: string[]; category?: string } | null;
      if (req.method() === "POST" && url.pathname === "/v1/me/notifications/read") {
        calls.read.push(body);
        for (const i of items) {
          if (body?.ids?.length ? body.ids.includes(i.id) : !body?.category || i.category === body.category) i.read = true;
        }
        return route.fulfill({ status: 204 });
      }
      if (req.method() === "POST" && url.pathname === "/v1/me/notifications/unread") {
        calls.unread.push(body);
        for (const i of items) if (body?.ids?.includes(i.id)) i.read = false;
        return route.fulfill({ status: 204 });
      }
      if (req.method() === "DELETE") {
        const id = url.pathname.split("/").pop()!;
        calls.dismissed.push(id);
        items.splice(
          items.findIndex((i) => i.id === id),
          1,
        );
        return route.fulfill({ status: 204 });
      }
      return route.fallback();
    },
  );
  // The bell counts the same inbox.
  await tweak(page, "/v1/me", (me) => ({ ...me, unread_notifications: unread() }));
  return calls;
}

async function openInbox(page: Page, items: N[], path = "/notifications") {
  await page.clock.install({ time: NOW });
  const calls = await fakeInbox(page, items);
  await signIn(page, "player");
  await page.goto(path);
  await expect(page.getByRole("heading", { name: "Notifications", level: 1 })).toBeVisible();
  return calls;
}

const kinds = (page: Page) => page.getByRole("navigation", { name: "Kinds of notification" });
const row = (page: Page, title: string) => page.locator(".notifications li").filter({ hasText: title });

test("the kinds down the side with what's unread in each, and the notifications by day", async ({ page }) => {
  await openInbox(page, seeded());
  const nav = kinds(page);
  await expect(nav.getByRole("link", { name: "All 4 unread" })).toHaveAttribute("aria-current", "page");
  await expect(nav.getByRole("link", { name: "Your apps 1 unread" })).toHaveAttribute("href", "/notifications/apps");
  await expect(nav.getByRole("link", { name: "Moderation 1 unread" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Security", exact: true })).toBeVisible();
  // Every kind is there, with its friendly name.
  await expect(nav.getByRole("listitem")).toHaveCount(12);
  await expect(page.getByRole("heading", { name: "All notifications", level: 2 })).toBeVisible();

  await expect(page.locator(".nbox-list").getByRole("heading", { level: 3 })).toHaveText(["Today", "Yesterday", "This week", "Earlier"]);
  const today = page.getByRole("region", { name: "Today" });
  await expect(today.getByRole("listitem")).toHaveCount(4);
  await expect(page.getByRole("region", { name: "Earlier" }).getByRole("listitem")).toHaveText(/A passkey was added to your account/);

  // An unread one: bold title, the preview, how long ago (the full date on hover), the dot.
  const sue = row(page, "Sue Support wants to be friends");
  await expect(sue).toHaveClass(/unread/);
  await expect(sue.locator(".n-title")).toHaveCSS("font-weight", "700");
  await expect(sue.locator(".n-body")).toHaveText("Friends can message each other in the SLAPorSLOP app.");
  await expect(sue.locator("time")).toHaveText("10 min ago");
  await expect(sue.locator("time")).toHaveAttribute("title", /Wednesday, October 21, 2026/);
  await expect(sue.locator(".n-dot")).toBeVisible();
  await expect(sue.getByRole("button", { name: /^Friends: Sue Support wants to be friends.*, unread$/ })).toBeVisible();
  const paid = row(page, "Cleo paid for Pixel art");
  await expect(paid).not.toHaveClass(/unread/);
  await expect(paid.locator(".n-title")).toHaveCSS("font-weight", "500");
  await expect(paid.locator(".n-dot")).toHaveCount(0);
  await expect(row(page, "New messages from Bo Builder")).toContainText("(3)");
  await expect(row(page, "Pocket Golf 1.0.0 was received").locator("time")).toHaveText("1 day ago");
  await expect(row(page, "A passkey was added").locator("time")).toHaveText("Oct 1");
  await expect(page.getByRole("link", { name: "your account page" })).toHaveAttribute("href", "/account/notifications");

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
});

test("a kind is in the address and Back returns to it; Unread only; Mark all as read reads only that kind", async ({ page }) => {
  const calls = await openInbox(page, seeded());
  await kinds(page).getByRole("link", { name: /^Your apps/ }).click();
  await expect(page).toHaveURL(/\/notifications\/apps$/);
  const heading = page.getByRole("heading", { name: "Your apps", level: 2 });
  await expect(heading).toBeFocused();
  await expect(kinds(page).getByRole("link", { name: /^Your apps/ })).toHaveAttribute("aria-current", "page");
  await expect(page.locator(".notifications li")).toHaveCount(2);
  expect(calls.list.at(-1)?.get("category")).toBe("apps");

  await page.getByRole("switch", { name: "Unread only" }).check();
  await expect(page).toHaveURL(/\/notifications\/apps\?unread=1$/);
  await expect(page.locator(".notifications li")).toHaveCount(1);
  expect(calls.list.at(-1)?.get("unread")).toBe("true");

  await page.getByRole("button", { name: "Mark all as read" }).click();
  await expect(page.locator(".notifications li.unread")).toHaveCount(0);
  expect(calls.read).toEqual([{ ids: [], category: "apps" }]);
  await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
  await expect(kinds(page).getByRole("link", { name: "Your apps", exact: true })).toBeVisible();
  await expect(kinds(page).getByRole("link", { name: "All 3 unread" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Notifications, 3 unread" })).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "All Your apps notifications marked as read" })).toBeAttached();

  // Unread only stays on from kind to kind.
  await kinds(page).getByRole("link", { name: /^Friends/ }).click();
  await expect(page).toHaveURL(/\/notifications\/friends\?unread=1$/);
  await expect(page.locator(".notifications li")).toHaveText([/Sue Support/]);
  await page.goBack();
  await expect(page).toHaveURL(/\/notifications\/apps\?unread=1$/);
  await expect(heading).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/\/notifications$/);
  await expect(page.getByRole("heading", { name: "All notifications", level: 2 })).toBeVisible();
  // The others are as they were.
  await expect(row(page, "Sue Support")).toHaveClass(/unread/);
  await expect(row(page, "SLAPorSLOP made a decision")).toHaveClass(/unread/);
});

test("each kind says what shows up there when it's empty", async ({ page }) => {
  await openInbox(page, seeded(), "/notifications/teams");
  await expect(page.getByText("Nothing here yet.")).toBeVisible();
  await expect(page.getByText("Team invitations, game nights and verification show up here.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
  await page.goto("/notifications/security?unread=1");
  await expect(page.getByText("Nothing unread in Security.")).toBeVisible();
  await page.getByRole("switch", { name: "Unread only" }).uncheck();
  await expect(row(page, "A passkey was added")).toBeVisible();
  await page.goto("/notifications?unread=1");
  await page.getByRole("button", { name: "Mark all as read" }).click();
  await page.reload();
  await expect(page.getByText("You're all caught up: nothing unread.")).toBeVisible();
  await page.goto("/notifications/marketing");
  await expect(page.getByText("There's no such kind of notification.")).toBeVisible();
});

test("one at a time: marked read, unread again, dismissed; arrows move between them", async ({ page }) => {
  const calls = await openInbox(page, seeded());
  const sue = row(page, "Sue Support wants to be friends");
  await sue.getByRole("button", { name: "Mark as read" }).click();
  await expect(sue).not.toHaveClass(/unread/);
  expect(calls.read).toEqual([{ ids: [seeded()[1].id] }]);
  await expect(kinds(page).getByRole("link", { name: "Friends", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Notifications, 3 unread" })).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Marked as read" })).toBeAttached();

  await sue.getByRole("button", { name: "Mark as unread" }).click();
  await expect(sue).toHaveClass(/unread/);
  expect(calls.unread).toEqual([{ ids: [seeded()[1].id] }]);
  await expect(kinds(page).getByRole("link", { name: "Friends 1 unread" })).toBeVisible();

  // The row's actions say which notification they're for.
  await expect(sue.getByRole("button", { name: "Dismiss" })).toHaveAccessibleDescription(/Sue Support wants to be friends/);
  const paid = row(page, "Cleo paid for Pixel art");
  await paid.getByRole("button", { name: "Dismiss" }).click();
  await expect(paid).toHaveCount(0);
  expect(calls.dismissed).toEqual([seeded()[2].id]);
  // Focus goes on to the next one.
  await expect(row(page, "Lantern Keep 1.1.0").locator("button.notification")).toBeFocused();

  // Up and down, Home and End.
  await page.keyboard.press("ArrowUp");
  await expect(sue.locator("button.notification")).toBeFocused();
  await page.keyboard.press("Home");
  await expect(row(page, "New messages from Bo").locator("button.notification")).toBeFocused();
  await page.keyboard.press("End");
  await expect(row(page, "A passkey was added").locator("button.notification")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(row(page, "A passkey was added").locator("button.notification")).toBeFocused();
  // Enter opens it, and reads it.
  await page.keyboard.press("Home");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/friends$/);
  expect(calls.read.at(-1)).toEqual({ ids: [seeded()[1].id] });
});

test("older ones load 30 at a time, and new ones arrive at the top", async ({ page }) => {
  const items = Array.from({ length: 35 }, (_, i) => n(100 + i, "apps", `Release number ${i}`, new Date(NOW.getTime() - (i + 1) * 3_600_000).toISOString(), i > 2));
  const calls = await openInbox(page, items);
  await expect(page.locator(".notifications li")).toHaveCount(30);
  await page.getByRole("button", { name: "Load older notifications" }).click();
  await expect(page.locator(".notifications li")).toHaveCount(35);
  expect(calls.list.at(-1)?.get("before")).toBe(items[29].id);
  await expect(page.getByRole("button", { name: "Load older notifications" })).toHaveCount(0);

  // Something new while the page is open: the bell notices within a minute, and so does the page.
  items.unshift(n(200, "friends", "Rae Reviewer wants to be friends", NOW.toISOString()));
  await page.clock.fastForward(61_000);
  await expect(page.locator(".notifications li").first()).toContainText("Rae Reviewer wants to be friends");
  await expect(page.locator(".notifications li")).toHaveCount(36);
  await expect(page.getByRole("status").filter({ hasText: "1 new notification" })).toBeAttached();
  await expect(kinds(page).getByRole("link", { name: "Friends 1 unread" })).toBeVisible();
});

test("on a phone the kinds are a row along the top that scrolls sideways", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  await openInbox(page, seeded(), "/notifications/security");
  const nav = kinds(page);
  const all = (await nav.getByRole("link", { name: /^All/ }).boundingBox())!;
  const apps = (await nav.getByRole("link", { name: /^Your apps/ }).boundingBox())!;
  expect(Math.abs(all.y - apps.y)).toBeLessThan(2);
  expect(await nav.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
  // The chosen kind is scrolled into view.
  await expect(nav.getByRole("link", { name: "Security", exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.goto("/notifications");
  await expect(page.locator(".notifications li")).toHaveCount(7);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
});

test("against the real server: read and unread again, dismissed, and Mark all as read for one kind", async ({ page }) => {
  // Mo starts with nothing unread and no friend notifications (other tests' are dismissed), then
  // two people ask to be friends.
  await call("moderator", "POST", "/v1/me/notifications/read", {});
  const old = await call<{ items: Array<{ id: string }> }>("moderator", "GET", "/v1/me/notifications?category=friends&limit=100");
  for (const n of old.items) await call("moderator", "DELETE", `/v1/me/notifications/${n.id}`);
  await call("support", "POST", "/v1/me/friends", { user: ids.moderator });
  await call("security", "POST", "/v1/me/friends", { user: ids.moderator });
  try {
    await signIn(page, "moderator");
    await page.goto("/notifications/friends");
    await expect(kinds(page).getByRole("link", { name: "Friends 2 unread" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Notifications, 2 unread" })).toBeVisible();
    const sue = row(page, "Sue Support wants to be friends");
    await sue.getByRole("button", { name: "Mark as read" }).click();
    await expect(page.getByRole("link", { name: "Notifications, 1 unread" })).toBeVisible();
    await sue.getByRole("button", { name: "Mark as unread" }).click();
    await expect(page.getByRole("link", { name: "Notifications, 2 unread" })).toBeVisible();
    await row(page, "Sec Security wants to be friends").getByRole("button", { name: "Dismiss" }).click();
    await expect(page.getByRole("link", { name: "Notifications, 1 unread" })).toBeVisible();
    await page.reload();
    await expect(row(page, "Sec Security")).toHaveCount(0);
    await expect(sue).toHaveClass(/unread/);
    // Only Friends is read; the All view shows it.
    await page.getByRole("button", { name: "Mark all as read" }).click();
    await expect(page.getByRole("link", { name: "Notifications", exact: true })).toBeVisible();
    await kinds(page).getByRole("link", { name: /^All/ }).click();
    await expect(sue).not.toHaveClass(/unread/);
    // The request itself is still there to answer.
    await sue.getByRole("button", { name: /Sue Support wants to be friends/ }).click();
    await expect(page.getByRole("heading", { name: /Requests \(2\)/ })).toBeVisible();
  } finally {
    await call("support", "DELETE", `/v1/me/friends/${ids.moderator}`);
    await call("security", "DELETE", `/v1/me/friends/${ids.moderator}`);
    await call("moderator", "POST", "/v1/me/notifications/read", {});
  }
});
