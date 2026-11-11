// SPDX-License-Identifier: AGPL-3.0-or-later
// Maker news (web/src/components/News.tsx, crates/slop-api/src/news.rs): Bo writes, edits and
// removes posts in Manage's News panel; Pat, who marked the app Interested, is told under the bell
// and switches news off per app (on the page, also once it's released, and in the library);
// everyone reads the newest three and older ones on request; a post can be reported and hidden,
// and appealed; the library's Activity shows it. One new post per app per day can't be undone, so
// every test makes its own app, with the retry in its address, and reads only its own posts and
// notices. At the end Pat's marks and the notices these apps caused are taken away again, and the
// apps leave the store.
import type { Browser, Page } from "@playwright/test";

import { openItem } from "./admin-helpers";
import { expect, signIn, test, type Who } from "./fixtures";
import { call, delist, failApi, listedApp, path, released, tryCall } from "./store-helpers";

const made: string[] = [];
const titles: string[] = [];

/** This test's own app (Bo's, Coming soon unless said), in the store. */
async function newsApp(name: string, title: string, stage: "coming_soon" | "beta" = "coming_soon") {
  const retry = test.info().retry;
  const slug = `news-${name}-${retry}`;
  const full = retry ? `${title} ${retry}` : title;
  made.push(slug);
  titles.push(full);
  await listedApp(slug, full, { who: "maker2", stage });
  return { slug, title: full };
}

/** Bo posts news through the API, as the panel would. */
const post = (slug: string, headline: string, body = "Made by the news browser tests.") =>
  call<{ id: string }>("maker2", "POST", `/v1/apps/${slug}/news`, { headline, body });

interface Notice {
  id: string;
  category: string;
  title: string;
  body: string;
  link: string;
  read: boolean;
}

/** `who`'s notices under the bell (news included) that open one of these apps' pages. */
async function noticesAbout(who: Who, slug: string): Promise<Notice[]> {
  const inbox = await call<{ items: Notice[] }>(who, "GET", "/v1/me/notifications?bell=true&limit=100");
  return inbox.items.filter((n) => n.link === `/app/${slug}` || n.link.startsWith(`/app/${slug}#`));
}

async function person(browser: Browser, who: Who | null): Promise<Page> {
  const page = await browser.newPage();
  if (who) await signIn(page, who);
  return page;
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/** Reloads the page and waits for its news to have come back, so "not shown" is a real answer. */
async function reloadWithNews(page: Page, slug: string) {
  const [news] = await Promise.all([
    page.waitForResponse((r) => new URL(r.url()).pathname === `/v1/apps/${slug}/news` && r.ok()),
    page.reload(),
  ]);
  await news.finished();
}
const newsSection = (page: Page) => page.getByRole("region", { name: "News", exact: true });
const newsSwitch = (scope: Page | ReturnType<Page["locator"]>) =>
  scope.getByRole("checkbox", { name: "Tell me about news from this app" });

test.afterAll(async () => {
  for (const slug of made) {
    await tryCall("player", "DELETE", `/v1/me/interested/${slug}`);
    for (const n of await noticesAbout("player", slug)) await tryCall("player", "DELETE", `/v1/me/notifications/${n.id}`);
  }
  // Bo's decision about the hidden post and his appeal's answer.
  const bo = await call<{ items: Notice[] }>("maker2", "GET", "/v1/me/notifications?bell=true&limit=100");
  for (const n of bo.items.filter((n) => titles.some((t) => n.title.includes(t)))) {
    await tryCall("maker2", "DELETE", `/v1/me/notifications/${n.id}`);
  }
  await tryCall("maker2", "GET", "/v1/me/decisions");
  await delist(...made);
});

test("a maker posts news and an Interested player is told", async ({ page, browser }) => {
  const app = await newsApp("told", "News Kite");
  await call("player", "PUT", `/v1/me/interested/${app.slug}`);

  await signIn(page, "maker2");
  await page.goto(`/publish/${app.slug}/manage`);
  const panel = newsSection(page);
  await expect(panel.getByText("One new post per app per day.", { exact: false })).toBeVisible();
  await panel.getByRole("button", { name: "Write news" }).click();
  const headline = panel.getByRole("textbox", { name: "Headline", exact: true });
  await expect(headline).toBeFocused();
  await headline.fill("Beta opens on Friday");
  await panel.getByRole("textbox", { name: "News", exact: true }).fill("The first three areas are **ready**.");
  await panel.getByRole("button", { name: "Post news" }).click();
  await expect(panel.getByRole("listitem").filter({ hasText: "Beta opens on Friday" })).toBeVisible();
  await expect(panel.getByText(/^You can post again on /)).toBeVisible();
  await expect(panel.getByRole("button", { name: "Write news" })).toHaveCount(0);

  // Pat's bell: one notice, under News.
  const pat = await person(browser, "player");
  await pat.goto("/notifications/news");
  const row = pat.locator(".notifications li").filter({ hasText: `${app.title}: news from its maker` });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("Beta opens on Friday");
  await row.getByRole("button", { name: new RegExp(escape(`${app.title}: news from its maker`)) }).click();
  await expect(pat).toHaveURL(new RegExp(`/app/${app.slug}#news$`));
  await expect(pat.getByRole("heading", { name: "News", level: 2, exact: true })).toBeInViewport();
  await expect(newsSection(pat).getByRole("heading", { name: "Beta opens on Friday", level: 3 })).toBeVisible();

  // Signed out, the post is on the page too.
  const visitor = await person(browser, null);
  await visitor.goto(`/app/${app.slug}`);
  await expect(newsSection(visitor).getByRole("heading", { name: "Beta opens on Friday" })).toBeVisible();
  await expect(newsSection(visitor).locator("strong")).toHaveText("ready");
});

test("the news switch: on the page, kept after a reload, refused with the reason, and the same in the library", async ({ page }) => {
  const app = await newsApp("switch", "News Lark");
  await tryCall("player", "DELETE", `/v1/me/interested/${app.slug}`);
  await signIn(page, "player");
  await page.goto(`/app/${app.slug}`);
  await page.getByRole("button", { name: "I'm interested", exact: true }).click();
  const box = newsSwitch(page);
  await expect(box).toBeChecked();
  await expect(page.getByText(/It's in your library; we'll tell you when it reaches beta or is released\./)).toBeVisible();

  // Refused: it says why and stays on.
  await failApi(page, path(`/v1/me/interested/${app.slug}/news`), { status: 429, code: "rate_limited", message: "too many Interested marks for now: try again in a while" });
  await box.click();
  await expect(page.locator(".interested").getByRole("alert")).toHaveText("too many Interested marks for now: try again in a while");
  await expect(box).toBeChecked();
  await page.unrouteAll({ behavior: "ignoreErrors" });

  await box.click();
  await expect(box).not.toBeChecked();
  await page.reload();
  await expect(newsSwitch(page)).not.toBeChecked();

  // Bo posts: Pat isn't told.
  await post(app.slug, "Nobody here asked");
  expect(await noticesAbout("player", app.slug)).toEqual([]);

  // The library shows the same switch, off.
  await page.goto(`/library/${app.slug}`);
  const entry = page.getByRole("region", { name: app.title, exact: true });
  await expect(newsSwitch(entry)).not.toBeChecked();
  await expect(entry.getByText("News: Nobody here asked")).toBeVisible();
});

test("a released app keeps the switch in its News section", async ({ page }) => {
  test.slow();
  const app = await newsApp("released", "News Wren");
  await call("player", "PUT", `/v1/me/interested/${app.slug}`);
  await released(app.slug);
  await call("admin", "POST", `/v1/admin/apps/${app.slug}/stage`, { stage: "released" });
  await post(app.slug, "Out now, with saves");

  await signIn(page, "player");
  await page.goto(`/app/${app.slug}`);
  await expect(page.getByRole("heading", { name: app.title, level: 1 })).toBeVisible();
  // No Interested button for a released app: the switch is in the News section.
  await expect(page.getByRole("button", { name: /interested/i })).toHaveCount(0);
  const section = newsSection(page);
  await expect(section.getByRole("heading", { name: "Out now, with saves" })).toBeVisible();
  await expect(newsSwitch(section)).toBeChecked();
  await newsSwitch(section).click();
  await expect(newsSwitch(section)).not.toBeChecked();
  await page.reload();
  await expect(newsSwitch(newsSection(page))).not.toBeChecked();

  // In the library, a released app marked before release has it in Activity.
  await page.goto(`/library/${app.slug}`);
  const entry = page.getByRole("region", { name: app.title, exact: true });
  await expect(entry.getByText("Released: add it to your library to install it.")).toBeVisible();
  await expect(newsSwitch(entry)).not.toBeChecked();
});

test("older news: the newest three, then ten more on request, with the focus on the first new one", async ({ page }) => {
  const app = await newsApp("older", "News Heron");
  // One post a day can't make thirteen: the answers are the test's.
  const posts = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => ({
      id: crypto.randomUUID(),
      headline: `Old news ${from + i}`,
      body: `Post number ${from + i}.`,
      created_at: new Date(Date.UTC(2026, 9, 30 - from - i, 12)).toISOString(),
      edited: false,
      hidden: false,
    }));
  const first = posts(1, 3);
  const second = posts(4, 10);
  const asked: Array<string | null> = [];
  await page.route(path(`/v1/apps/${app.slug}/news`), (route) => {
    const q = new URL(route.request().url()).searchParams;
    asked.push(q.get("before"));
    return route.fulfill({ json: q.get("before") ? { news: second, more: false } : { news: first, more: true } });
  });
  const older = (u: URL) => u.pathname === `/v1/apps/${app.slug}/news` && u.searchParams.has("before");
  await failApi(page, older);

  await page.goto(`/app/${app.slug}`);
  const section = newsSection(page);
  await expect(section.getByRole("listitem")).toHaveCount(3);
  const more = section.getByRole("button", { name: "Show older news" });
  await more.click();
  await expect(section.getByRole("alert")).toHaveText("Something broke on SLAPorSLOP's side.");
  await expect(section.getByRole("listitem")).toHaveCount(3);

  await page.unroute(older);
  await more.click();
  await expect(section.getByRole("listitem")).toHaveCount(13);
  await expect(section.getByRole("heading", { name: "Old news 4", level: 3 })).toBeFocused();
  await expect(more).toHaveCount(0);
  await expect(section.getByRole("alert")).toHaveCount(0);
  await expect(section.locator("li").last()).toHaveAttribute("id", `news-${second[9].id}`);
  // The newest three first, then ten older than the last one shown.
  expect(asked).toEqual([null, first[2].id]);
});

test("editing tells nobody again; removing asks first and takes the notice away", async ({ page, browser }) => {
  const app = await newsApp("edit", "News Finch");
  await call("player", "PUT", `/v1/me/interested/${app.slug}`);
  await post(app.slug, "Fixes for the crash", "The crash on start is fixed.");

  await signIn(page, "maker2");
  await page.goto(`/publish/${app.slug}/manage`);
  const panel = newsSection(page);
  await panel.getByRole("button", { name: "Edit: Fixes for the crash" }).click();
  const headline = panel.getByRole("textbox", { name: "Headline", exact: true });
  await expect(headline).toHaveValue("Fixes for the crash");
  await headline.fill("Fixes for both crashes");
  await panel.getByRole("textbox", { name: "News", exact: true }).fill("Both crashes on start are fixed.");
  await panel.getByRole("button", { name: "Save changes" }).click();
  const edit = panel.getByRole("button", { name: "Edit: Fixes for both crashes" });
  await expect(edit).toBeFocused();

  const visitor = await person(browser, null);
  await visitor.goto(`/app/${app.slug}`);
  await expect(newsSection(visitor).getByRole("heading", { name: "Fixes for both crashes" })).toBeVisible();
  await expect(newsSection(visitor).getByText("Both crashes on start are fixed.")).toBeVisible();
  // Pat still has the one notice, which says the new headline.
  const told = await noticesAbout("player", app.slug);
  expect(told.map((n) => [n.category, n.body])).toEqual([["news", "Fixes for both crashes"]]);

  // Remove, then Keep it: nothing changes.
  const remove = panel.getByRole("button", { name: "Remove: Fixes for both crashes" });
  await remove.click();
  await expect(panel.getByText("Remove this post? It doesn't give the day's post back.")).toBeVisible();
  await expect(panel.getByRole("button", { name: "Keep it" })).toBeFocused();
  await panel.getByRole("button", { name: "Keep it" }).click();
  await expect(remove).toBeFocused();
  await expect(panel.getByText("Fixes for both crashes")).toBeVisible();

  await remove.click();
  await panel.getByRole("button", { name: "Remove post" }).click();
  await expect(panel.getByText("Fixes for both crashes")).toHaveCount(0);
  await expect(panel.getByRole("heading", { name: "News", level: 2 })).toBeFocused();
  // Removing doesn't give the day back.
  await expect(panel.getByText(/^You can post again on /)).toBeVisible();
  expect(await noticesAbout("player", app.slug)).toEqual([]);
  await reloadWithNews(visitor, app.slug);
  await expect(visitor.getByRole("heading", { name: app.title, level: 1 })).toBeVisible();
  await expect(newsSection(visitor)).toHaveCount(0);
});

test("anyone can report a post; staff hide it, the maker sees why and appeals", async ({ page, browser }) => {
  test.slow();
  const app = await newsApp("hidden", "News Crow");
  const headline = "Vote Slaps and get a key";
  await post(app.slug, headline, "Everyone who votes Slaps gets a key for our next game.");

  // Signed out, Report is there and asks who's reporting.
  await page.goto(`/app/${app.slug}`);
  const item = newsSection(page).getByRole("listitem").filter({ hasText: headline });
  await item.getByRole("button", { name: "Report" }).click();
  const dialog = page.getByRole("dialog", { name: "Report this news post" });
  await dialog.getByRole("radio", { name: "Spam" }).check();
  await dialog.getByRole("textbox", { name: /What did you see/ }).fill("It offers a key for votes.");
  await dialog.getByRole("textbox", { name: /Your name/ }).fill("Vic Visitor");
  await dialog.getByRole("textbox", { name: /Your email/ }).fill("vic@example.com");
  await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("heading", { name: "Thanks: we've got your report" })).toBeVisible();
  const reference = (await dialog.locator("b.mono").textContent())!.trim();

  // Staff hide it from the queue.
  const admin = await person(browser, "admin");
  await openItem(admin, "reports", reference);
  const card = admin.getByRole("listitem").filter({ hasText: reference });
  await expect(card.getByText("The post", { exact: true })).toBeVisible();
  await card.getByRole("button", { name: "Decide" }).click();
  await expect(card.getByRole("combobox", { name: "What to do" }).locator("option")).toHaveText([
    "Hide the news post",
    "Warn them (no restriction)",
    "Suspend their account",
  ]);
  await card.getByRole("textbox", { name: /The facts/ }).fill("The post offers a key for Slaps votes.");
  await card.getByRole("textbox", { name: "What the reporter is told" }).fill("We hid the post.");
  await card.getByRole("button", { name: "Hide the news post" }).click();
  await expect(card).toHaveCount(0);

  // Gone from the page.
  await reloadWithNews(page, app.slug);
  await expect(page.getByRole("heading", { name: app.title, level: 1 })).toBeVisible();
  await expect(page.getByText(headline)).toHaveCount(0);

  // Bo's panel keeps it, marked, with the rule and the way to appeal, and no buttons.
  const bo = await person(browser, "maker2");
  await bo.goto(`/publish/${app.slug}/manage`);
  const row = newsSection(bo).getByRole("listitem").filter({ hasText: headline });
  await expect(row.getByText("HIDDEN", { exact: true })).toBeVisible();
  await expect(row.getByText(/^Removed by a moderation decision: Spam\. You can appeal it from/)).toBeVisible();
  await expect(row.getByRole("link", { name: "your account" })).toHaveAttribute("href", "/account#decisions");
  await expect(row.getByRole("button")).toHaveCount(0);

  // The yearly report counts it.
  const visitor = await person(browser, null);
  await visitor.goto("/transparency");
  await expect(visitor.getByRole("cell", { name: "News post hidden", exact: true }).first()).toBeVisible();

  // Bo appeals; the person deciding reads what was hidden, and the decision stands.
  await row.getByRole("link", { name: "your account" }).click();
  const decision = bo.getByRole("article", { name: `News post hidden: News from ${app.title}: ${headline}` });
  await decision.getByRole("button", { name: "Appeal this decision" }).click();
  await decision.getByRole("textbox", { name: /Why should we look again/ }).fill("It was a joke.");
  await decision.getByRole("button", { name: "Send appeal" }).click();
  await expect(decision.getByText(/You appealed on/)).toBeVisible();

  await openItem(admin, "appeals", `News from ${app.title}`);
  const appeal = admin
    .getByRole("listitem")
    .filter({ hasText: `News from ${app.title}` })
    .filter({ has: admin.getByRole("button", { name: "Reverse the decision" }) });
  await expect(appeal.getByText("What was hidden", { exact: true })).toBeVisible();
  await expect(appeal.locator("blockquote")).toContainText(headline);
  await expect(appeal.locator("blockquote")).toContainText("Everyone who votes Slaps gets a key for our next game.");
  await appeal.getByRole("textbox", { name: /Why \(the person reads this\)/ }).fill("Offering keys for votes isn't allowed.");
  await appeal.getByRole("button", { name: "Keep it" }).click();
  await expect(appeal).toHaveCount(0);

  // Appealed once and kept: the panel no longer offers an appeal.
  await bo.goto(`/publish/${app.slug}/manage`);
  const kept = newsSection(bo).getByRole("listitem").filter({ hasText: headline });
  await expect(kept.getByText(/^Removed by a moderation decision: Spam\. See the decision on your account\.$/)).toBeVisible();
});

test("the library's Activity shows the maker's news", async ({ page }) => {
  const app = await newsApp("library", "News Robin");
  await call("player", "PUT", `/v1/me/interested/${app.slug}`);
  await post(app.slug, "Five new levels are in", "Five new levels and a harder mode.");

  await signIn(page, "player");
  await page.goto(`/library/${app.slug}`);
  const entry = page.getByRole("region", { name: app.title, exact: true });
  const activity = entry.getByRole("tabpanel");
  await expect(activity.getByText("News: Five new levels are in")).toBeVisible();
  await expect(activity.getByText("Five new levels and a harder mode.")).toBeVisible();
  await expect(activity.getByRole("button", { name: "Report" })).toBeVisible();
  // Waiting for it: the switch is with the Interested bar, on.
  await expect(newsSwitch(entry)).toBeChecked();
});
