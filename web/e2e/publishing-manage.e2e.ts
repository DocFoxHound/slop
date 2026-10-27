// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 3: the Manage page. Which release players get (straight away, or checked by a
// person), moving the repository, withdrawing a release not reviewed yet, requests and cancelling
// them; and, read-only on the seeded Lantern Keep, ratings, how each release landed and bug triage.
import { expect, ids, signIn, test } from "./fixtures";
import { answer, approve, call, createApp, fakeRepo, readNotices, submitRelease, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

// Mia's app for this file: two approved releases, from the fake host.
const slug = uniq("pub-kite");
const title = `Kite Runner ${slug.slice(-4)}`;
const repo = fakeRepo("mia", slug);
const manage = `/publish/${slug}/manage`;

test.beforeAll(async () => {
  await createApp("maker", slug, title, repo, { bug_reports: false });
  await approve(await submitRelease("maker", slug, repo, "1.0.0"));
  await approve(await submitRelease("maker", slug, repo, "1.1.0"));
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
  await expect(page.getByRole("heading", { name: `Manage ${title}`, level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: "Edit store page" })).toHaveAttribute("href", `/publish/${slug}`);
  await expect(page.getByRole("link", { name: "Releases & progress" })).toHaveAttribute("href", `/publish/${slug}/releases`);
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "What players get" }) });
  await expect(section).toContainText("Players get v1.1.0 NEWEST APPROVED");
  const choice = section.getByRole("radiogroup", { name: "Release players get" });
  await expect(choice.getByRole("radio", { name: /Always the newest approved release/ })).toBeChecked();
  const send = section.getByRole("button", { name: /Switch|Send for review/ });
  await expect(send).toBeDisabled();

  // Every check passed already, so holding them at 1.0.0 happens at once.
  await choice.getByRole("radio", { name: /Hold players at v1\.0\.0/ }).check();
  await expect(section.getByText(/This release already passed every check, so it goes out straight away\./)).toBeVisible();
  await section.getByRole("textbox", { name: /Why/ }).fill("1.1.0 crashes on start");
  await section.getByRole("button", { name: "Switch" }).click();
  await expect(section.getByText(/^Done\./)).toBeVisible();
  await expect(section).toContainText("Players get v1.0.0 HELD AT THIS RELEASE");
  await expect(section.getByText("v1.1.0 is approved but isn't going out, because you're holding players at v1.0.0.")).toBeVisible();
  const request = page.locator("ul.release-list > li").filter({ hasText: "Hold players at v1.0.0" });
  await expect(request).toContainText("DONE AUTOMATICALLY");
  await expect(request).toContainText("Your note: 1.1.0 crashes on start");

  // Back to the newest.
  await choice.getByRole("radio", { name: /Always the newest approved release/ }).check();
  await section.getByRole("button", { name: "Switch" }).click();
  await expect(section).toContainText("Players get v1.1.0 NEWEST APPROVED");
  await expect(page.locator("ul.release-list > li").filter({ hasText: "Follow the newest approved release" })).toContainText("DONE");
});

test("with automatic approval off, a change waits for a person and can be cancelled", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  // The update settings are on Manage too.
  const auto = page.locator("section.updates-panel").getByRole("checkbox", { name: /Approve updates automatically/ });
  await expect(auto).toBeChecked();
  await auto.click();
  await expect(auto).not.toBeChecked();

  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "What players get" }) });
  await section.getByRole("radio", { name: /Hold players at v1\.0\.0/ }).check();
  await expect(section.getByText(/so a person checks this first: /)).toBeVisible();
  await section.getByRole("button", { name: "Send for review" }).click();
  await expect(section.getByText(/^Sent for review\./)).toBeVisible();
  await expect(section.getByText("Waiting for review: hold players at v1.0.0.")).toBeVisible();
  await expect(section).toContainText("Players get v1.1.0");
  const request = page.locator("ul.release-list > li").filter({ hasText: "Hold players at v1.0.0" }).filter({ hasText: "WAITING FOR REVIEW" });
  await request.getByRole("button", { name: "Cancel request" }).click();
  await expect(page.locator("ul.release-list > li").filter({ hasText: "CANCELLED" })).toContainText("Hold players at v1.0.0");
  await expect(section.getByText("Waiting for review: hold players at v1.0.0.")).toBeHidden();

  await auto.click();
  await expect(auto).toBeChecked();
});

test("moving the repository: players have it, so a person checks the move first", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(manage);
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "Repository", exact: true }) });
  await expect(section.getByRole("link", { name: repo })).toHaveAttribute("href", repo);
  await expect(section.getByText(/Players already have this app, so a move is always checked by a person/)).toBeVisible();
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
  await expect(section.getByText(/^Sent for review\./)).toBeVisible();
  await expect(section.getByText(`Waiting for review: move to ${moved}.`)).toBeVisible();
  await expect(section.getByRole("textbox", { name: "Move to" })).toHaveValue("");
  const row = page.locator("ul.release-list > li").filter({ hasText: `Move to ${moved}` });
  await expect(row).toContainText("WAITING FOR REVIEW");
  await expect(row).toContainText("Your note: Moved to the studio's account.");
  // Still the old repository until a person approves.
  await expect(page.getByRole("link", { name: "Edit store page" })).toBeVisible();
  await row.getByRole("button", { name: "Cancel request" }).click();
  await expect(row).toContainText("CANCELLED");
  await expect(section.getByText(/Waiting for review: move to/)).toBeHidden();
});

test("a release not reviewed yet can be withdrawn", async ({ page }) => {
  await submitRelease("maker", slug, repo, "1.2.0");
  await signIn(page, "maker");
  await page.goto(manage);
  const waiting = page.locator("section").filter({ has: page.getByRole("heading", { name: "Not reviewed yet" }) });
  const row = waiting.getByRole("listitem").filter({ hasText: "v1.2.0" });
  await expect(row).toContainText("Waiting for review, submitted from your account");
  // Asked first; saying no keeps it.
  page.once("dialog", (d) => {
    expect(d.message()).toBe("Withdraw v1.2.0? It won't be reviewed or reach players.");
    void d.dismiss();
  });
  await row.getByRole("button", { name: "Withdraw" }).click();
  await expect(row).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: "Withdraw" }).click();
  await expect(waiting).toBeHidden();
  await page.goto(`/publish/${slug}/releases`);
  await expect(page.locator("ul.release-list > li").filter({ hasText: "1.2.0" }).getByText("WITHDRAWN BEFORE REVIEW")).toBeVisible();
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
  await expect(pulse).toContainText("a reviewer can't list this one before its first Pulse handshake");
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
  await expect(landed.getByRole("listitem").filter({ hasText: "1.0.0" })).toContainText(/\d+% Slaps from \d+ votes · \d+ 👍 and \d+ 👎 reviews · \d+ bug reports?/);

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
