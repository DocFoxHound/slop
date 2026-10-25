// SPDX-License-Identifier: AGPL-3.0-or-later
// What's on every page (web/src/components/Layout.tsx, main.tsx): the tabs, sign-in and the bell,
// notices about the account, the store's search and sidebar, and the notifications page.
import AxeBuilder from "@axe-core/playwright";

import { apiError, call, fake, tweak } from "./accounts-helpers";
import { expect, ids, server, signIn, test } from "./fixtures";

test.describe("the app shell", () => {
  test("an unknown address says the page wasn't found", async ({ page }) => {
    await page.goto("/nope/not-here");
    await expect(page.getByText("Page not found.")).toBeVisible();
    await page.getByRole("link", { name: "Back to the store" }).click();
    await expect(page).toHaveURL(`${server.url}/`);
    await expect(page).toHaveTitle(/Slop/);
  });

  test("the top tabs mark the section you're in", async ({ page }) => {
    const top = page.getByRole("navigation", { name: "Main" }).first();
    for (const [path, tab] of [
      ["/", "STORE"],
      ["/app/castle-rock", "STORE"],
      ["/search?q=castle", "STORE"],
      ["/people", "PEOPLE"],
      [`/jobs/${ids.requestedJob}`, "PEOPLE"],
      ["/library", "LIBRARY"],
      ["/publish", "PUBLISH"],
    ]) {
      await page.goto(path);
      await expect(top.locator(".tabs a.on")).toHaveText(tab);
    }
    // Chat isn't a top tab: it's the Friends & Chat bar (and a bottom tab on phones).
    await expect(top.getByRole("link", { name: "CHAT" })).toHaveCount(0);
    for (const path of ["/account", "/chat", "/friends"]) {
      await page.goto(path);
      await expect(top.locator(".tabs a.on")).toHaveCount(0);
    }
  });

  test("the top bar has no search: the store and People have their own", async ({ page }) => {
    await page.goto("/");
    const top = page.getByRole("navigation", { name: "Main" }).first();
    await expect(page.getByRole("navigation", { name: "Store" }).getByRole("searchbox", { name: "Search the store" })).toBeVisible();
    await expect(top.getByRole("searchbox")).toHaveCount(0);
    await page.goto("/library");
    await expect(page.getByRole("searchbox")).toHaveCount(0);
    await page.goto("/people");
    await expect(page.getByRole("searchbox", { name: "Search people" })).toBeVisible();
    await expect(page.getByRole("searchbox", { name: "Search the store" })).toHaveCount(0);
  });

  test("the avatar keeps its size however narrow the window", async ({ page }) => {
    await signIn(page, "player");
    for (const width of [1280, 900, 700, 620]) {
      await page.setViewportSize({ width, height: 700 });
      await page.goto("/");
      const box = await page.locator(".me .avatar").boundingBox();
      expect(box?.width, `at ${width}px`).toBe(32);
      expect(box?.height, `at ${width}px`).toBe(32);
    }
  });

  test("on a phone, the tabs are at the bottom, with Account", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, "player");
    await page.goto("/");
    const bottom = page.getByRole("navigation", { name: "Main" }).last();
    await expect(bottom).toBeVisible();
    await bottom.getByRole("link", { name: "Library" }).click();
    await expect(page).toHaveURL(/\/library$/);
    await expect(bottom.getByRole("link", { name: "Library" })).toHaveClass(/on/);
    // Chat is a bottom tab on a phone, instead of the Friends & Chat bar.
    await expect(page.getByRole("button", { name: /^Friends & Chat/ })).toBeHidden();
    await bottom.getByRole("link", { name: "Chat" }).click();
    await expect(page).toHaveURL(/\/chat$/);
    await expect(page.getByText("Chat is in the Slop app.")).toBeVisible();
    await expect(bottom.getByRole("link", { name: "Chat" })).toHaveClass(/on/);
    await bottom.getByRole("link", { name: "Account" }).click();
    await expect(page.getByRole("heading", { name: "Your account" })).toBeVisible();
    await expect(bottom.getByRole("link", { name: "Account" })).toHaveClass(/on/);
    // No sideways scrolling on a phone.
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });

  test("a link to part of a page goes there once it has loaded, even if parts above load later", async ({ page }) => {
    // The decisions arrive late and push everything below them down.
    await page.route("**/v1/me/decisions", async (route) => {
      await new Promise((r) => setTimeout(r, 1500));
      await route.fulfill({
        json: [1, 2, 3].map((n) => ({
          id: `01a00000-0000-7000-8000-00000000000${n}`,
          reference: `D-TEST-000${n}`,
          subject_kind: "account",
          subject_label: "Sue Support",
          action: "warn",
          action_label: "Warning",
          rule: "spam",
          rule_title: "Spam",
          law: null,
          facts: "Made up for a test, to make the page longer. ".repeat(12),
          automated: false,
          created_at: "2026-10-20T10:00:00Z",
          can_appeal: false,
          appeal_until: "2027-04-20T10:00:00Z",
          appeal_status: null,
          appealed_at: null,
          appeal_reason: null,
          reversed_at: null,
          new: false,
        })),
      });
    });
    // Sue's reports, below them.
    await page.route("**/v1/me/reports", (route) =>
      route.fulfill({
        json: [1, 2].map((n) => ({
          reference: `R-TEST-000${n}`,
          subject_kind: "app",
          subject_label: `Made-up app ${n}`,
          reason: "spam",
          reason_label: "Spam",
          status: "open",
          outcome: null,
          created_at: "2026-10-20T10:00:00Z",
          decided_at: null,
        })),
      }),
    );
    await signIn(page, "support");
    // An old link to the reports: the Moderation section, at the reports.
    await page.goto("/account#reports");
    await expect(page).toHaveURL(/\/account\/moderation#reports$/);
    await expect(page.getByRole("heading", { name: "Decisions about your content or account" })).toBeAttached();
    await expect(page.getByRole("heading", { name: "Your reports" })).toBeInViewport();
  });

  test("a new page starts at the top", async ({ page }) => {
    await page.goto("/publish/guide");
    await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
    await page.locator("main").evaluate((m) => m.scrollTo({ top: 3000 }));
    expect(await page.locator("main").evaluate((m) => m.scrollTop)).toBeGreaterThan(0);
    await page.getByRole("link", { name: "Slop home" }).click();
    await expect(page).toHaveURL(`${server.url}/`);
    await expect.poll(() => page.locator("main").evaluate((m) => m.scrollTop)).toBe(0);
  });

  test("the website offers the desktop app", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: "⬇ Download" }).click();
    await expect(page).toHaveURL(/\/download$/);
  });

  test("ADMIN shows only for staff", async ({ page }) => {
    const admin = page.getByRole("link", { name: "ADMIN" });
    await page.goto("/");
    await expect(page.getByRole("link", { name: "STORE" })).toBeVisible();
    await expect(admin).toHaveCount(0);
    await signIn(page, "player");
    await page.reload();
    await expect(page.getByRole("link", { name: "Pat Player" })).toBeVisible();
    await expect(admin).toHaveCount(0);
    // Staff with any one role see it.
    await signIn(page, "reviewer");
    await page.reload();
    await expect(admin).toBeVisible();
    await admin.click();
    await expect(page.getByRole("heading", { name: "Admin" })).toBeVisible();
    await expect(admin).toHaveClass(/on/);
  });

  test("a player who types the admin address gets nothing", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/admin");
    await expect(page.getByText("Page not found.")).toBeVisible();
  });
});

test.describe("search", () => {
  test("Enter searches for what was typed, trimmed and encoded", async ({ page }) => {
    await page.goto("/browse");
    const box = page.getByRole("searchbox", { name: "Search the store" });
    await expect(box).toHaveAttribute("maxlength", "100");
    await box.fill("  castle & rock  ");
    await box.press("Enter");
    await expect(page).toHaveURL(`${server.url}/search?q=castle%20%26%20rock`);
    await expect(box).toHaveValue("castle & rock");
  });

  test("on the search page the box shows the search, and leaves it elsewhere", async ({ page }) => {
    await page.goto("/search?q=neon");
    const box = page.getByRole("searchbox", { name: "Search the store" });
    await expect(box).toHaveValue("neon");
    await page.goto("/upcoming");
    await expect(box).toHaveValue("");
  });

  test("People has its own search, by name, handle or headline", async ({ page }) => {
    await page.goto("/people");
    const box = page.getByRole("searchbox", { name: "Search people" });
    await box.fill("vera");
    await box.press("Enter");
    await expect(page).toHaveURL(/\/people\?q=vera$/);
    await expect(page.getByRole("link", { name: /Vera Okafor/ })).toBeVisible();
    await box.fill("nobody-is-called-this");
    await page.getByRole("search").getByRole("button", { name: "Search" }).click();
    await expect(page.getByText("Nobody matches “nobody-is-called-this” with those filters.")).toBeVisible();
    // Clearing the box shows everyone again.
    await box.fill("");
    await expect(page).toHaveURL(/\/people$/);
    await expect(page.getByRole("link", { name: /Vera Okafor/ })).toBeVisible();
  });
});

test.describe("signing in from any page", () => {
  test("Sign in comes back to the page you were on", async ({ page }) => {
    await page.goto("/search?q=castle");
    const signInLink = page.getByRole("link", { name: "Sign in" });
    await expect(signInLink).toHaveAttribute("href", "/signin?return_to=%2Fsearch%3Fq%3Dcastle");
    await signInLink.click();
    await expect(page.getByRole("link", { name: "Continue with GitHub" })).toHaveAttribute(
      "href",
      "/v1/auth/github/start?return_to=%2Fsearch%3Fq%3Dcastle",
    );
  });

  test("signed in, your name links to your account", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/");
    await expect(page.getByRole("link", { name: "Sign in" })).toHaveCount(0);
    await page.getByRole("link", { name: "Pat Player" }).click();
    await expect(page.getByRole("heading", { name: "Your account" })).toBeVisible();
  });
});

test.describe("the bell", () => {
  test("signed out there's no bell; signed in it counts what's unread", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("link", { name: "STORE" })).toBeVisible();
    await expect(page.getByRole("link", { name: /^Notifications/ })).toHaveCount(0);
    await signIn(page, "maker2");
    const unread = (await call("maker2", "GET", "/v1/me")).unread_notifications as number;
    await page.reload();
    const bell = page.getByRole("link", { name: `Notifications, ${unread} unread` });
    await expect(bell).toBeVisible();
    await expect(bell.locator(".bell-count")).toHaveText(String(unread));
    // Its count was white on light red: too faint to read.
    const results = await new AxeBuilder({ page }).include(".bell").withRules(["color-contrast"]).analyze();
    expect(results.violations.map((v) => v.id)).toEqual([]);
    await bell.click();
    await expect(page).toHaveURL(/\/notifications$/);
    await expect(bell).toHaveClass(/on/);
  });

  test("the count catches up every minute", async ({ page }) => {
    await page.clock.install();
    await signIn(page, "reviewer");
    await page.goto("/bounties");
    const before = (await call("reviewer", "GET", "/v1/me")).unread_notifications as number;
    const label = (n: number) => (n > 0 ? `Notifications, ${n} unread` : "Notifications");
    await expect(page.getByRole("link", { name: label(before), exact: true })).toBeVisible();
    await call("security", "POST", "/v1/me/friends", { user: ids.reviewer });
    try {
      await page.clock.fastForward(61_000);
      await expect(page.getByRole("link", { name: label(before + 1), exact: true })).toBeVisible();
    } finally {
      await call("security", "DELETE", `/v1/me/friends/${ids.reviewer}`);
      // Rae had nothing unread before: leave it that way.
      if (before === 0) await call("reviewer", "POST", "/v1/me/notifications/read", {});
    }
  });

  test("more than 99 shows as 99+", async ({ page }) => {
    await tweak(page, "/v1/me", (me) => ({ ...me, unread_notifications: 150 }));
    await signIn(page, "player");
    await page.goto("/");
    const bell = page.getByRole("link", { name: "Notifications, 150 unread" });
    await expect(bell.locator(".bell-count")).toHaveText("99+");
  });
});

test.describe("notices about your account", () => {
  test("a suspended account is told, on every page, with the way to appeal", async ({ page }) => {
    await signIn(page, "suspended");
    await page.goto("/bounties");
    const notice = page.locator(".account-notices");
    await expect(notice).toContainText("Your account is suspended. You can read Slop, download your data and appeal.");
    await expect(notice.getByRole("link", { name: "See why, and appeal" })).toHaveAttribute("href", "/account/moderation#decisions");
    // The link stands out by more than its colour.
    await expect(notice.getByRole("link", { name: "See why, and appeal" })).toHaveCSS("text-decoration-line", "underline");
  });

  test("new decisions are counted until seen", async ({ page }) => {
    await tweak(page, "/v1/me", (me) => ({ ...me, new_decisions: 2 }));
    await signIn(page, "player");
    await page.goto("/");
    await expect(page.locator(".account-notices")).toContainText("Slop made 2 decisions about your content.");
    await expect(page.getByRole("link", { name: "See what, why, and how to appeal" })).toHaveAttribute("href", "/account/moderation#decisions");
  });

  test("changed agreements: upcoming ones can be dismissed for good, ones to accept now can't", async ({ page }) => {
    await fake(page, "GET", "/v1/me/legal", {
      status: 200,
      body: [
        { kind: "privacy", title: "Privacy Policy", version: "2026-11-15", summary: "Names the new backup provider.", effective_at: "2026-11-15T00:00:00Z", in_effect: false, first: false, action: "read" },
        { kind: "terms", title: "Terms of Service", version: "2026-10-01", summary: "Clearer refunds.", effective_at: "2026-10-01T00:00:00Z", in_effect: true, first: false, action: "accept" },
        { kind: "content", title: "Content Policy", version: "2026-10-10", summary: "", effective_at: "2026-10-10T00:00:00Z", in_effect: true, first: true, action: "read" },
      ],
    });
    await signIn(page, "player");
    await page.goto("/");
    const notices = page.locator(".account-notices");
    const privacy = notices.locator(".account-notice").filter({ hasText: "Privacy Policy" });
    await expect(privacy).toContainText("The Privacy Policy changes on Nov 15, 2026. Names the new backup provider.");
    await expect(privacy.getByRole("link", { name: "Read it" })).toHaveAttribute("href", "/legal/privacy?version=2026-11-15");
    const terms = notices.locator(".account-notice").filter({ hasText: "Terms of Service" });
    await expect(terms).toContainText("The Terms of Service changed on Oct 1, 2026.");
    await expect(terms.getByRole("link", { name: "Review and accept" })).toHaveAttribute("href", "/legal/terms");
    await expect(terms.getByRole("button", { name: "Dismiss" })).toHaveCount(0);
    await expect(notices.locator(".account-notice").filter({ hasText: "Slop has published its Content Policy." })).toBeVisible();

    await privacy.getByRole("button", { name: "Dismiss" }).click();
    await expect(privacy).toHaveCount(0);
    await page.reload();
    await expect(terms).toBeVisible();
    await expect(privacy).toHaveCount(0);
  });

  test("dismissing works for the page even where the browser keeps nothing", async ({ page }) => {
    await page.addInitScript(() => {
      Storage.prototype.getItem = () => {
        throw new Error("storage is off");
      };
      Storage.prototype.setItem = () => {
        throw new Error("storage is off");
      };
    });
    await fake(page, "GET", "/v1/me/legal", {
      status: 200,
      body: [{ kind: "privacy", title: "Privacy Policy", version: "2026-11-15", summary: "Names the new backup provider.", effective_at: "2026-11-15T00:00:00Z", in_effect: false, first: false, action: "read" }],
    });
    await signIn(page, "player");
    await page.goto("/");
    const privacy = page.locator(".account-notice").filter({ hasText: "Privacy Policy" });
    await privacy.getByRole("button", { name: "Dismiss" }).click();
    await expect(privacy).toHaveCount(0);
    await page.reload();
    await expect(privacy).toBeVisible();
  });

  test("with nothing to say, there are no notices", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/");
    await expect(page.getByRole("link", { name: "Pat Player" })).toBeVisible();
    await expect(page.locator(".account-notices")).toHaveCount(0);
  });
});

test.describe("the simulation sandbox", () => {
  test("is announced everywhere, with a way to view as a simulated user", async ({ page }) => {
    await fake(page, "GET", "/v1/server", { status: 200, body: { scanning: false, maker_signatures_required: false, simulation: true } });
    await page.goto("/bounties");
    const banner = page.getByRole("note").filter({ hasText: "Simulation sandbox." });
    await expect(banner).toContainText("Every app, player and vote here is made up");
    await expect(banner.getByRole("link", { name: "View as a simulated user" })).toHaveAttribute("href", "/sim");
  });

  test("a real server has no banner", async ({ page }) => {
    await page.goto("/bounties");
    await expect(page.getByRole("link", { name: "STORE" })).toBeVisible();
    await expect(page.getByText("Simulation sandbox.")).toHaveCount(0);
  });
});

test.describe("the store sidebar", () => {
  test("isn't on the front page, which is all apps", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("navigation", { name: "Store" })).toBeVisible();
    await expect(page.getByRole("complementary", { name: "Browse" })).toHaveCount(0);
  });

  test("lists the categories on list pages, and marks the one you're in", async ({ page }) => {
    const categories = await call<Array<{ slug: string; name: string }>>(null, "GET", "/v1/categories");
    await page.goto("/browse");
    const sidebar = page.getByRole("complementary", { name: "Browse" });
    for (const c of categories) {
      await expect(sidebar.getByRole("link", { name: c.name, exact: true })).toHaveAttribute("href", `/browse/${c.slug}`);
    }
    await sidebar.getByRole("link", { name: categories[0].name, exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/browse/${categories[0].slug}$`));
    await expect(page.getByRole("complementary", { name: "Browse" }).getByRole("link", { name: categories[0].name, exact: true })).toHaveClass(/on/);
    await expect(page.getByRole("complementary", { name: "Browse" }).getByRole("link", { name: categories[1].name, exact: true })).not.toHaveClass(/on/);
  });
});

test.describe("the notifications page", () => {
  test("signed out, it asks you to sign in", async ({ page }) => {
    await page.goto("/notifications");
    await expect(page.getByRole("link", { name: "Sign in", exact: true }).last()).toHaveAttribute("href", "/signin?return_to=/notifications");
  });

  test("empty, it says what will show up", async ({ page }) => {
    await fake(page, "GET", "/v1/me/notifications", { status: 200, body: { unread: 0, items: [] } });
    await signIn(page, "moderator");
    await page.goto("/notifications");
    await expect(page.getByText("Nothing yet. Job steps, apps you're Interested in and decisions about your content show up here.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
    await expect(page.getByRole("link", { name: "your account page" })).toHaveAttribute("href", "/account/notifications");
  });

  test("opening one reads it and goes to it; Mark all as read reads the rest", async ({ page }) => {
    // Mo starts with nothing unread and no friend notifications (other tests leave him some:
    // notifications.e2e.ts asks him to be friends too, money-bounties.e2e.ts has him open a
    // bounty), then two people ask Mo to be friends: two notifications.
    await call("moderator", "POST", "/v1/me/notifications/read", {});
    const old = await call<{ items: Array<{ id: string }> }>("moderator", "GET", "/v1/me/notifications?category=friends&limit=100");
    for (const n of old.items) await call("moderator", "DELETE", `/v1/me/notifications/${n.id}`);
    await call("support", "POST", "/v1/me/friends", { user: ids.moderator });
    await call("security", "POST", "/v1/me/friends", { user: ids.moderator });
    try {
      await signIn(page, "moderator");
      await page.goto("/notifications");
      await expect(page.getByRole("link", { name: "Notifications, 2 unread" })).toBeVisible();
      const sue = page.getByRole("listitem").filter({ hasText: "Sue Support wants to be friends" });
      await expect(sue).toHaveClass(/unread/);
      await expect(sue).toContainText("Friends can message each other in the Slop app.");
      await sue.getByRole("button", { name: /Sue Support wants to be friends/ }).click();
      await expect(page).toHaveURL(new RegExp(`/friends#${ids.support}$`));
      await expect(page.getByRole("heading", { name: /Requests \(2\)/ })).toBeVisible();
      await expect(page.getByRole("link", { name: "Notifications, 1 unread" })).toBeVisible();

      await page.goto("/notifications");
      await expect(page.getByRole("listitem").filter({ hasText: "Sue Support" })).not.toHaveClass(/unread/);
      await page.getByRole("button", { name: "Mark all as read" }).click();
      await expect(page.getByRole("link", { name: "Notifications", exact: true })).toBeVisible();
      await expect(page.getByRole("listitem").filter({ hasText: "Sec Security" })).not.toHaveClass(/unread/);
      await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
    } finally {
      await call("support", "DELETE", `/v1/me/friends/${ids.moderator}`);
      await call("security", "DELETE", `/v1/me/friends/${ids.moderator}`);
      await call("moderator", "POST", "/v1/me/notifications/read", {});
    }
  });

  test("Mark all as read that fails says so", async ({ page }) => {
    await fake(page, "POST", "/v1/me/notifications/read", { status: 503, body: apiError("unavailable", "Slop is busy: try again in a minute") });
    await signIn(page, "player");
    await page.goto("/notifications");
    await page.getByRole("button", { name: "Mark all as read" }).click();
    await expect(page.getByRole("alert")).toHaveText("Slop is busy: try again in a minute");
    await expect(page.getByRole("button", { name: "Mark all as read" })).toBeVisible();
  });
});
