// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue's items opened in full from the inbox (admin-inbox.e2e.ts tests the inbox
// itself): releases (refused while frozen, scan holds cleared, approved, rejected), changes (a
// repository move turned down, then approved), pictures (turned down, approved, then listed from
// Apps to list), Private going public, and the seeded re-scan flag, possible-abuse signal and
// portfolio (looked at and answered without changing them: the other tests need them as they
// are). The apps are Sec Security's, made here (admin-helpers.ts).
import type { Page } from "@playwright/test";

import {
  approveRelease,
  closePages,
  fakeRepo,
  githubRepo,
  inbox,
  inboxRow,
  newApp,
  newPage,
  newRelease,
  ok,
  openItem,
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

test("releases: refused while the app is frozen, a scan hold cleared, approved; another rejected", async ({ browser }) => {
  const t = tag();
  const one = { slug: `adm-q1-${t}`, title: `Quarry ${t}` };
  const two = { slug: `adm-q2-${t}`, title: `Lighthouse ${t}` };
  await newApp(one, MAKER);
  await newApp(two, MAKER);
  const first = await newRelease(one.slug, "1.0.0", MAKER);
  await newRelease(two.slug, "0.1.0", MAKER);
  await ok("admin", "POST", `/v1/admin/apps/${one.slug}/freeze`, { reason: "Checking a report first." });

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await openItem(admin, "releases", one.title);
  const release = item(admin, one.title, "Reject");
  await expect(release.getByText("1.0.0 · by Sec Security")).toBeVisible();
  await expect(release.getByText("AWAITING REVIEW")).toBeVisible();
  await expect(release.getByRole("link", { name: fakeRepo(one.slug).replace(/^https?:\/\//, "") })).toBeVisible();
  await expect(release.getByText("bbbbbbbbbbbb", { exact: true })).toBeVisible();
  await expect(release.getByText("NOT CHECKED", { exact: true })).toBeVisible();
  await expect(release.getByText(`${one.slug}.zip · starts pong.exe`)).toBeVisible();
  await expect(release.getByText("NOT SIGNED", { exact: true })).toBeVisible();

  // Frozen: refused, and it stays.
  await release.getByRole("button", { name: "Approve and sign" }).click();
  await expect(admin.getByRole("alert")).toHaveText(
    "the app is frozen (Checking a report first.), so its releases can't be checked or approved",
  );
  await expect(release).toHaveCount(1);
  await ok("admin", "POST", `/v1/admin/apps/${one.slug}/unfreeze`, { note: "Report checked." });

  // The Windows build has no Defender check: clearing it takes a real reason.
  await release.getByRole("button", { name: "Clear as a false positive" }).click();
  const why = release.getByRole("textbox", { name: "Clear as a false positive: reason" });
  await why.fill("Fine");
  await release.getByRole("button", { name: "Clear as a false positive" }).click();
  await expect(admin.getByRole("alert")).toHaveText("say why it's a false positive (10 to 1,000 characters)");
  await why.fill("Built by the maker's own CI; the hash matches the source build.");
  await release.getByRole("button", { name: "Clear as a false positive" }).click();
  await expect(release.getByRole("button", { name: "Clear as a false positive" })).toHaveCount(0);
  await expect(admin.getByRole("alert")).toHaveCount(0);

  await release.getByRole("button", { name: "Approve and sign" }).click();
  await expect(release).toHaveCount(0);
  const statuses = await ok(MAKER, "GET", `/v1/apps/${one.slug}/releases`);
  expect(statuses.find((r: { id: string }) => r.id === first).status).toBe("approved");

  // The other is rejected, with the reason the maker reads.
  await openItem(admin, "releases", two.title);
  const other = item(admin, two.title, "Reject");
  await other.getByRole("button", { name: "Reject" }).click();
  await expect(other.getByRole("button", { name: "Reject" })).toBeDisabled();
  await other.getByRole("textbox", { name: "Reject: reason" }).fill("The zip has no program in it.");
  await other.getByRole("button", { name: "Reject" }).click();
  await expect(other).toHaveCount(0);
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  await maker.goto(`/publish/${two.slug}/releases`);
  const rejected = maker.getByRole("listitem").filter({ hasText: "0.1.0" });
  await expect(rejected.getByText("REJECTED", { exact: true })).toBeVisible();
  await expect(rejected.getByText("The zip has no program in it.").first()).toBeVisible();
});

test("changes: a repository move turned down, then approved", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-q3-${t}`, title: `Canal ${t}` };
  await newApp(app, MAKER);
  await approveRelease(await newRelease(app.slug, "1.0.0", MAKER));
  // Players may have it, so moving it waits for a person.
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

test("changes: holding players at an earlier release, approved", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-q8-${t}`, title: `Ferry ${t}` };
  await newApp(app, MAKER);
  const first = await newRelease(app.slug, "1.0.0", MAKER);
  await approveRelease(first);
  // With automatic updates off, holding players at a release waits for a person.
  await ok(MAKER, "PUT", `/v1/apps/${app.slug}/update-settings`, { auto_update: false });
  await approveRelease(await newRelease(app.slug, "1.1.0", MAKER));
  await ok(MAKER, "POST", `/v1/apps/${app.slug}/changes`, { kind: "release", release_id: first, note: "1.1.0 breaks old saves." });

  await signIn(page, "admin");
  await openItem(page, "changes", app.title);
  const change = item(page, app.title, "Turn down");
  await expect(change.getByText("SWITCH RELEASE")).toBeVisible();
  await expect(change.getByText("v1.1.0", { exact: true })).toBeVisible();
  await expect(change.getByText("v1.0.0 (held there)")).toBeVisible();
  await expect(change.getByText("1.1.0 breaks old saves.")).toBeVisible();
  await change.getByRole("button", { name: "Approve" }).click();
  await expect(change).toHaveCount(0);
  const manage = await ok(MAKER, "GET", `/v1/apps/${app.slug}/manage`);
  expect(manage.changes.find((c: { kind: string }) => c.kind === "release").status).toBe("approved");
});

test("pictures: looked at, turned down, approved, and the app put in the store", async ({ browser }) => {
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

  // The other can't go in the store while its pictures wait.
  await openItem(admin, "listing", app.title);
  const ready = admin
    .getByRole("listitem")
    .filter({ has: admin.getByRole("link", { name: app.title, exact: true }) })
    .filter({ hasText: "Coming soon · by Sec Security" });
  await expect(ready.getByText("Review its pictures first")).toBeVisible();
  await expect(ready.getByRole("button", { name: "Put in the store" })).toHaveCount(0);
  await openItem(admin, "pictures", app.title);
  const waiting = item(admin, app.title, "Turn down");
  await waiting.getByRole("button", { name: "Approve pictures" }).click();
  await expect(waiting).toHaveCount(0);
  await openItem(admin, "listing", app.title);
  await ready.getByRole("button", { name: "Put in the store" }).click();
  await expect(ready).toHaveCount(0);
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
  await expect(panel.getByText(/Waiting for a reviewer\./)).toBeVisible();
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
      if (path.includes("rescan") && JSON.parse(body ?? "{}").reason.length < 10) return route.fallback();
      sent.push({ url: route.request().url(), body: body ? JSON.parse(body) : null });
      await route.fulfill({ status: 204 });
    });
  }
  // The re-scan flag: keeping a release takes a real reason (this one reaches the server).
  await openItem(page, "rescans", "Pocket Golf");
  const rescan = page.getByRole("listitem").filter({ hasText: "FILE CHANGED" });
  await expect(rescan.getByRole("link", { name: "Pocket Golf" })).toBeVisible();
  await expect(rescan.getByText("1.0.0 · windows")).toBeVisible();
  await rescan.getByRole("button", { name: "Keep it: false positive" }).click();
  const keep = rescan.getByRole("textbox", { name: "Keep it: false positive: reason" });
  await keep.fill("It's fine");
  await rescan.getByRole("button", { name: "Keep it: false positive" }).click();
  await expect(page.getByRole("alert")).toHaveText("say why in 10 to 1000 characters");
  await keep.fill("The maker re-signed the same build.");
  await rescan.getByRole("button", { name: "Keep it: false positive" }).click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].url).toMatch(/\/files\/windows\/rescan-clear$/);
  expect(sent[0].body).toEqual({ reason: "The maker re-signed the same build." });
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
