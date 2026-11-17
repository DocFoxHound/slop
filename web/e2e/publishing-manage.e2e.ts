// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 3: the Manage page. Which release players get (straight away: the maker
// published it), moving the repository (checked by SLAPorSLOP staff), withdrawing a release not
// published yet, requests and cancelling them, and putting a draft in the store the first time;
// and, read-only on the seeded Lantern Keep, ratings, its history, how each release landed and bug
// triage.
import AxeBuilder from "@axe-core/playwright";

import type { AppHistory } from "../src/api";
import { expect, ids, signIn, test } from "./fixtures";
import { answer, call, createApp, fakeRepo, publish, readNotices, submitRelease, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

// Mia's app for this file: two published releases, from the fake host.
const slug = uniq("pub-kite");
const title = `Kite Runner ${slug.slice(-4)}`;
const repo = fakeRepo("mia", slug);
const manage = `/publish/${slug}/manage`;

test.beforeAll(async () => {
  await createApp("maker", slug, title, repo, { bug_reports: false });
  await publish("maker", slug, await submitRelease("maker", slug, repo, "1.0.0"));
  await publish("maker", slug, await submitRelease("maker", slug, repo, "1.1.0"));
});

test.afterAll(async () => {
  await readNotices("maker", slug);
});

test("only the maker manages an app", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto(manage);
  await expect(visitor.getByText("to manage your app.")).toBeVisible();
  await expect(visitor.getByRole("link", { name: "Sign in" }).last()).toHaveAttribute("href", `/signin?return_to=${manage}`);
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(manage);
  await expect(bo.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});

test("what players get: hold them at a release, then follow the newest again", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  await expect(page.getByRole("heading", { name: title, level: 1 })).toBeVisible();
  // The maker's pages for the app are tabs.
  const tabs = page.getByRole("navigation", { name: `${title}: maker pages` });
  await expect(tabs.getByRole("link", { name: "Manage" })).toHaveAttribute("aria-current", "page");
  await expect(tabs.getByRole("link", { name: "Edit store page" })).toHaveAttribute("href", `/publish/${slug}`);
  await expect(tabs.getByRole("link", { name: "Releases" })).toHaveAttribute("href", `/publish/${slug}/releases`);
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "What players get" }) });
  await expect(section).toContainText("Players get v1.1.0 NEWEST PUBLISHED");
  const choice = section.getByRole("radiogroup", { name: "Release players get" });
  await expect(choice.getByRole("radio", { name: /Always the newest published release/ })).toBeChecked();
  const send = section.getByRole("button", { name: "Switch" });
  await expect(send).toBeDisabled();

  // Mia published it, so holding them at 1.0.0 happens at once.
  await choice.getByRole("radio", { name: /Hold players at v1\.0\.0/ }).check();
  await expect(section.getByText(/You published this release, so it goes out straight away\./)).toBeVisible();
  await section.getByRole("textbox", { name: /Why/ }).fill("1.1.0 crashes on start");
  await section.getByRole("button", { name: "Switch" }).click();
  await expect(section.getByText(/^Done\./)).toBeVisible();
  await expect(section).toContainText("Players get v1.0.0 HELD AT THIS RELEASE");
  await expect(section.getByText("v1.1.0 is published but isn't going out, because you're holding players at v1.0.0.")).toBeVisible();
  const request = page.locator("ul.release-list > li").filter({ hasText: "Hold players at v1.0.0" });
  await expect(request).toContainText("DONE AUTOMATICALLY");
  await expect(request).toContainText("Your note: 1.1.0 crashes on start");

  // Back to the newest.
  await choice.getByRole("radio", { name: /Always the newest published release/ }).check();
  await section.getByRole("button", { name: "Switch" }).click();
  await expect(section).toContainText("Players get v1.1.0 NEWEST PUBLISHED");
  await expect(page.locator("ul.release-list > li").filter({ hasText: "Follow the newest published release" })).toContainText("DONE");
});

test("the update settings are on Manage too, with nothing to approve automatically", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  const panel = page.locator("section.updates-panel");
  await expect(panel.getByRole("checkbox", { name: /Pick up new GitHub releases/ })).toBeVisible();
  await expect(panel.getByRole("checkbox", { name: /Approve updates automatically/ })).toHaveCount(0);
});

test("moving the repository: players have it, so SLAPorSLOP staff check the move first", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "Repository", exact: true }) });
  await expect(section.getByRole("link", { name: repo })).toHaveAttribute("href", repo);
  await expect(
    section.getByText(
      /Players already have this app, so a move is checked by SLAPorSLOP staff\. The first release from the new repository goes through the same checks, and you publish it\./,
    ),
  ).toBeVisible();
  const request = section.getByRole("button", { name: "Request the move" });
  await expect(request).toBeDisabled();

  // A repository that's another app's.
  await section.getByRole("textbox", { name: "Move to" }).fill("https://github.com/e2e/lantern-keep");
  await request.click();
  await expect(section.locator(".form-error")).toHaveText(
    "that repository is already on SLAPorSLOP as Lantern Keep. To build on it, fork it and register a branch",
  );

  const moved = fakeRepo("mia-studio", slug);
  await section.getByRole("textbox", { name: "Move to" }).fill(moved);
  await section.getByRole("textbox", { name: /Why/ }).fill("Moved to the studio's account.");
  await request.click();
  await expect(section.getByText(/^Sent to SLAPorSLOP staff\./)).toBeVisible();
  await expect(section.getByText(`Waiting for SLAPorSLOP staff: move to ${moved}.`)).toBeVisible();
  await expect(section.getByRole("textbox", { name: "Move to" })).toHaveValue("");
  const row = page.locator("ul.release-list > li").filter({ hasText: `Move to ${moved}` });
  await expect(row).toContainText("WAITING FOR SLAPorSLOP STAFF");
  await expect(row).toContainText("Your note: Moved to the studio's account.");
  // Still the old repository until staff approve.
  await expect(section.getByRole("link", { name: repo })).toBeVisible();
  await row.getByRole("button", { name: "Cancel request" }).click();
  await expect(row).toContainText("CANCELLED");
  await expect(section.getByText(/Waiting for SLAPorSLOP staff: move to/)).toBeHidden();
});

test("a release not published yet can be published from Releases, or withdrawn", async ({ page }) => {
  await submitRelease("maker", slug, repo, "1.2.0");
  await signIn(page, "maker");
  await page.goto(manage);
  const waiting = page.locator("section").filter({ has: page.getByRole("heading", { name: "Not published yet" }) });
  await expect(waiting).toContainText("Nothing here reaches players until you publish it.");
  const row = waiting.getByRole("listitem").filter({ hasText: "v1.2.0" });
  await expect(row).toContainText("Passed, not published, submitted from your account");
  await expect(row.getByRole("link", { name: "Publish it on Releases" })).toHaveAttribute(
    "href",
    new RegExp(`^/publish/${slug}/releases#release-[0-9a-f-]+$`),
  );
  // Asked first; saying no keeps it.
  page.once("dialog", (d) => {
    expect(d.message()).toBe("Withdraw v1.2.0? It won't be published or reach players.");
    void d.dismiss();
  });
  await row.getByRole("button", { name: "Withdraw" }).click();
  await expect(row).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: "Withdraw" }).click();
  await expect(waiting).toBeHidden();
  await page.goto(`/publish/${slug}/releases`);
  await expect(page.locator("ul.release-list > li").filter({ hasText: "1.2.0" }).getByText("WITHDRAWN BEFORE PUBLISHING")).toBeVisible();
});

test("pictures only come from GitHub repositories so far", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(`/publish/${slug}`);
  const media = page.locator("section.media-editor");
  await expect(media.getByText("Pictures from the repository only work for GitHub repositories so far.")).toBeVisible();
  await expect(media.getByRole("button", { name: "Check my repository now" })).toBeDisabled();
  // Players have it: the repository moves only through Manage.
  await expect(page.getByRole("link", { name: "Request a move from Manage" })).toHaveAttribute("href", manage);
});

test("ratings, release feedback and bug reports on a new app: empty, and reports turned off", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  const ratings = page.locator("section").filter({ has: page.getByRole("heading", { name: "Ratings" }) });
  await expect(ratings.getByText("No votes yet.")).toBeVisible();
  const landed = page.getByRole("region", { name: "How each release landed" });
  await expect(landed.getByRole("listitem").filter({ hasText: "1.1.0" })).toContainText("No votes on it yet");
  const bugs = page.getByRole("region", { name: "Bug reports" });
  await expect(bugs.getByText("You've turned bug reports off on the store page.")).toBeVisible();
  await expect(bugs.getByText("None.")).toBeVisible();
  // With nothing to show, how releases landed isn't shown at all.
  await answer(page, `/v1/apps/${slug}/release-feedback`, []);
  await page.reload();
  await expect(page.getByRole("region", { name: "Bug reports" })).toBeVisible();
  await expect(landed).toHaveCount(0);
});

test("community builds of a game that counts players are told about Slop Pulse", async ({ page }) => {
  await signIn(page, "maker2");
  const build = ids.communityBuild;
  const overview = await call<Record<string, unknown>>("maker2", "GET", `/v1/apps/${build}/manage`);
  await answer(page, `/v1/apps/${build}/manage`, { ...overview, pulse_required: false });
  await page.goto(`/publish/${build}/manage`);
  const pulse = page.locator("section").filter({ has: page.getByRole("heading", { name: "Slop Pulse" }) });
  await expect(pulse).toContainText("this one can't go in the store before its first Pulse handshake");
  await expect(pulse.getByRole("link", { name: "publishing guide" })).toHaveAttribute("href", "/publish/guide#slop-pulse-player-counts");
  await page.unroute(`**/v1/apps/${build}/manage`);
  await answer(page, `/v1/apps/${build}/manage`, { ...overview, pulse_required: true });
  await page.reload();
  await expect(pulse).toContainText("✓ SLAPorSLOP has had a Pulse handshake from this build");
  // Lantern Keep doesn't count players: no Pulse section for its build.
  await page.unroute(`**/v1/apps/${build}/manage`);
  await page.reload();
  await expect(page.getByRole("heading", { name: "What players get" })).toBeVisible();
  await expect(pulse).toHaveCount(0);
});

test("Lantern Keep's maker reads its ratings, how its release landed, and its bug reports", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(`/publish/${ids.listedApp}/manage`);
  const ratings = page.locator("section").filter({ has: page.getByRole("heading", { name: "Ratings" }) });
  const byVersion = ratings.getByRole("table", { name: "Version" });
  await expect(byVersion.getByRole("row", { name: /v1\.0\.0/ })).toContainText(/\d+%/);
  await expect(ratings.getByRole("table", { name: "Platform" }).getByRole("row", { name: /Windows/ })).toBeVisible();
  await expect(ratings.getByRole("link", { name: "How ratings work" })).toHaveAttribute("href", "/ratings");

  const landed = page.getByRole("region", { name: "How each release landed" });
  await expect(landed.getByRole("listitem").filter({ hasText: "1.0.0" })).toContainText(/\d+% Slaps from \d+ votes · \d+ Slaps and \d+ Slop reviews · \d+ bug reports?/);

  // Triage: open ones first, by status; each can go to GitHub as an issue, filled in.
  const bugs = page.getByRole("region", { name: "Bug reports" });
  const fog = bugs.getByRole("listitem").filter({ hasText: "Fog covers the menu after alt-tab" });
  await expect(fog).toContainText("Pat Player · 1.0.0 on windows");
  await expect(fog).toContainText("1 more with it");
  const issue = new URL((await fog.getByRole("link", { name: "Open as a GitHub issue" }).getAttribute("href"))!);
  expect(issue.origin + issue.pathname).toBe("https://github.com/e2e/lantern-keep/issues/new");
  expect(issue.searchParams.get("title")).toBe("Fog covers the menu after alt-tab");
  expect(issue.searchParams.get("body")).toContain("Version: 1.0.0 · Platform: windows · 2 players on SLAPorSLOP");
  await expect(fog.getByRole("button", { name: "Mark fixed" })).toBeVisible();
  await expect(fog.getByRole("button", { name: "Open again" })).toHaveCount(0);
  await expect(bugs.getByRole("listitem").filter({ hasText: "Crash on level 3" })).toHaveCount(0);

  await bugs.getByLabel("Show").selectOption("fixed");
  const crash = bugs.getByRole("listitem").filter({ hasText: "Crash on level 3" });
  await expect(crash).toContainText("FIXED");
  await expect(crash.getByRole("textbox", { name: "Note for the player" })).toHaveValue("Fixed in 1.0.1.");
  await expect(crash.getByRole("button", { name: "Open again" })).toBeVisible();
  await crash.getByText("Last output the player sent").click();
  await expect(crash.locator("pre")).toHaveText("thread 'main' panicked at src/level.rs:88");
  await expect(bugs.getByRole("listitem").filter({ hasText: "Fog covers the menu" })).toHaveCount(0);

  await bugs.getByLabel("Show").selectOption("wont_fix");
  await expect(bugs.getByText("None.")).toBeVisible();
  await bugs.getByLabel("Show").selectOption("all");
  await expect(bugs.getByRole("listitem").filter({ hasText: /Fog covers|Crash on level 3|Text too small/ })).toHaveCount(3);
});

test("Lantern Keep's maker reads its history: players, first plays and library adds per day", async ({ page }) => {
  // Seeded players launched it through the real route (examples/e2e_server/seed.rs), so the seed
  // itself made counts. Not tied to "today": a run across UTC midnight still passes.
  const h = await call<AppHistory>("maker", "GET", `/v1/apps/${ids.listedApp}/history?days=30`);
  expect(h.days).toHaveLength(30);
  expect(h.days.some((d) => (d.players ?? 0) >= 1 && (d.first_plays ?? 0) >= 1)).toBe(true);

  await signIn(page, "maker");
  await page.goto(`/publish/${ids.listedApp}/manage`);
  const history = page.getByRole("region", { name: "History", exact: true });
  await expect(history).toBeVisible();
  await expect(history.getByText("Totals only, never who.")).toBeVisible();
  await expect(history.getByText("Players and first plays per day", { exact: true })).toBeVisible();
  await expect(history.getByText("Library adds per day", { exact: true })).toBeVisible();
  // The app was made when the test server started: the days before that weren't counted, and it says so.
  await expect(history.getByText(new RegExp(`^Nothing is counted before ${h.history_from}: the day the app was made`))).toBeVisible();

  // Every day in numbers, with at least one day someone played.
  await history.getByRole("button", { name: "Show numbers" }).first().click();
  const rows = history.getByRole("table").locator("tbody tr");
  await expect(rows).toHaveCount(30);
  await expect(rows.filter({ has: page.getByRole("cell", { name: /^[1-9]\d*$/ }) }).first()).toBeVisible();

  // Another range: the open table follows it.
  const days90 = history.getByRole("button", { name: "90 days" });
  await days90.click();
  await expect(days90).toHaveAttribute("aria-pressed", "true");
  await expect(history.getByRole("button", { name: "30 days" })).toHaveAttribute("aria-pressed", "false");
  await expect(rows).toHaveCount(90);
  await expect(history).toHaveAttribute("aria-busy", "false");

  const results = await new AxeBuilder({ page })
    .include('section[aria-labelledby="history-title"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
});

test("a Coming soon page goes in the store by its maker; the stage bar waits for a published release, then Beta and Released, only forward", async ({
  page,
}) => {
  const soon = uniq("pub-dawn");
  const soonTitle = `Dawn Patrol ${soon.slice(-4)}`;
  const soonRepo = fakeRepo("mia", soon);
  await createApp("maker", soon, soonTitle, soonRepo, { stage: "coming_soon" });
  await signIn(page, "maker");
  await page.goto(`/publish/${soon}/manage`);
  const bar = page.getByRole("region", { name: "Stage" });
  // The first listing is Mia's: a Coming soon page needs no release. Asked first.
  await expect(page.locator(".maker-head")).toContainText("DRAFT: NOT IN THE STORE YET");
  const list = page.getByRole("region", { name: "Not in the store yet" });
  await expect(list).toContainText("Only you can see this draft. Put it in the store when you're ready for players to find it: a Coming soon page needs no release.");
  await list.getByRole("button", { name: "Put it in the store…" }).click();
  const sure = list.getByRole("group", { name: "Put it in the store" });
  await expect(sure).toContainText("Anyone can find its page and get what you've published. Taking it out again is for SLAPorSLOP staff.");
  await sure.getByRole("button", { name: "Not yet" }).click();
  await expect(sure).toBeHidden();
  await list.getByRole("button", { name: "Put it in the store…" }).click();
  // The keyboard lands on the button that does it.
  await expect(sure.getByRole("button", { name: "Yes, put it in the store" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator(".maker-head")).toContainText("IN THE STORE");
  await expect(page.getByRole("region", { name: "Not in the store yet" })).toHaveCount(0);
  expect((await call<{ status?: string }>("maker", "GET", `/v1/apps/${soon}`)).status ?? "listed").toBe("listed");

  await expect(bar.getByRole("listitem").filter({ hasText: "Coming soon" })).toHaveAttribute("aria-current", "step");
  await expect(bar).toContainText("Must reach Beta by");
  // Nothing to play yet: the way on is a release.
  await expect(bar).toContainText("Move to Beta or Release it once you've published a release.");
  await expect(bar.getByRole("link", { name: "Submit a release" })).toHaveAttribute("href", `/publish/${soon}/releases`);
  await expect(bar.getByRole("button")).toHaveCount(0);

  await publish("maker", soon, await submitRelease("maker", soon, soonRepo, "0.1.0"));
  await page.reload();
  await bar.getByRole("button", { name: "Move to Beta…" }).click();
  const confirm = bar.getByRole("group", { name: "Move to Beta" });
  await expect(confirm).toContainText("players can play it, rate it");
  await expect(confirm).toContainText("Only forward");
  // Changing their mind leaves it as it was.
  await confirm.getByRole("button", { name: "Not yet" }).click();
  await expect(confirm).toBeHidden();
  await bar.getByRole("button", { name: "Move to Beta…" }).click();
  await confirm.getByRole("button", { name: "Move to Beta" }).click();
  await expect(bar.getByRole("listitem").filter({ hasText: "Beta" }).first()).toHaveAttribute("aria-current", "step");
  await expect(page.locator(".maker-head")).toContainText("BETA");
  await expect(bar).toContainText("Must be released by");
  await expect(bar.getByRole("button", { name: "Move to Beta…" })).toHaveCount(0);

  // The same bar on the other tabs; released from Releases.
  await page.getByRole("navigation", { name: /maker pages/ }).getByRole("link", { name: "Releases" }).click();
  await bar.getByRole("button", { name: "Release it…" }).click();
  await bar.getByRole("group", { name: "Release it" }).getByRole("button", { name: "Release it" }).click();
  await expect(bar).toContainText("Released: the last stage.");
  await expect(bar.getByRole("button")).toHaveCount(0);
  await expect(page.locator(".maker-head")).toContainText("RELEASED");
});
