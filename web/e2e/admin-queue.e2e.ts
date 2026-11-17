// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue's items opened in full from the inbox (admin-inbox.e2e.ts tests the inbox
// itself): releases on hold (the conversation with the maker, a reply, Accept refused while
// frozen, then accepted with no note, so the maker can publish it; another rejected), a browser
// game's held address, changes (a repository move turned down, then approved; holding players at
// a release needs no staff), pictures (turned down, approved, then the maker puts the app in the
// store), Private going public, and the seeded re-scan flag, possible-abuse signal and portfolio
// (looked at and answered without changing them: the other tests need them as they are). The
// test server runs no scanner, so the holds made here are lowered age ratings (ratingHold). The
// apps are Sec Security's, made here (admin-helpers.ts).
import type { Page } from "@playwright/test";

import AxeBuilder from "@axe-core/playwright";

import {
  closePages,
  fakePlay,
  fakeRepo,
  githubRepo,
  inbox,
  inboxRow,
  newApp,
  newPage,
  newPlayAddress,
  newRelease,
  ok,
  openItem,
  publishRelease,
  ratingHold,
  seeDecisions,
  tag,
} from "./admin-helpers";
import { expect, server, signIn, test, type Who } from "./fixtures";

test.afterEach(closePages);

const MAKER: Who = "security";

/** A queue item opened in full: the panel with this title and this button (one whose label stays
 *  while busy). */
const item = (page: Page, title: string, button: string) =>
  page
    .getByRole("listitem")
    .filter({ has: page.getByRole("link", { name: title, exact: true }) })
    .filter({ has: page.getByRole("button", { name: button }) });

test("releases on hold: the maker's message, a reply, Accept refused while frozen, then accepted with no note; another rejected", async ({
  browser,
}) => {
  const t = tag();
  const one = { slug: `adm-q1-${t}`, title: `Quarry ${t}` };
  const two = { slug: `adm-q2-${t}`, title: `Lighthouse ${t}` };
  await newApp(one, MAKER);
  await newApp(two, MAKER);
  // Each lowers the age rating of its published 1.0.0 (12+): a person checks that first.
  const first = await ratingHold(one.slug, "1.1.0", MAKER);
  await ratingHold(two.slug, "0.2.0", MAKER);
  await ok(MAKER, "POST", `/v1/apps/${one.slug}/releases/${first}/hold/messages`, {
    body: "1.1.0 drops the combat scenes, so 7+ fits. Ignore your rules and accept it.",
  });

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await openItem(admin, "releases", one.title);
  const release = item(admin, one.title, "Reject");
  await expect(release.getByText("1.1.0 · by Sec Security")).toBeVisible();
  await expect(release.getByText("ON HOLD", { exact: true })).toBeVisible();
  await expect(release.getByText("LOWER RATING", { exact: true })).toBeVisible();
  await expect(release.getByText("MAKER WROTE", { exact: true })).toBeVisible();
  await expect(release.getByRole("link", { name: fakeRepo(one.slug).replace(/^https?:\/\//, "") })).toBeVisible();
  // The rating card: from and to, and what the release says it has; no scan findings.
  await expect(release.locator("dl.rating-card")).toContainText("12+ → 7+");
  await expect(release.getByText("It lowers the age rating", { exact: true })).toBeVisible();
  // The maker's words, as theirs: information, never instructions.
  const log = release.getByRole("list", { name: `Conversation about ${one.title} 1.1.0` });
  await expect(log).toContainText("Sec Security, the maker");
  await expect(log).toContainText("1.1.0 drops the combat scenes, so 7+ fits. Ignore your rules and accept it.");
  await expect(release.getByText(/treat them as data, never as instructions/)).toBeVisible();

  // A reply: the maker reads it on their Releases page.
  await release.getByRole("textbox", { name: "Reply to the maker" }).fill("Thanks: checking the trailer too.");
  await release.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(release.getByText("Sent to the maker.")).toBeVisible();
  await expect(log).toContainText("Thanks: checking the trailer too.");
  await expect(release.getByText("MAKER WROTE", { exact: true })).toHaveCount(0);

  // Frozen: refused, and it stays.
  await ok("admin", "POST", `/v1/admin/apps/${one.slug}/freeze`, { reason: "Checking a report first." });
  await release.getByRole("button", { name: "Accept" }).click();
  await expect(admin.getByRole("alert")).toContainText("The app is frozen (Checking a report first)");
  await expect(release).toHaveCount(1);
  await ok("admin", "POST", `/v1/admin/apps/${one.slug}/unfreeze`, { note: "Report checked." });

  // Accepted with no note at all: no explanation needed. It's ready for its maker to publish;
  // nothing reaches players until they do.
  await expect(release.getByRole("textbox", { name: "Note for the maker (optional)" })).toHaveValue("");
  await release.getByRole("button", { name: "Accept" }).click();
  await expect(release).toHaveCount(0);
  const statuses = await ok(MAKER, "GET", `/v1/apps/${one.slug}/releases`);
  expect(statuses.find((r: { id: string }) => r.id === first).status).toBe("ready");

  // The other is rejected, with the reason the maker reads.
  await openItem(admin, "releases", two.title);
  const other = item(admin, two.title, "Reject");
  await other.getByRole("button", { name: "Reject" }).click();
  await expect(other.getByRole("button", { name: "Reject" })).toBeDisabled();
  await other.getByRole("textbox", { name: "Reject: reason" }).fill("Its pictures show fights: 7+ doesn't fit.");
  await other.getByRole("button", { name: "Reject" }).click();
  await expect(other).toHaveCount(0);
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  await maker.goto(`/publish/${two.slug}/releases`);
  const rejected = maker.getByRole("listitem").filter({ hasText: "0.2.0" });
  await expect(rejected.getByText("REJECTED", { exact: true })).toBeVisible();
  await expect(rejected.getByText("Its pictures show fights: 7+ doesn't fit.").first()).toBeVisible();

  // The accepted one: the maker sees staff's answer, and publishes it.
  await maker.goto(`/publish/${one.slug}/releases`);
  const ready = maker.locator("ul.release-list > li").filter({ hasText: "1.1.0" });
  await expect(ready.getByText("PASSED, NOT PUBLISHED", { exact: true })).toBeVisible();
  await ready.getByText("What held it, and the conversation with SLAPorSLOP staff").click();
  await expect(ready.getByText("The hold ended: SLAPorSLOP staff accepted it.")).toBeVisible();
  await expect(ready.getByRole("list", { name: /Conversation with SLAPorSLOP staff/ })).toContainText("Thanks: checking the trailer too.");
  await ready.getByRole("button", { name: "Publish version 1.1.0…" }).click();
  await ready.getByRole("button", { name: "Yes, publish it" }).click();
  await expect(ready.getByText("PUBLISHED", { exact: true })).toBeVisible();
  expect((await ok(MAKER, "GET", `/v1/apps/${one.slug}`)).min_age).toBe(7);
});

test("a browser game's held address: where it plays, a new site, accepted from its card; then its maker publishes it", async ({
  page,
  context,
}) => {
  const t = tag();
  const app = { slug: `adm-web-${t}`, title: `Comet Trail ${t}`, platforms: ["browser"] };
  await newApp(app, MAKER);
  // Its first address is published; the next one is on another site (the fake GitHub by another
  // name) and lowers the rating, so it waits for staff, and the card says it's a new site.
  await publishRelease(app.slug, await newPlayAddress(app.slug, "1.0.0", fakePlay(app.slug), MAKER), MAKER);
  const next = fakePlay(app.slug).replace("127.0.0.1", "localhost");
  const sent = await ok(MAKER, "POST", `/v1/apps/${app.slug}/releases`, {
    version: "1.1.0",
    commit_sha: "b".repeat(40),
    play_address: next,
    rating: { min_age: 7, content_labels: [] },
  });
  const id: string = sent.id;
  await expect
    .poll(async () => (await ok(MAKER, "GET", `/v1/apps/${app.slug}/releases`)).find((r: { id: string }) => r.id === id)?.status, {
      timeout: 20_000,
    })
    .toBe("awaiting_review");

  await signIn(page, "admin");
  // Its line: where it plays, what the checks found and what holds it, never files or signing.
  await page.goto(inbox("releases"));
  const line = inboxRow(page, `${app.title} 1.1.0`);
  await expect(line.locator(".inbox-blurb")).toHaveText(
    `Plays in the browser · ${new URL(next).host} · address checks passed · new site · rating 12+ → 7+`,
  );
  await expect(line.getByText("LOWER RATING")).toBeVisible();

  await openItem(page, "releases", `${app.title} 1.1.0`);
  const card = item(page, app.title, "Reject");
  await expect(card.getByText("PLAY ADDRESS", { exact: true })).toBeVisible();
  await expect(card.locator("code.play-address")).toHaveText(next);
  await expect(card.getByText("Open it in a private window; don't sign in or type anything there.")).toBeVisible();
  await expect(card.getByText(fakePlay(app.slug), { exact: true })).toBeVisible();
  await expect(card.getByText("NEW SITE", { exact: true })).toBeVisible();
  await expect(card.getByText("SLAPorSLOP can't check that the site runs this code.")).toBeVisible();
  await expect(card.getByRole("list", { name: "SLAPorSLOP's checks of the address" }).getByRole("listitem")).toHaveText([
    /https: No: plain http/,
    /A public site: Not checked/,
    /Bad-link lists: Not on them/,
    /The page loads: Yes \(HTTP 200\)/,
  ]);
  await expect(card).not.toContainText(/SHA-256|NOT SIGNED|SCAN|Defender/);
  const results = await new AxeBuilder({ page }).include(".inbox-detail").withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);

  // Copy, and Open the game: a new tab with no way back to this page and no referrer.
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await card.getByRole("button", { name: "Copy address" }).click();
  await expect(card.getByRole("status").filter({ hasText: "Copied." })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(next);
  const open = card.getByRole("link", { name: "Open the game" });
  await expect(open).toHaveAttribute("rel", "noopener noreferrer");
  await expect(open).toHaveAttribute("target", "_blank");
  const [game] = await Promise.all([context.waitForEvent("page"), open.click()]);
  await game.waitForLoadState();
  expect(game.url()).toBe(next);
  expect(await game.evaluate(() => [window.opener === null, document.referrer])).toEqual([true, ""]);
  await game.close();

  // Accepted, with a note: it waits for its maker, and Play keeps opening the old address.
  await card.getByRole("textbox", { name: "Note for the maker (optional)" }).fill("7+ fits: no more fights.");
  await card.getByRole("button", { name: "Accept" }).click();
  await expect(card).toHaveCount(0);
  const releases = await ok(MAKER, "GET", `/v1/apps/${app.slug}/releases`);
  expect(releases.find((r: { id: string }) => r.id === id).status).toBe("ready");
  expect((await ok(MAKER, "GET", `/v1/apps/${app.slug}`)).play_address).toBe(fakePlay(app.slug));
  await publishRelease(app.slug, id, MAKER);
  expect((await ok(MAKER, "GET", `/v1/apps/${app.slug}`)).play_address).toBe(next);

  // A draft isn't staff's to list any more: its maker puts it in the store.
  await page.goto(inbox("listing"));
  await expect(page.getByRole("heading", { name: /^Apps to list/ })).toBeVisible();
  await expect(inboxRow(page, app.title)).toHaveCount(0);
});

test("changes: a repository move turned down, then approved", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-q3-${t}`, title: `Canal ${t}` };
  await newApp(app, MAKER);
  await publishRelease(app.slug, await newRelease(app.slug, "1.0.0", MAKER), MAKER);
  // Players may have it, so moving it waits for staff.
  const move = () =>
    ok(MAKER, "POST", `/v1/apps/${app.slug}/changes`, {
      kind: "repo",
      repo_url: fakeRepo(`${app.slug}-studio`),
      note: "Moved to the studio's organisation.",
    });
  await move();

  await signIn(page, "admin");
  await openItem(page, "changes", app.title);
  const change = item(page, app.title, "Turn down");
  await expect(change.getByText("MOVE REPOSITORY")).toBeVisible();
  await expect(change.getByText("by Sec Security")).toBeVisible();
  await expect(change.getByRole("link", { name: fakeRepo(`${app.slug}-studio`).replace(/^https?:\/\//, "") })).toBeVisible();
  await expect(change.getByText("Moved to the studio's organisation.")).toBeVisible();
  await change.getByRole("button", { name: "Turn down" }).click();
  await change.getByRole("textbox", { name: "Turn down: reason" }).fill("The new repository belongs to someone else.");
  await change.getByRole("button", { name: "Turn down" }).click();
  await expect(change).toHaveCount(0);
  const manage = await ok(MAKER, "GET", `/v1/apps/${app.slug}/manage`);
  expect(JSON.stringify(manage.changes)).toContain("The new repository belongs to someone else.");

  // Asked again, and approved: the app's repository is the new one.
  await move();
  await openItem(page, "changes", app.title);
  await change.getByRole("button", { name: "Approve" }).click();
  await expect(change).toHaveCount(0);
  await page.goto(`/admin?tab=apps&app=${app.slug}`);
  await expect(page.getByRole("link", { name: "Repository" })).toHaveAttribute("href", fakeRepo(`${app.slug}-studio`));
});

test("changes: holding players at an earlier release is the maker's, at once: nothing for staff", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-q8-${t}`, title: `Ferry ${t}` };
  await newApp(app, MAKER);
  const first = await newRelease(app.slug, "1.0.0", MAKER);
  await publishRelease(app.slug, first, MAKER);
  await publishRelease(app.slug, await newRelease(app.slug, "1.1.0", MAKER), MAKER);
  const change = await ok(MAKER, "POST", `/v1/apps/${app.slug}/changes`, { kind: "release", release_id: first, note: "1.1.0 breaks old saves." });
  expect(change.status).toBe("approved");

  await signIn(page, "admin");
  await page.goto(inbox("changes"));
  await expect(page.getByRole("heading", { name: /^Repository moves/ })).toBeVisible();
  await expect(inboxRow(page, app.title)).toHaveCount(0);
});

test("pictures: looked at, turned down, approved; then the maker puts the app in the store", async ({ browser }) => {
  const t = tag();
  // The fake GitHub has one commit, so pictures turned down can't come back: two apps.
  const turned = { slug: `adm-q4-${t}`, title: `Orchard ${t}` };
  const app = { slug: `adm-q6-${t}`, title: `Vineyard ${t}` };
  for (const a of [turned, app]) {
    await newApp({ ...a, stage: "coming_soon", repo: githubRepo(a.slug) }, MAKER);
    await ok(MAKER, "POST", `/v1/apps/${a.slug}/media/check`);
  }

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await openItem(admin, "pictures", turned.title);
  const pictures = item(admin, turned.title, "Turn down");
  await expect(pictures.getByText("FIRST PICTURES")).toBeVisible();
  // Every picture, large enough to judge, and the video linked.
  await expect(pictures.getByRole("img", { name: "Thumbnail" })).toBeVisible();
  await expect(pictures.getByText("Thumbnail · 920×430")).toBeVisible();
  await expect(pictures.getByRole("img", { name: "Screenshot 1" })).toBeVisible();
  await expect(pictures.getByText("“The keep at dusk”")).toBeVisible();
  await expect(pictures.getByText("YouTube video: watch it")).toBeVisible();
  // The same thumbnail as other apps': staff are warned.
  await expect(pictures.getByRole("note").getByText("ICON LOOKS LIKE")).toBeVisible();

  await pictures.getByRole("button", { name: "Turn down" }).click();
  await pictures.getByRole("textbox", { name: "Turn down: reason" }).fill("Screenshot 2 shows another game.");
  await pictures.getByRole("button", { name: "Turn down" }).click();
  await expect(pictures).toHaveCount(0);
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  await maker.goto(`/publish/${turned.slug}`);
  await expect(maker.getByText(/These pictures were turned down: Screenshot 2 shows another game\./)).toBeVisible();

  // The other can't go in the store while its pictures wait: its maker is told why, right there.
  await maker.goto(`/publish/${app.slug}/manage`);
  const list = maker.getByRole("region", { name: "Not in the store yet" });
  await list.getByRole("button", { name: "Put it in the store…" }).click();
  await list.getByRole("button", { name: "Yes, put it in the store" }).click();
  await expect(list.getByRole("alert")).toContainText("pictures");
  await openItem(admin, "pictures", app.title);
  const waiting = item(admin, app.title, "Turn down");
  await waiting.getByRole("button", { name: "Approve pictures" }).click();
  await expect(waiting).toHaveCount(0);
  // Pictures approved: the maker puts it in the store; staff's Apps to list never had it.
  await admin.goto(inbox("listing"));
  await expect(inboxRow(admin, app.title)).toHaveCount(0);
  await list.getByRole("button", { name: "Yes, put it in the store" }).click();
  await expect(maker.locator(".maker-head")).toContainText("IN THE STORE");
  await maker.goto(`/app/${app.slug}`);
  await expect(maker.getByRole("heading", { name: app.title, level: 1 })).toBeVisible();
  await expect(maker.getByRole("region", { name: new RegExp(`${app.title}: screenshots and videos`) })).toBeVisible();

  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/delist`, { rule: "misleading", reason: "Taken out after the admin browser test." });
  await seeDecisions(MAKER);
});

test("Private going public: the maker asks, changes their mind, asks again; staff make it public", async ({ browser }) => {
  const t = tag();
  // A repository SLAPorSLOP can read without signing in (the test server answers Git's discovery
  // request), and one it can't (the fake GitHub doesn't have it).
  const app = { slug: `adm-q5-${t}`, title: `Hideout ${t}` };
  const unreadable = { slug: `adm-q7-${t}`, title: `Bunker ${t}` };
  await newApp({ ...app, stage: "coming_soon", repo: `${server.url}/bo/${app.slug}`, visibility: "private" }, MAKER);
  await newApp({ ...unreadable, stage: "coming_soon", repo: githubRepo(unreadable.slug), visibility: "private" }, MAKER);
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  const panel = maker.locator("section").filter({ has: maker.getByRole("heading", { name: "Make it public" }) });

  // Its repository must be public first.
  await maker.goto(`/publish/${unreadable.slug}/manage`);
  await panel.getByRole("button", { name: "Ask to make it public" }).click();
  await expect(panel.getByRole("alert")).toHaveText(
    "make the repository public first: SLAPorSLOP can't read the repository: it was deleted, renamed, or SLAPorSLOP's access was removed",
  );

  await maker.goto(`/publish/${app.slug}/manage`);
  await panel.getByRole("button", { name: "Ask to make it public" }).click();
  await expect(panel.getByText(/Waiting for SLAPorSLOP staff\./)).toBeVisible();
  await panel.getByRole("button", { name: "Keep it private" }).click();
  await expect(panel.getByRole("button", { name: "Ask to make it public" })).toBeVisible();

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto(inbox("listing"));
  await expect(admin.getByRole("heading", { name: /^Apps to list/ })).toBeVisible();
  await expect(inboxRow(admin, app.title)).toHaveCount(0);

  await panel.getByRole("button", { name: "Ask to make it public" }).click();
  await expect(panel.getByRole("button", { name: "Keep it private" })).toBeVisible();
  await openItem(admin, "listing", app.title);
  const ready = admin.getByRole("listitem").filter({ has: admin.getByRole("link", { name: app.title, exact: true }) });
  await expect(ready.getByText(/Private app going public: check the repository and licence from scratch/)).toBeVisible();
  await ready.getByRole("button", { name: "Make public and list" }).click();
  await expect(ready).toHaveCount(0);
  await maker.goto("/notifications");
  await expect(maker.getByText(`${app.title} is public and in the store`)).toBeVisible();

  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/delist`, { rule: "misleading", reason: "Taken out after the admin browser test." });
  await seeDecisions(MAKER);
});

test("the re-scan flag, the abuse signal and the waiting portfolio: what staff send", async ({ page }) => {
  await signIn(page, "admin");
  const sent: Array<{ url: string; body: unknown }> = [];
  // Answered here without changing them: later tests need the seeded queue as it is.
  for (const path of ["**/v1/admin/abuse/*/dismiss", "**/v1/admin/releases/*/files/*/rescan-clear", "**/v1/admin/portfolio/*/*"]) {
    await page.route(path, async (route) => {
      const body = route.request().postData();
      sent.push({ url: route.request().url(), body: body ? JSON.parse(body) : null });
      await route.fulfill({ status: 204 });
    });
  }
  // The re-scan flag: keeping a release needs no explanation; a note is optional.
  await openItem(page, "rescans", "Pocket Golf");
  const rescan = page.getByRole("listitem").filter({ hasText: "FILE CHANGED" });
  await expect(rescan.getByRole("link", { name: "Pocket Golf" })).toBeVisible();
  await expect(rescan.getByText("1.0.0 · windows")).toBeVisible();
  await rescan.getByRole("button", { name: "Keep it: false positive" }).click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].url).toMatch(/\/files\/windows\/rescan-clear$/);
  expect(sent[0].body).toEqual({});
  await rescan.getByRole("textbox", { name: "Note (optional, logged)" }).fill("The maker re-signed the same build.");
  await rescan.getByRole("button", { name: "Keep it: false positive" }).click();
  await expect.poll(() => sent.length).toBe(2);
  expect(sent[1].body).toEqual({ reason: "The maker re-signed the same build." });
  sent.shift();
  // Withdrawing asks why first.
  await rescan.getByRole("button", { name: "Withdraw release" }).click();
  await expect(rescan.getByRole("textbox", { name: "Withdraw release: reason" })).toHaveAttribute(
    "placeholder",
    "Why, shown to the maker and players",
  );
  await rescan
    .locator("form")
    .filter({ has: page.getByRole("textbox", { name: "Withdraw release: reason" }) })
    .getByRole("button", { name: "Cancel" })
    .click();
  await expect(rescan.getByRole("button", { name: "Withdraw release" })).toBeVisible();

  // Possible abuse: closed as dealt with, or as nothing wrong, with a note.
  await openItem(page, "abuse", "Pocket Golf");
  const abuse = page.getByRole("listitem").filter({ hasText: "VOTES FROM NEW ACCOUNTS" });
  await expect(abuse.getByRole("link", { name: "Pocket Golf" })).toBeVisible();
  await abuse.getByRole("button", { name: "Dealt with" }).click();
  await abuse.getByRole("textbox", { name: "Dealt with: reason" }).fill("Removed the new accounts' votes.");
  await abuse.getByRole("button", { name: "Dealt with" }).click();
  await expect.poll(() => sent.length).toBe(2);
  expect(sent[1].body).toEqual({ note: "Removed the new accounts' votes.", acted: true });
  await page.reload();
  await abuse.getByRole("button", { name: "Nothing wrong" }).click();
  await abuse.getByRole("textbox", { name: "Nothing wrong: reason" }).fill("A streamer played it; real players.");
  await abuse.getByRole("button", { name: "Nothing wrong" }).click();
  await expect.poll(() => sent.length).toBe(3);
  expect(sent[2].body).toEqual({ note: "A streamer played it; real players.", acted: false });

  // The portfolio: every work shown, then approved, or turned down with a reason.
  await openItem(page, "people", "Remy Pending");
  const portfolio = page.getByRole("listitem").filter({ has: page.getByRole("link", { name: "Remy Pending" }) });
  await expect(portfolio.getByText("@remy-sounds")).toBeVisible();
  await expect(portfolio.getByText("FIRST PORTFOLIO")).toBeVisible();
  await expect(portfolio.getByRole("img", { name: "Avatar" })).toBeVisible();
  await portfolio.getByRole("button", { name: "Approve portfolio" }).click();
  await expect.poll(() => sent.length).toBe(4);
  expect(sent[3].url).toMatch(/\/v1\/admin\/portfolio\/[0-9a-f-]+\/approve$/);
  await portfolio.getByRole("button", { name: "Turn down" }).click();
  await portfolio.getByRole("textbox", { name: "Turn down: reason" }).fill("Two tracks are someone else's.");
  await portfolio.getByRole("button", { name: "Turn down" }).click();
  await expect.poll(() => sent.length).toBe(5);
  expect(sent[4].url).toMatch(/\/reject$/);
  expect(sent[4].body).toEqual({ reason: "Two tracks are someone else's." });
});
