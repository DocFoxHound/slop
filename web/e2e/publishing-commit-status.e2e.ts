// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing: Results on GitHub on the Manage page (crates/slop-api/src/commit_status.rs). The
// test server has no GitHub App and the server switch is off (examples/e2e_server/main.rs), so
// the panel is hidden for real. Every other state is the real Manage answer with `commit_status`
// added, and the switch's answers are given here; the API tests
// (crates/slop-api/tests/api/publishing_tests.rs) cover the real way to GitHub.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page, Route } from "@playwright/test";

import type { AppChange, CommitStatus, ManageOverview } from "../src/api";
import { expect, signIn, test } from "./fixtures";
import { answer, apiError, call, createApp, fakeRepo, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

// Mia's app for this file. Its repository is on the fake host: nothing here reaches GitHub.
const slug = uniq("pub-status");
const title = `Status Lights ${slug.slice(-4)}`;
const repo = fakeRepo("mia", slug);
const manage = `/publish/${slug}/manage`;
const GITHUB = "https://github.com/apps/slaporslop/installations/new";

const off: CommitStatus = {
  on: false,
  since: null,
  installed: true,
  permission: null,
  github_url: GITHUB,
  in_store: true,
  problem: null,
  last: null,
};
const on: CommitStatus = { ...off, on: true, since: "2026-11-12T09:30:00Z", permission: true };

function problem(code: number): CommitStatus {
  return { ...on, problem: { code, at: "2026-11-12T10:05:00Z" } };
}

let overview: ManageOverview;

test.beforeAll(async () => {
  await createApp("maker", slug, title, repo);
  overview = await call<ManageOverview>("maker", "GET", `/v1/apps/${slug}/manage`);
});

test.afterAll(async () => {
  await readNotices("maker", slug);
});

/** Opens Manage as Mia with `status` as the answer's `commit_status`, and returns the panel. */
async function open(page: Page, status: CommitStatus) {
  await page.unroute(`**/v1/apps/${slug}/manage`);
  await answer(page, `/v1/apps/${slug}/manage`, { ...overview, commit_status: status }, 200, "GET");
  await page.goto(manage);
  const panel = page.getByRole("region", { name: "Results on GitHub" });
  await expect(panel).toBeVisible();
  return panel;
}

/** Answers the switch with `reply(on)`, after `delay` ms, and keeps every body the page sent. */
async function fakeSwitch(page: Page, reply: (on: boolean) => { status: number; body: unknown }, delay = 0) {
  const sent: unknown[] = [];
  await page.unroute(`**/v1/apps/${slug}/commit-status`);
  await page.route(`**/v1/apps/${slug}/commit-status`, async (route: Route) => {
    if (route.request().method() !== "PUT") return route.fallback();
    const body = route.request().postDataJSON() as { on: boolean };
    sent.push(body);
    const r = reply(body.on);
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
    return route.fulfill({ status: r.status, contentType: "application/json", body: JSON.stringify(r.body) });
  });
  return sent;
}

const checkbox = (panel: Locator) =>
  panel.getByRole("checkbox", { name: /Show SLAPorSLOP's result on each commit/ });

test("nothing shows while the server doesn't offer it, and the App's sentence stays true", async ({ page }) => {
  // The test server is like a server without SLOP_GITHUB_STATUSES: no key, and the route isn't there.
  expect(overview).not.toHaveProperty("commit_status");
  await expect(call("maker", "PUT", `/v1/apps/${slug}/commit-status`, { on: true })).rejects.toThrow(/answered 404/);

  // The Updates box offers the App as before; what it says the App does is true with or without
  // results on GitHub.
  const settings = await call<Record<string, unknown>>("maker", "GET", `/v1/apps/${slug}/update-settings`);
  await answer(
    page,
    `/v1/apps/${slug}/update-settings`,
    { ...settings, watch_releases: true, github_app: { installed: false, install_url: GITHUB } },
    200,
    "GET",
  );
  await signIn(page, "maker");
  await page.goto(manage);
  const updates = page.locator("section").filter({ has: page.getByRole("heading", { name: "Updates", exact: true }) });
  await expect(updates.getByText(/It reads your releases and code, and never changes them\./)).toBeVisible();
  await expect(updates.getByText(/only reads/)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Repository", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Results on GitHub" })).toHaveCount(0);
});

test("switching it on and off", async ({ page }) => {
  await signIn(page, "maker");
  const panel = await open(page, off);
  const box = checkbox(panel);
  await expect(box).not.toBeChecked();
  await expect(box).toBeEnabled();
  // Before switching it on, the maker reads that anyone who sees the commit sees the result.
  await expect(panel.getByText(/Anyone who can see the commit on GitHub sees it, also when a release isn't approved/)).toBeVisible();
  await expect(panel.getByText(/It never says why\./)).toBeVisible();
  // The live region is there, empty, before anything happens in it.
  await expect(panel.getByRole("status")).toHaveText("");

  const sent = await fakeSwitch(page, (wanted) => ({ status: 200, body: wanted ? on : off }));
  await box.click();
  await expect(box).toBeChecked();
  expect(sent).toEqual([{ on: true }]);
  await expect(panel.getByRole("status")).toHaveText("Nothing posted yet: the next release you send gets a status.");

  await box.click();
  await expect(box).not.toBeChecked();
  expect(sent).toEqual([{ on: true }, { on: false }]);
  await expect(panel.getByRole("status")).toHaveText("");
});

test("a permission not accepted on GitHub: what to accept and where, then Check again", async ({ page }) => {
  await signIn(page, "maker");
  const panel = await open(page, { ...on, permission: false });
  const notice = panel.locator(".notice");
  await expect(notice).toContainText("The App asks for one more permission, Commit statuses: Read and write.");
  await expect(notice).toContainText("choose Configure next to SLAPorSLOP's App, then Review request. Then press Check again.");
  await expect(panel.getByRole("link", { name: "Open the App on GitHub" })).toHaveAttribute("href", GITHUB);
  await expect(checkbox(panel)).toBeChecked();

  const results = await new AxeBuilder({ page })
    .include('section[aria-labelledby="commit-status-title"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);

  // Accepted on GitHub: Check again (from the keyboard) asks once more, the notice and the button
  // go, and focus moves to the switch instead of being lost.
  const sent = await fakeSwitch(page, () => ({ status: 200, body: on }));
  await panel.getByRole("button", { name: "Check again" }).focus();
  await page.keyboard.press("Enter");
  await expect(notice).toHaveCount(0);
  expect(sent).toEqual([{ on: true }]);
  await expect(panel.getByRole("button", { name: "Check again" })).toHaveCount(0);
  await expect(checkbox(panel)).toBeFocused();
  await expect(panel.getByRole("status")).toHaveText("Nothing posted yet: the next release you send gets a status.");
});

test("each problem says what happened and what to do", async ({ page }) => {
  await signIn(page, "maker");
  const cases: Array<[CommitStatus, RegExp, string | null]> = [
    [{ ...on, installed: false }, /^SLAPorSLOP's GitHub App isn't installed on this repository any more, so nothing can be posted\./, "Install on GitHub"],
    [problem(403), /^GitHub refused the last status \(403\)\. Most often the App's Commit statuses permission hasn't been accepted/, "Open the App on GitHub"],
    [problem(404), /^GitHub couldn't find this repository for SLAPorSLOP's App \(404\)/, "Open the App on GitHub"],
    [problem(422), /^GitHub doesn't have that release's commit in this repository \(422\)/, null],
    [problem(301), /^This repository has moved on GitHub, so the status had nowhere to go\. Change the repository address under Repository/, null],
    [problem(410), /^GitHub refused the last status \(410\)\. Press Check again to try once more\.$/, null],
    [problem(502), /^SLAPorSLOP couldn't reach GitHub after several tries\. Press Check again to try once more\.$/, null],
  ];
  for (const [status, text, link] of cases) {
    const panel = await open(page, status);
    const notice = panel.locator(".notice");
    await expect(notice).toHaveText(text);
    if (link) await expect(notice.getByRole("link", { name: link })).toHaveAttribute("href", GITHUB);
    else await expect(notice.getByRole("link")).toHaveCount(0);
    // Every one of them can be tried again from here, and switched off.
    await expect(panel.getByRole("button", { name: "Check again" })).toBeVisible();
    await expect(checkbox(panel)).toBeEnabled();
  }
});

test("before the App is installed, outside the store, and the last status posted", async ({ page }) => {
  await signIn(page, "maker");
  // Never switched on and the App isn't there: the way to install it, and nothing to switch yet.
  let panel = await open(page, { ...off, installed: false });
  await expect(checkbox(panel)).toBeDisabled();
  await expect(panel.locator(".notice")).toContainText("To use this, install SLAPorSLOP's GitHub App on this repository first.");
  await expect(panel.getByRole("link", { name: "Install on GitHub" })).toHaveAttribute("href", GITHUB);
  await expect(panel.getByRole("button", { name: "Check again" })).toHaveCount(0);

  // Out of the store: nothing new is posted, and there's nothing to check again (a Private app
  // couldn't), so an old problem waits until the app is back.
  panel = await open(page, { ...problem(403), in_store: false });
  await expect(panel.getByRole("status")).toHaveText(
    "Nothing new is posted while the app isn't in the store. When it is, the newest result is posted.",
  );
  await expect(panel.getByRole("button", { name: "Check again" })).toHaveCount(0);
  await expect(checkbox(panel)).toBeEnabled();

  const last = {
    commit_sha: "a".repeat(40),
    version: "1.2.0",
    state: "success" as const,
    description: "Passed SLAPorSLOP's checks",
    posted_at: "2026-11-12T10:02:11Z",
  };
  panel = await open(page, { ...on, last });
  await expect(panel.getByRole("status")).toHaveText(/^Last posted .+ for 1\.2\.0: Passed SLAPorSLOP's checks\.$/);
  // The release behind it is gone: no version.
  panel = await open(page, { ...on, last: { ...last, version: null, state: "failure", description: "Not approved by SLAPorSLOP" } });
  await expect(panel.getByRole("status")).toHaveText(/^Last posted .+: Not approved by SLAPorSLOP\.$/);
  await expect(panel.getByRole("status")).not.toContainText(" for ");
  await expect(panel.locator(".notice")).toHaveCount(0);
});

test("errors: a refusal shows its reason, and a 404 takes the panel away", async ({ page }) => {
  await signIn(page, "maker");
  const panel = await open(page, off);
  const box = checkbox(panel);
  const message = "SLAPorSLOP's GitHub App isn't installed on this repository: install it on GitHub, then try again";
  await fakeSwitch(page, () => ({ status: 409, body: apiError("conflict", message) }));
  await box.click();
  await expect(panel.getByRole("alert")).toHaveText(message);
  await expect(box).not.toBeChecked();
  await expect(box).toBeEnabled();

  // Not offered for this app any more (for example it was made Private meanwhile): the page reads
  // the real answer again, which has no `commit_status`, and the panel goes without an error.
  const sent = await fakeSwitch(page, () => ({ status: 404, body: apiError("not_found", "not found") }));
  await page.unroute(`**/v1/apps/${slug}/manage`);
  await box.click();
  await expect(page.getByRole("heading", { name: "Results on GitHub" })).toHaveCount(0);
  expect(sent).toEqual([{ on: true }]);
  await expect(page.getByRole("heading", { name: "Repository", exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  // Focus went to the page's content, not lost with the panel.
  await expect(page.locator("#main")).toBeFocused();
});

test("switched off once the App is gone, focus moves from the switch to the install link", async ({ page }) => {
  await signIn(page, "maker");
  const panel = await open(page, { ...on, installed: false });
  const box = checkbox(panel);
  const sent = await fakeSwitch(page, () => ({ status: 200, body: { ...off, installed: false } }));
  await box.focus();
  await page.keyboard.press("Space");
  await expect(box).not.toBeChecked();
  await expect(box).toBeDisabled();
  expect(sent).toEqual([{ on: false }]);
  await expect(panel.getByRole("link", { name: "Install on GitHub" })).toBeFocused();
  await expect(panel.getByRole("status")).toContainText("To use this, install SLAPorSLOP's GitHub App on this repository first.");
});

test("a change elsewhere on Manage brings each side panel back once", async ({ page }) => {
  await signIn(page, "maker");
  // A waiting request to cancel: cancelling it is one of the changes that redraw the side panels.
  const change: AppChange = {
    id: "0190f000-0000-7000-8000-000000000001",
    kind: "repo",
    release_id: null,
    release_version: null,
    repo_url: fakeRepo("mia", `${slug}-moved`),
    note: null,
    status: "pending",
    status_reason: null,
    created_at: "2026-11-12T09:00:00Z",
    reviewed_at: null,
    automatic: false,
  };
  let changes = [change];
  // The Updates panel's answer changes with it, so the redrawn panel can be told from the old one.
  const settings = await call<Record<string, unknown>>("maker", "GET", `/v1/apps/${slug}/update-settings`);
  let autoUpdate = false;
  await page.unroute(`**/v1/apps/${slug}/manage`);
  await page.route(`**/v1/apps/${slug}/manage`, (route: Route) =>
    route.request().method() !== "GET"
      ? route.fallback()
      : route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ ...overview, changes, commit_status: on }) }),
  );
  await page.route(`**/v1/apps/${slug}/update-settings`, (route: Route) =>
    route.request().method() !== "GET"
      ? route.fallback()
      : route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ ...settings, auto_update: autoUpdate }) }),
  );
  await page.route(`**/v1/apps/${slug}/changes/${change.id}/cancel`, (route: Route) => {
    changes = [{ ...change, status: "cancelled" }];
    autoUpdate = true;
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(changes[0]) });
  });
  await page.goto(manage);
  const auto = page.getByRole("checkbox", { name: /Approve updates automatically/ });
  await expect(auto).not.toBeChecked();
  await expect(page.getByRole("heading", { name: "Results on GitHub" })).toBeVisible();

  await page.getByRole("button", { name: "Cancel request" }).click();
  await expect(page.getByText("CANCELLED")).toBeVisible();
  // One Updates panel, the redrawn one (a stale copy would make this match two checkboxes).
  await expect(auto).toBeChecked();
  await expect(page.getByRole("heading", { name: "Updates", exact: true })).toHaveCount(1);
  await expect(page.getByRole("heading", { name: "Results on GitHub" })).toHaveCount(1);
});

test("the keyboard: Space switches it, and focus stays on the checkbox", async ({ page }) => {
  await signIn(page, "maker");
  const panel = await open(page, off);
  const box = checkbox(panel);
  const sent = await fakeSwitch(page, (wanted) => ({ status: 200, body: wanted ? on : off }), 400);
  await box.focus();
  await page.keyboard.press("Space");
  // While GitHub's answer is awaited: busy, a second press sends nothing, and focus stays.
  await expect(panel).toHaveAttribute("aria-busy", "true");
  await expect(box).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Space");
  await expect(box).toBeFocused();
  await expect(box).toBeChecked();
  await expect(panel).toHaveAttribute("aria-busy", "false");
  await expect(box).toBeFocused();
  await page.keyboard.press("Space");
  await expect(box).not.toBeChecked();
  await expect(box).toBeFocused();
  expect(sent).toEqual([{ on: true }, { on: false }]);
});
