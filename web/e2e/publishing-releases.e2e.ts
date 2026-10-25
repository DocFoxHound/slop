// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 2: the Releases page. Submitting by hand (field errors, the rating, runtimes,
// notes), the history (badges, checks, notes, failures, withdrawal), picking a GitHub release, the
// update settings, fixes and Slop Build panels. Then "Check a file" finds the builds.
import { createHash } from "node:crypto";

import { expect, ids, signIn, test } from "./fixtures";
import { answer, apiError, approve, call, createApp, fakeRepo, fakeZip, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

// Mia's app for this file, on the fake host (its release files really download).
const slug = uniq("pub-tide");
const title = `Tide Pool ${slug.slice(-4)}`;
const repo = fakeRepo("mia", slug);
const page_ = `/publish/${slug}/releases`;

test.beforeAll(async () => {
  await createApp("maker", slug, title, repo, { min_age: 12, content_labels: ["violence"] });
});

test.afterAll(async () => {
  await readNotices("maker", slug);
});

test("only the maker sees an app's releases", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto(page_);
  await expect(visitor.getByText("to manage releases.")).toBeVisible();
  await expect(visitor.getByRole("link", { name: "Sign in" }).last()).toHaveAttribute("href", `/signin?return_to=${page_}`);
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(page_);
  await expect(bo.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
  // Not even a listed app that isn't theirs.
  await bo.goto(`/publish/${ids.listedApp}/releases`);
  await expect(bo.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});

test("submitting by hand: each mistake shows by its field", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(page_);
  await expect(page.getByRole("heading", { name: "Releases", level: 1 })).toBeVisible();
  await expect(page.getByText("No releases yet.")).toBeVisible();
  // Nothing to fix before there's a release.
  await expect(page.getByRole("heading", { name: /^To fix in/ })).toHaveCount(0);
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "New release" }) });
  const version = form.getByRole("textbox", { name: /^Version/ });
  const commit = form.getByRole("textbox", { name: /^Commit/ });
  const url = form.getByRole("textbox", { name: /^File URL/ });
  const exec = form.getByRole("textbox", { name: /^Program to start/ });
  // Not GitHub: files come from the repository's own site.
  await expect(form.getByText("On the same site as your repository.")).toBeVisible();

  await version.fill("1 0");
  await commit.fill("Z".repeat(40));
  await url.fill("https://example.com/tide.zip");
  await exec.fill("tide.exe");
  await form.getByRole("button", { name: "Submit" }).click();
  await expect(form.locator("label").filter({ hasText: "Version" }).locator(".field-error")).toHaveText(
    "use up to 40 letters, numbers, dots and dashes, like 1.2.0",
  );
  await version.fill("1.0.0");
  await form.getByRole("button", { name: "Submit" }).click();
  await expect(form.locator("label").filter({ hasText: "Commit" }).locator(".field-error")).toHaveText(
    "use the full 40-character commit SHA, in lowercase",
  );
  await commit.fill("c".repeat(40));
  await form.getByRole("button", { name: "Submit" }).click();
  await expect(form.getByRole("alert")).toHaveText("files must be on the same site as the repository");
  await url.fill(fakeZip(repo, "1.0.0"));
  // Spaces pass the browser's check but aren't a program.
  await exec.fill("   ");
  await form.getByRole("button", { name: "Submit" }).click();
  await expect(form.getByRole("alert")).toHaveText(
    "archives need the path of the program to start, inside the archive, like bin/game",
  );
  await expect(page.getByText("No releases yet.")).toBeVisible();
});

test("a release with its rating, runtimes and notes goes through, after the agreement", async ({ page }) => {
  await signIn(page, "maker");
  // The Publisher Agreement changed since Mia accepted it: the release waits for it.
  let asked = false;
  await page.route(`**/v1/apps/${slug}/releases`, (route) => {
    if (route.request().method() !== "POST" || asked) return route.fallback();
    asked = true;
    return route.fulfill({ status: 409, json: apiError("terms_required", "accept the current Publisher Agreement first", "publisher") });
  });
  await page.goto(page_);
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "New release" }) });
  await form.getByRole("textbox", { name: /^Version/ }).fill("1.0.0");
  await form.getByRole("textbox", { name: /^Commit/ }).fill("c".repeat(40));
  await form.getByRole("textbox", { name: /^File URL/ }).fill(fakeZip(repo, "1.0.0"));
  await form.getByRole("textbox", { name: /^Program to start/ }).fill("pong.exe");
  await form.getByRole("textbox", { name: /^Arguments/ }).fill("--windowed --fast");

  // The rating starts from the store page's; lowering it means a person looks first.
  const rating = form.locator("details.release-rating").filter({ hasText: "Rating for this release" });
  await rating.locator("summary").click();
  await expect(rating.locator("summary")).toContainText("12+ · Violence");
  await rating.getByLabel("Age rating").selectOption("7");
  await expect(rating.getByText(/Lower than the store page now: a person reviews this release/)).toBeVisible();
  await rating.getByLabel("Age rating").selectOption("16");
  await expect(rating.getByText("When this release is approved, the store page shows its rating.")).toBeVisible();

  const runtimes = form.locator("details.release-rating").filter({ hasText: "Shared runtimes" });
  await expect(runtimes.locator("summary")).toContainText("Shared runtimes (Windows): none");
  await runtimes.locator("summary").click();
  await runtimes.getByRole("checkbox", { name: ".NET Desktop Runtime 8" }).check();
  await expect(runtimes.locator("summary")).toContainText(".NET Desktop Runtime 8");

  await form.getByLabel("What's new, for players (optional)").fill("**Tides** now turn twice a day.\n\n- Crabs\n- Shells");
  await form.getByRole("button", { name: "Submit" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("checkbox", { name: /I've read and accept/ }).check();
  await dialog.getByRole("button", { name: "Accept and continue" }).click();
  await expect(form.getByRole("status")).toHaveText(/Submitted\. Slop is checking the files/);
  // The form starts over.
  await expect(form.getByRole("textbox", { name: /^Version/ })).toHaveValue("");

  const release = page.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
  await expect(release.getByText("IN REVIEW", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(release).toContainText("16+ · Violence");
  await expect(release.getByText("Waiting for a person at Slop to review it.")).toBeVisible();
  await expect(release.locator(".release-notes strong")).toHaveText("Tides");
  await expect(release.locator(".release-notes li")).toHaveText(["Crabs", "Shells"]);
  await expect(release).toContainText("starts pong.exe");
  await expect(release).toContainText(/SHA-256 [0-9a-f]{16}…/);
  // This server doesn't scan files, and its fake host has no source to read.
  await expect(release.getByText("NOT SCANNED", { exact: true })).toBeVisible();
  await expect(release.getByText("SOURCE NOT CHECKED")).toBeVisible();
  const saved = await call<Array<{ version: string; runtimes: string[]; files: Array<{ args: string[] }> }>>(
    "maker",
    "GET",
    `/v1/apps/${slug}/releases`,
  );
  expect(saved[0]).toMatchObject({ version: "1.0.0", runtimes: ["dotnet-desktop-8"], files: [{ args: ["--windowed", "--fast"] }] });
});

test("a file Slop can't download fails, with the reason", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(page_);
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "New release" }) });
  await form.getByRole("textbox", { name: /^Version/ }).fill("1.0.1-missing");
  await form.getByRole("textbox", { name: /^Commit/ }).fill("d".repeat(40));
  // On the right site, but not there.
  await form.getByRole("textbox", { name: /^File URL/ }).fill(`${repo}/files/tide.zip`);
  await form.getByRole("textbox", { name: /^Program to start/ }).fill("pong.exe");
  await form.getByRole("button", { name: "Submit" }).click();
  const release = page.locator("ul.release-list > li").filter({ hasText: "1.0.1-missing" });
  await expect(release.getByText("FAILED", { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(release.locator(".notice")).toContainText("download failed: HTTP 404");
  await expect(release.getByText("Slop couldn't check the files. See the reason below.")).toBeVisible();
});

test("while files are being fetched, the page checks again every 10 seconds", async ({ page }) => {
  await page.clock.install();
  await signIn(page, "maker");
  let loads = 0;
  await page.route(`**/v1/apps/${slug}/releases`, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    loads++;
    const res = await route.fetch();
    const releases = (await res.json()) as Array<{ status: string }>;
    releases[0].status = "fetching";
    await route.fulfill({ response: res, json: releases });
  });
  await page.goto(page_);
  await expect(page.getByText("CHECKING FILES", { exact: true })).toBeVisible();
  const before = loads;
  await page.clock.fastForward(10_500);
  await expect.poll(() => loads).toBeGreaterThan(before);
});

test("the history shows what Slop's checks found", async ({ page }) => {
  await signIn(page, "maker");
  // This server doesn't scan files or read source: a release as a scanning server reports it.
  await page.route(`**/v1/apps/${slug}/releases`, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const res = await route.fetch();
    const releases = (await res.json()) as Array<Record<string, unknown> & { files: Array<Record<string, unknown>> }>;
    const r = releases.find((x) => x.version === "1.0.0")!;
    r.files[0].scan_result = "note";
    r.files[0].scan_detail = "A packer was used; a person checked it.";
    r.source_checks = {
      secrets: [{ file: "src/net.rs", line: 12, rule: "aws-access-token" }],
      vulnerabilities: [],
      tools: { gitleaks: "8.21" },
      checked: true,
    };
    r.repo_scan = {
      status: "checked",
      licence: { declared: "MIT", files: [{ path: "LICENSE", spdx: "MIT" }], headers: [], problems: [] },
      monetization: [{ rule: "iap", label: "In-app purchase API", count: 2, examples: ["src/shop.rs:4"] }],
      code_loading: [],
      network: [],
      save_folder: "slop",
    };
    r.advisories = [
      {
        vuln_id: "GHSA-tide-0001",
        ecosystem: "crates.io",
        package: "tidal",
        version: "0.3.1",
        summary: "Tidal reads past the end of a buffer",
        fixed: "0.3.2",
        malicious: false,
        known_at_build: false,
        found_at: "2026-10-20T10:00:00Z",
      },
    ];
    await route.fulfill({ response: res, json: releases });
  });
  await page.goto(page_);
  const release = page.locator("ul.release-list > li").filter({ hasText: "16+ · Violence" });
  await expect(release.getByText("SCAN PASSED WITH NOTES")).toBeVisible();
  await expect(release.getByText("A packer was used; a person checked it.")).toBeVisible();
  await expect(release.getByText("POSSIBLE SECRETS IN THE SOURCE (1)")).toBeVisible();
  await expect(release.getByText(/src\/net\.rs:12 \(aws-access-token\)/)).toBeVisible();
  await expect(release.getByText("NO KNOWN-VULNERABLE DEPENDENCIES")).toBeVisible();
  await expect(release.getByText("LICENCE FILE MATCHES")).toBeVisible();
  await expect(release.getByText("POSSIBLE PURCHASES OR LOOT BOXES (1)")).toBeVisible();
  await expect(release.getByText("SAVES IN SLOP'S SAVE FOLDER")).toBeVisible();
  await release.getByText("NEW ADVISORIES SINCE THIS BUILD (1)").click();
  await expect(release.getByRole("listitem").filter({ hasText: "GHSA-tide-0001" })).toContainText(
    "tidal 0.3.1 (crates.io): GHSA-tide-0001 Tidal reads past the end of a buffer Fixed in 0.3.2.",
  );

  // Seeded: Bo's releases were scanned, and one waits for review.
  await page.context().clearCookies();
  await signIn(page, "maker2");
  await page.goto(`/publish/${ids.releaseInReviewApp}/releases`);
  const waiting = page.locator("ul.release-list > li").filter({ hasText: "1.1.0" });
  await expect(waiting.getByText("IN REVIEW", { exact: true })).toBeVisible();
  await expect(waiting.getByText("SCAN PASSED", { exact: true })).toBeVisible();
  await expect(page.locator("ul.release-list > li").filter({ hasText: "1.0.0" }).getByText("APPROVED", { exact: true })).toBeVisible();
});

test("update settings: pick up GitHub releases, approve updates automatically", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(page_);
  const panel = page.locator("section.updates-panel");
  const watch = panel.getByRole("checkbox", { name: /Pick up new GitHub releases/ });
  const auto = panel.getByRole("checkbox", { name: /Approve updates automatically/ });
  await expect(watch).toBeChecked();
  await expect(panel.getByText("Only GitHub repositories can be watched so far.")).toBeVisible();
  await expect(auto).toBeChecked();
  await expect(panel.getByText(/Your next release waits for a person: an app's first release is always reviewed by a person\./)).toBeVisible();
  // Each switch saves as it's clicked.
  await watch.click();
  await expect(watch).not.toBeChecked();
  await expect(panel.getByText("Only GitHub repositories can be watched so far.")).toBeHidden();
  await auto.click();
  await expect(auto).not.toBeChecked();
  await expect(panel.getByText(/Your next release waits for a person/)).toBeHidden();
  await page.reload();
  await expect(watch).not.toBeChecked();
  await expect(auto).not.toBeChecked();
  await watch.click();
  await expect(watch).toBeChecked();
  await auto.click();
  await expect(auto).toBeChecked();
  expect(await call("maker", "GET", `/v1/apps/${slug}/update-settings`)).toMatchObject({ watch_releases: true, auto_update: true });

  // Refused: the reason shows, and nothing changes.
  await answer(
    page,
    `/v1/apps/${slug}/update-settings`,
    apiError("unavailable", "Slop is busy: try again in a minute"),
    503,
    "PUT",
  );
  await auto.click();
  await expect(panel.locator(".form-error")).toHaveText("Slop is busy: try again in a minute");
  await expect(auto).toBeChecked();
  await page.unroute(`**/v1/apps/${slug}/update-settings`);

  // A watched GitHub repository: when Slop last looked, and the faster GitHub App.
  const settings = await call<Record<string, unknown>>("maker", "GET", `/v1/apps/${slug}/update-settings`);
  await answer(
    page,
    `/v1/apps/${slug}/update-settings`,
    {
      ...settings,
      watch_note: null,
      watch_checked_at: "2026-10-22T09:30:00Z",
      watch_last_tag: "v2.0.0",
      watch_problem: "v2.0.1 has no file for Windows",
      github_app: { installed: false, install_url: "https://github.com/apps/slop/installations/new" },
    },
    200,
    "GET",
  );
  await page.reload();
  await expect(panel.getByText(/Last checked .* · newest seen: v2\.0\.0\./)).toBeVisible();
  await expect(panel.getByText("v2.0.1 has no file for Windows")).toBeVisible();
  await expect(panel.getByRole("link", { name: "Install on GitHub" })).toHaveAttribute(
    "href",
    "https://github.com/apps/slop/installations/new",
  );
  await page.unroute(`**/v1/apps/${slug}/update-settings`);
  await answer(page, `/v1/apps/${slug}/update-settings`, { ...settings, github_app: { installed: true, install_url: "" } }, 200, "GET");
  await page.reload();
  await expect(panel.getByText("GITHUB APP INSTALLED")).toBeVisible();
});

test("fixes from the checks, ready to copy for an AI agent", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await signIn(page, "maker");
  // The newest release failed: that's the fix.
  await page.goto(page_);
  const panel = page.getByRole("region", { name: "To fix in 1.0.1-missing" });
  await expect(panel.locator("li")).toHaveCount(1);
  await expect(panel.locator("li")).toContainText("MUST FIX");
  await expect(panel.locator("li")).toContainText("Release 1.0.1-missing was failed: windows build: download failed: HTTP 404");
  await panel.getByRole("button", { name: "Copy for your AI" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    "Slop's checks of my app found this. Release 1.0.1-missing was failed: windows build: download failed: HTTP 404\n" +
      "Fix it in the repository: Read the reason, fix what it names in the repository (or the release's files), then submit a new release with a new version.",
  );
  await panel.getByRole("button", { name: "Copy all for your AI" }).click();
  const all = await page.evaluate(() => navigator.clipboard.readText());
  expect(all).toContain("Release 1.0.1-missing was failed");
  expect(all).toMatch(/Slop's checks run again on the new release\.$/);

  // What the source checks find on a scanning server: a "should", with its guide section.
  await answer(page, `/v1/apps/${slug}/fixes`, {
    release: { id: "r1", version: "1.0.2", status: "awaiting_review" },
    fixes: [
      { kind: "secret", severity: "must", problem: "A key is in src/net.rs.", instruction: "Revoke it and remove it." },
      {
        kind: "saves",
        severity: "should",
        problem: "Saves go to Documents.",
        instruction: "Write saves to SLOP_SAVE_DIR.",
        guide_section: "Saves and sandbox mode",
      },
    ],
    next: "Then submit a new release.",
  });
  await page.reload();
  const mocked = page.getByRole("region", { name: "To fix in 1.0.2" });
  await expect(mocked.locator("li")).toHaveCount(2);
  await expect(mocked.locator("li").nth(1)).toContainText("SHOULD FIX");
  await mocked.locator("li").nth(1).getByRole("button", { name: "Copy for your AI" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    "Slop's checks of my app found this. Saves go to Documents.\nFix it in the repository: Write saves to SLOP_SAVE_DIR.\n" +
      '(Slop\'s publishing guide, section "Saves and sandbox mode", has the details.)',
  );
});

test("Slop Build: hidden where it isn't set up, a ready-made file where it is", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto(page_);
  await expect(page.locator("section.updates-panel")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Let Slop Build do it" })).toHaveCount(0);
  const setup = {
    supported: true,
    project_type: "rust",
    detected: "Cargo.toml",
    note: null,
    file_path: ".github/workflows/slop-build.yml",
    workflow: "name: Slop Build\non: push",
    create_url: "https://github.com/e2e/tide/new/main?filename=.github/workflows/slop-build.yml",
    check_file_path: ".github/workflows/slop-check.yml",
    check_workflow: "name: Slop Check",
    check_create_url: "https://github.com/e2e/tide/new/main?filename=.github/workflows/slop-check.yml",
    build_now_url: "https://github.com/e2e/tide/actions/workflows/slop-build.yml",
  };
  await answer(page, `/v1/apps/${slug}/slop-build`, setup);
  await page.reload();
  const section = page.locator("section.slop-build");
  await expect(section.getByRole("heading", { name: "Let Slop Build do it" })).toBeVisible();
  await expect(section.getByText("Detected a Rust project.")).toBeVisible();
  await expect(section.getByText(".github/workflows/slop-build.yml")).toBeVisible();
  await expect(section.getByRole("link", { name: "Add to my repository on GitHub" })).toHaveAttribute("href", setup.create_url);
  await expect(section.getByRole("link", { name: "Build now" })).toHaveAttribute("href", setup.build_now_url);
  await section.getByText("Building somewhere else?").click();
  await expect(section.getByRole("link", { name: "Add Slop Check to my repository" })).toHaveAttribute("href", setup.check_create_url);

  // A project it can't tell: a note instead of the guess.
  await page.unroute(`**/v1/apps/${slug}/slop-build`);
  await answer(page, `/v1/apps/${slug}/slop-build`, { ...setup, supported: false, project_type: "custom", note: "Slop couldn't tell how to build this; fill in the build command." });
  await page.reload();
  await expect(section.getByText("Slop couldn't tell how to build this; fill in the build command.")).toBeVisible();
  await expect(section.getByText(/Detected a/)).toHaveCount(0);
});

test("an app with no platforms is asked for them first", async ({ page }) => {
  await signIn(page, "maker");
  await page.route(`**/v1/apps/${slug}`, async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), platforms: [] } });
  });
  await page.goto(page_);
  await expect(page.getByText("First choose what your app runs on (Windows, Linux or both) on its store page.")).toBeVisible();
  await expect(page.getByRole("link", { name: "Edit store page" })).toHaveAttribute("href", `/publish/${slug}`);
  await expect(page.getByRole("heading", { name: "New release" })).toHaveCount(0);
});

test("a GitHub app picks a release from GitHub instead of typing it", async ({ page }) => {
  await signIn(page, "maker");
  // Starship Draft (seeded, Mia's) is on GitHub: Windows, Linux and a .deb. Nothing is sent here:
  // the fake GitHub has no releases, so the list and the sending are answered by the test.
  const draft = ids.draftApp;
  await page.goto(`/publish/${draft}/releases`);
  const picker = page.locator("form").filter({ has: page.getByRole("heading", { name: "Submit a release" }) });
  await expect(picker.getByText("Your repository has no releases yet.")).toBeVisible();
  await expect(picker).toContainText("Windows (a .zip of the app's files), Linux (a .tar.gz of the app's files), Linux .deb (a .deb package)");

  const gh = `https://github.com/e2e/${draft}`;
  const asset = (name: string) => ({ name, url: `${gh}/releases/download/v1.0.0/${name}`, size: 1000, platform: null, kind: null });
  const list = [
    { tag: "v1.1.0-rc1", name: "Release candidate", version: "1.1.0-rc1", published_at: null, prerelease: true, html_url: `${gh}/releases/tag/v1.1.0-rc1`, submitted: false, assets: [asset("starship-rc.zip")] },
    {
      tag: "v1.0.0",
      name: "First flight",
      version: "1.0.0",
      published_at: null,
      prerelease: false,
      html_url: `${gh}/releases/tag/v1.0.0`,
      submitted: false,
      assets: [asset("starship-windows.zip"), asset("starship-linux.tar.gz"), asset("starship.deb"), asset("NOTES.txt")],
    },
    { tag: "v0.9.0", name: null, version: "0.9.0", published_at: null, prerelease: false, html_url: `${gh}/releases/tag/v0.9.0`, submitted: true, assets: [] },
  ];
  let listed = 0;
  await page.route(`**/v1/apps/${draft}/github-releases`, (route) => {
    listed++;
    return route.fulfill({ json: list });
  });
  await picker.getByRole("button", { name: "look again" }).click();

  // The newest one not sent yet that isn't a pre-release.
  const choose = picker.getByRole("combobox", { name: "GitHub release" });
  await expect(choose).toHaveValue("v1.0.0");
  await expect(choose.locator("option", { hasText: "v0.9.0 (already sent)" })).toHaveAttribute("disabled", "");
  await expect(picker.locator("p", { hasText: "see it on GitHub" })).toContainText("Version 1.0.0");
  await expect(picker.getByRole("link", { name: "see it on GitHub" })).toHaveAttribute("href", `${gh}/releases/tag/v1.0.0`);
  const row = (name: string) => picker.locator("fieldset.pick-row").filter({ hasText: name });
  await expect(row("starship-windows.zip").getByRole("checkbox")).toBeChecked();
  await expect(row("starship-linux.tar.gz").getByRole("checkbox")).toBeChecked();
  await expect(row("starship.deb").getByRole("checkbox")).toBeChecked();
  await expect(row("NOTES.txt").getByRole("checkbox")).not.toBeChecked();
  await expect(row("starship-windows.zip").getByLabel("Build for")).toHaveValue("windows");
  await expect(row("starship.deb").getByLabel("Build for")).toHaveValue("linux-deb");
  const send = picker.getByRole("button", { name: "Send version 1.0.0" });
  await expect(send).toBeEnabled();

  // Two files for Linux, none for the .deb, and a .deb isn't a .tar.gz.
  await row("starship.deb").getByLabel("Build for").selectOption("linux");
  await expect(picker.getByText("Pick one file for Linux.")).toBeVisible();
  await expect(picker.getByText(/Still needed: Linux \.deb \(a \.deb package\)/)).toBeVisible();
  await expect(row("starship.deb").getByText("The Linux build must be a .tar.gz of the app's files.")).toBeVisible();
  await expect(send).toBeDisabled();
  await row("starship.deb").getByLabel("Build for").selectOption("linux-deb");
  await expect(send).toBeEnabled();
  await row("starship.deb").getByRole("checkbox").uncheck();
  await expect(picker.getByText(/Still needed: Linux \.deb/)).toBeVisible();
  await expect(send).toBeDisabled();
  await row("starship.deb").getByRole("checkbox").check();

  // A pre-release can be picked by hand.
  await choose.selectOption("v1.1.0-rc1");
  await expect(picker.getByRole("button", { name: "Send version 1.1.0-rc1" })).toBeDisabled();
  await expect(picker.getByText(/Still needed: Linux/)).toBeVisible();
  await choose.selectOption("v1.0.0");

  await row("starship-windows.zip").getByRole("textbox", { name: /Program to start/ }).fill("starship.exe");
  await row("starship-linux.tar.gz").getByRole("textbox", { name: /Program to start/ }).fill("bin/starship");
  await picker.getByLabel("What's new, for players (optional)").fill("First flight.");
  // Refused first, then sent.
  let sent: Record<string, unknown> | undefined;
  let tries = 0;
  await page.route(`**/v1/apps/${draft}/releases/from-github`, (route) => {
    tries++;
    if (tries === 1) return route.fulfill({ status: 400, json: apiError("invalid", "v1.0.0's commit isn't on the default branch", "tag") });
    sent = route.request().postDataJSON() as Record<string, unknown>;
    list[1].submitted = true;
    return route.fulfill({ status: 201, json: { id: "x", version: "1.0.0", status: "fetching" } });
  });
  await send.click();
  await expect(picker.getByRole("alert")).toHaveText("v1.0.0's commit isn't on the default branch");
  const before = listed;
  await send.click();
  await expect(picker.getByRole("status")).toHaveText(/Version 1\.0\.0 sent\./);
  expect(sent).toMatchObject({
    tag: "v1.0.0",
    notes: "First flight.",
    files: [
      { platform: "windows", kind: "archive", url: `${gh}/releases/download/v1.0.0/starship-windows.zip`, exec: "starship.exe" },
      { platform: "linux", kind: "archive", url: `${gh}/releases/download/v1.0.0/starship-linux.tar.gz`, exec: "bin/starship" },
      { platform: "linux-deb", kind: "installer", url: `${gh}/releases/download/v1.0.0/starship.deb` },
    ],
  });
  // The list is read again: v1.0.0 is sent now, so the page moves on to the one left, and the
  // notice stays (it used to vanish as the page picked the next release).
  await expect.poll(() => listed).toBeGreaterThan(before);
  await expect(choose).toHaveValue("v1.1.0-rc1");
  await expect(choose.locator("option", { hasText: "v1.0.0 — First flight (already sent)" })).toHaveAttribute("disabled", "");
  await expect(picker.getByRole("status")).toHaveText(/Version 1\.0\.0 sent\. Slop is checking the files/);

  // GitHub apps can still be entered by hand: files must be on the repository's releases page.
  await page.getByText("Enter a release by hand").click();
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "New release" }) });
  await expect(form.getByText("A file on this repository's GitHub releases page.").first()).toBeVisible();
  await form.getByRole("textbox", { name: /^Version/ }).fill("1.0.0");
  await form.getByRole("textbox", { name: /^Commit/ }).fill("e".repeat(40));
  for (const url of await form.getByRole("textbox", { name: /^File URL/ }).all()) await url.fill("https://example.com/starship.zip");
  for (const exec of await form.getByRole("textbox", { name: /^Program to start/ }).all()) await exec.fill("starship");
  await form.getByRole("button", { name: "Submit" }).click();
  await expect(form.getByRole("alert")).toHaveText("files must be on this repository's GitHub releases page");
});

test("staff approve the release, then withdraw it; Check a file says which", async ({ page }) => {
  const releases = await call<Array<{ id: string; version: string; files: Array<{ sha256: string }> }>>("maker", "GET", `/v1/apps/${slug}/releases`);
  const release = releases.find((r) => r.version === "1.0.0")!;
  await approve(release.id);
  await signIn(page, "maker");
  await page.goto(page_);
  const row = page.locator("ul.release-list > li").filter({ hasText: "16+ · Violence" });
  await expect(row.getByText("APPROVED", { exact: true })).toBeVisible();
  await expect(row.getByText("Signed by Slop. Players can install it, and get it as an update.")).toBeVisible();

  // Anyone can check a copy they got elsewhere: the fake GitHub's build is the one Slop approved.
  const zip = Buffer.from(await (await fetch(fakeZip(repo, "1.0.0"))).arrayBuffer());
  const sha = createHash("sha256").update(zip).digest("hex");
  expect(release.files[0].sha256).toBe(sha);
  const visitor = await page.context().browser()!.newPage();
  await visitor.goto("/verify");
  await visitor.getByLabel("Choose the file").setInputFiles({ name: "tide.zip", mimeType: "application/zip", buffer: zip });
  await expect(visitor.getByLabel("Or paste its SHA-256")).toHaveValue(sha);
  const answerList = visitor.getByRole("status");
  await expect(answerList.getByRole("listitem").filter({ hasText: /^Approved on Slop: Lantern Keep 1\.0\.0/ })).toContainText(
    /Approved on Slop: Lantern Keep 1\.0\.0 by Mia Maker, for windows\. Approved .*; scanned, nothing found\./,
  );
  await expect(answerList.getByRole("link", { name: "Lantern Keep", exact: true })).toHaveAttribute("href", "/app/lantern-keep");
  // A draft isn't in the store, so it isn't named.
  await expect(answerList.getByRole("listitem").filter({ hasText: title })).toHaveCount(0);

  // Staff withdraw it: it no longer launches, and the check says not to run it.
  await call("admin", "POST", `/v1/admin/releases/${release.id}/revoke`, { reason: "Ships a debug backdoor." });
  await page.reload();
  await expect(row.getByText("WITHDRAWN", { exact: true })).toBeVisible();
  await expect(row.getByText("Slop withdrew this release; it no longer launches.")).toBeVisible();
  await visitor.getByLabel("Or paste its SHA-256").fill(sha.toUpperCase());
  await visitor.getByRole("button", { name: "Check" }).click();
  await expect(answerList.getByRole("listitem").filter({ hasText: title })).toContainText(/Withdrawn by Slop: .*Withdrawn .*: don't run it\./);

  // A hash Slop never saw, pasted.
  await visitor.getByLabel("Or paste its SHA-256").fill("ab".repeat(32));
  await visitor.getByRole("button", { name: "Check" }).click();
  await expect(visitor.getByText("Slop doesn't know this file.")).toBeVisible();
  // Not a SHA-256: nothing to check.
  await visitor.getByLabel("Or paste its SHA-256").fill("not a hash");
  await expect(visitor.getByRole("button", { name: "Check" })).toBeDisabled();
});
