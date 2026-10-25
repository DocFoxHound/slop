// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing progress, file by file (web/src/components/PublishProgress.tsx): which file Slop is
// checking and at which step, which are done or waiting, an estimate, then the wait for a person,
// with the page refreshing itself until nothing is under way. This server checks files in a
// moment, so the states in between are answered in the browser. Also the "Donations: off" pointer
// on the Publish page.
import type { Route } from "@playwright/test";

import { expect, signIn, test } from "./fixtures";
import { answer, createApp, fakeRepo, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

const slug = uniq("pub-prog");
const title = `Progress Pond ${slug.slice(-4)}`;
const repo = fakeRepo("mia", slug);

const NOW = Date.parse("2026-10-25T12:00:00Z");
const ago = (minutes: number) => new Date(NOW - minutes * 60_000).toISOString();

test.beforeAll(async () => {
  await createApp("maker", slug, title, repo);
});

test.afterAll(async () => {
  await readNotices("maker", slug);
});

type Check = "waiting" | "checking" | "checked" | "not_checked";

function file(platform: string, file_name: string, check: Check, over: Record<string, unknown> = {}) {
  return {
    platform,
    kind: platform.startsWith("linux-") ? "installer" : "archive",
    url: `${repo}/releases/download/v1.0.0/${file_name}`,
    file_name,
    exec: platform.startsWith("linux-") ? null : "game",
    args: [],
    size_bytes: check === "checked" ? 1000 : null,
    sha256: check === "checked" ? "ab".repeat(32) : null,
    provenance: null,
    provenance_problem: null,
    maker_key: null,
    scan_result: check === "checked" ? "passed" : null,
    scan_detail: null,
    scan_report: null,
    defender: null,
    cleared_reason: null,
    check_stage: null,
    check_started_at: null,
    checked_at: null,
    check,
    ...over,
  };
}

function release(status: string, files: unknown[], over: Record<string, unknown> = {}) {
  return {
    id: "0192f0c0-0000-7000-8000-000000000001",
    version: "1.0.0",
    commit_sha: "c".repeat(40),
    status,
    commit_verified: true,
    created_at: ago(15),
    source: "slop_build",
    auto_approved: false,
    rating: null,
    runtimes: [],
    notes: null,
    files,
    source_checks: null,
    repo_scan: status === "fetching" ? null : { status: "checked", licence: { declared: "MIT", files: [], headers: [], problems: [] }, monetization: [] },
    advisories: [],
    ...over,
  };
}

const progress = (over: Record<string, unknown>) => ({
  stage: "checking_files",
  files_total: 4,
  files_checked: 1,
  started_at: ago(12),
  seconds_per_file: 300,
  remaining_seconds: null,
  summary: "",
  ...over,
});

const done = (platform: string, name: string, startedMinAgo: number) =>
  file(platform, name, "checked", { check_started_at: ago(startedMinAgo), checked_at: ago(startedMinAgo - 5) });

/** The four builds of a Slop Build release, as Slop checks them one at a time. */
const PHASES = [
  // File 2 is being scanned.
  release(
    "fetching",
    [
      done("linux", "game.tar.gz", 12),
      file("linux-deb", "game.deb", "checking", { check_stage: "scanning", check_started_at: ago(2) }),
      file("linux-rpm", "game.rpm", "waiting"),
      file("windows", "game.zip", "waiting"),
    ],
    { progress: progress({}) },
  ),
  // File 3 has started.
  release(
    "fetching",
    [
      done("linux", "game.tar.gz", 12),
      done("linux-deb", "game.deb", 7),
      file("linux-rpm", "game.rpm", "checking", { check_stage: "downloading", check_started_at: ago(0) }),
      file("windows", "game.zip", "waiting"),
    ],
    { progress: progress({ files_checked: 2 }) },
  ),
  // Every file is checked: a person reviews it.
  release(
    "awaiting_review",
    [
      done("linux", "game.tar.gz", 22),
      done("linux-deb", "game.deb", 17),
      done("linux-rpm", "game.rpm", 12),
      done("windows", "game.zip", 7),
    ],
    { status_reason: "Waiting for a person: an app's first release is always reviewed by a person" },
  ),
  // Approved.
  release("approved", [done("linux", "game.tar.gz", 22), done("windows", "game.zip", 7)]),
];

test("each file's check shows while Slop works through them, and the page refreshes itself until it's decided", async ({ page }) => {
  await page.clock.install({ time: NOW });
  await signIn(page, "maker");
  let phase = 0;
  let loads = 0;
  await page.route(`**/v1/apps/${slug}/releases`, (route: Route) => {
    if (route.request().method() !== "GET") return route.fallback();
    loads++;
    return route.fulfill({ json: [PHASES[phase]] });
  });

  // The Publish page says which file is being checked, and that donations are off.
  await page.goto("/publish");
  const row = page.locator("li.status-row").filter({ hasText: title });
  await expect(row).toContainText("Waiting on: Files checked");
  await expect(row).toContainText("1 of 4 done");
  await expect(row).toContainText("Checking file 2 of 4: Linux (.deb) — Scanning for malware");
  await expect(row).toContainText("About 5 minutes per file; started 12 minutes ago. About 13 minutes left.");
  await expect(row.getByRole("link", { name: `Set up donations for ${title}` })).toHaveAttribute("href", `/publish/${slug}/manage#donations`);
  await expect(row).toContainText("Donations: off · Set up");
  // An app that takes donations has no pointer.
  await expect(page.locator("li.status-row").filter({ hasText: "Lantern Keep" }).getByText("Donations: off")).toHaveCount(0);

  await page.goto(`/publish/${slug}/releases`);
  const box = page.getByRole("region", { name: "Publishing progress" });
  const filesStep = box.locator("li.step").filter({ hasText: "Files checked" });
  await expect(filesStep).toContainText("1 of 4 done");
  await expect(filesStep).toContainText("Checking file 2 of 4: Linux (.deb) — Scanning for malware");
  await expect(filesStep).toContainText("About 5 minutes per file; started 12 minutes ago. About 13 minutes left.");
  const files = filesStep.getByRole("list", { name: "Files checked: each file" }).getByRole("listitem");
  await expect(files).toHaveCount(4);
  await expect(files.nth(0)).toContainText("Linux (.tar.gz)");
  await expect(files.nth(0)).toContainText("Passed");
  await expect(files.nth(0)).toContainText("Took 5 minutes");
  await expect(files.nth(1)).toContainText("Linux (.deb)");
  await expect(files.nth(1)).toContainText("Scanning for malware");
  await expect(files.nth(1)).toContainText("Started 2 minutes ago");
  await expect(files.nth(2)).toContainText("Linux (.rpm) Waiting");
  await expect(files.nth(3)).toContainText("Windows (.zip) Waiting");
  await expect(box.locator("li.step").filter({ hasText: "Release" }).first()).toContainText("Version 1.0.0 received from Slop Build.");
  await expect(box.locator("li.step").filter({ hasText: "Review by a person" })).toContainText("Not yet");

  // Ten seconds on, the page looks again by itself: the next file.
  phase = 1;
  await page.clock.fastForward(10_500);
  await expect(filesStep).toContainText("Checking file 3 of 4: Linux (.rpm) — Downloading");
  await expect(filesStep).toContainText("2 of 4 done");
  await expect(files.nth(1)).toContainText("Passed");

  // All checked: it waits for a person, and says what happens next.
  phase = 2;
  await page.clock.fastForward(10_500);
  const review = box.locator("li.step").filter({ hasText: "Review by a person" });
  await expect(review).toContainText("Waiting for a person");
  await expect(review).toContainText("Waiting for review by a person at Slop. It usually takes a day or two.");
  await expect(review).toContainText("Why a person: an app's first release is always reviewed by a person.");
  await expect(review).toContainText("Either way you get a notification (under 🔔, and by email).");
  await expect(filesStep).toContainText("4 files downloaded and hashed.");
  // Files that all passed fold away, and open on request.
  await filesStep.getByText("Each file").click();
  await expect(filesStep.getByRole("list", { name: "Files checked: each file" }).getByRole("listitem")).toHaveCount(4);

  // While a person reviews it, once a minute.
  const waiting = loads;
  await page.clock.fastForward(30_000);
  expect(loads).toBe(waiting);
  phase = 3;
  await page.clock.fastForward(31_000);
  await expect.poll(() => loads).toBeGreaterThan(waiting);
  await expect(review).toContainText("Version 1.0.0 approved and signed by Slop.");
  // Decided: no more refreshing.
  const decided = loads;
  await page.clock.fastForward(180_000);
  expect(loads).toBe(decided);
});

test("a file on hold or a release not approved says why in plain words, and links to what to fix", async ({ page }) => {
  await signIn(page, "maker");
  // As a scanning server reports it.
  await answer(page, "/v1/server", { scanning: true, maker_signatures_required: false, simulation: false });
  let state = release(
    "awaiting_review",
    [
      done("linux", "game.tar.gz", 22),
      done("windows", "game.zip", 7),
    ].map((f) =>
      f.platform === "windows"
        ? {
            ...f,
            scan_result: "on_hold",
            scan_detail: "YARA: Packed_UPX.",
            defender: { sha256: "ab".repeat(32), engine: "1.1", signatures: "1.4", signatures_updated: null, detections: [], signed_by: null },
          }
        : f,
    ),
  );
  await page.route(`**/v1/apps/${slug}/releases`, (route: Route) =>
    route.request().method() === "GET" ? route.fulfill({ json: [state] }) : route.fallback(),
  );
  await page.goto(`/publish/${slug}/releases`);
  const box = page.getByRole("region", { name: "Publishing progress" });
  const filesStep = box.locator("li.step").filter({ hasText: "Files checked" });
  const windows = filesStep.getByRole("listitem").filter({ hasText: "Windows (.zip)" });
  await expect(windows).toContainText("On hold");
  await expect(windows).toContainText("YARA: Packed_UPX. · Microsoft Defender found nothing");
  const scan = box.locator("li.step").filter({ hasText: /^.Scan/ });
  await expect(scan).toContainText("Windows (.zip) is on hold: YARA: Packed_UPX.");
  await expect(scan).toContainText("If it's a false positive, they clear it and tell you why.");
  await expect(box.locator("li.step").filter({ hasText: "Review by a person" })).toContainText("looks at the file on hold first");

  // Not approved: why, and the fixes to copy for an AI agent.
  state = { ...state, status: "rejected", status_reason: "The game doesn't start on Windows." };
  await answer(page, `/v1/apps/${slug}/fixes`, {
    release: { id: state.id, version: "1.0.0", status: "rejected" },
    fixes: [{ kind: "release:rejected", severity: "must", problem: "The game doesn't start on Windows.", instruction: "Fix the start-up crash." }],
    next: "Then submit a new release.",
  });
  await page.reload();
  const review = box.locator("li.step").filter({ hasText: "Review by a person" });
  await expect(review).toContainText("Not approved: The game doesn't start on Windows. Fix it and submit a new version.");
  await expect(review).toContainText("or ask your AI agent to run get_fixes.");
  await review.getByRole("link", { name: "See what to fix" }).click();
  await expect(page).toHaveURL(new RegExp(`/publish/${slug}/releases#fixes$`));
  const fixes = page.getByRole("region", { name: "To fix in 1.0.0" });
  await expect(fixes).toBeInViewport();
  await expect(fixes.getByRole("button", { name: "Copy for your AI" })).toBeVisible();
});
