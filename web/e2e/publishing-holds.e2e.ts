// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, a release on hold (web/src/components/ReleaseHold.tsx): its maker sees what holds
// it in plain words, with what the scanner found; writes to SLAPorSLOP staff, who answer; runs the
// scans again; sees Publish waiting for the hold to end; and asks staff about a release whose
// checks failed. The seeded hold is Bo's Pocket
// Golf 1.1.0 (a scanner flagged it); it stays on hold (the staff queue's tests need it). The test
// server runs no scanner, so what Scan again does once allowed is answered in the browser.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { expect, ids, server, signIn, test } from "./fixtures";
import { answer, apiError, call, createApp, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

const golf = ids.releaseInReviewApp as string;

async function axe(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

interface Row {
  id: string;
  version: string;
  status: string;
}

const heldId = async () => (await call<Row[]>("maker2", "GET", `/v1/apps/${golf}/releases`)).find((r) => r.version === "1.1.0")!.id;

test.afterAll(async () => {
  await readNotices("maker2", golf);
});

test("the maker sees what holds a release, with what was found, and writes to SLAPorSLOP staff, who answer", async ({ page, browser }) => {
  await signIn(page, "maker2");
  await page.goto(`/publish/${golf}/releases`);
  const release = page.locator("ul.release-list > li").filter({ hasText: "1.1.0" });
  await expect(release.getByText("ON HOLD", { exact: true })).toBeVisible();
  const hold = release.getByRole("region", { name: "Why it's on hold" });
  // In plain words, with what the scanner matched and where (as code, so it can be found).
  await expect(hold).toContainText("A scanner flagged the Windows (.zip) build");
  await expect(hold).toContainText("ClamAV, a virus scanner, matched the signature Win.Test.Seed in pong.exe. Scanners sometimes match harmless programs.");
  await expect(hold.locator("code").filter({ hasText: "Win.Test.Seed" })).toBeVisible();
  await expect(hold).toContainText("SLAPorSLOP staff will look at it. You'll get a notice when they decide or answer.");
  // Fixing the build is doable right here: the new version's form is on this page.
  await expect(hold.getByRole("link", { name: "Send a new version" })).toHaveAttribute("href", "#new-release");
  await expect(page.locator("#new-release")).toHaveCount(1);
  // The scanner is off on this test server: said so, and no button says it would run.
  await expect(hold.getByRole("button", { name: "Scan again" })).toHaveCount(0);
  await expect(hold).toContainText("SLAPorSLOP's scanner is off right now, so it can't scan again.");
  await expect(hold).not.toContainText(/\bsafe\b|\bverified\b|\btrusted\b/i);
  // The reason is said once, by what holds it: no notice repeating it above.
  await expect(release.locator("p.notice").filter({ hasText: /^On hold:/ })).toHaveCount(0);
  // Publish is there, unavailable, saying when it will be; withdrawing it is right here too.
  const publish = release.getByRole("button", { name: "Publish version 1.1.0", exact: true });
  await expect(publish).toBeDisabled();
  await expect(publish).toHaveAccessibleDescription("You can publish once the hold ends.");
  await expect(hold.getByRole("button", { name: "Withdraw version 1.1.0…" })).toBeVisible();
  await axe(page);

  // Writing to staff: a labelled box, sent with the keyboard.
  const box = hold.getByRole("textbox", { name: "Your message to SLAPorSLOP staff" });
  await expect(box).toHaveAttribute("maxlength", "2000");
  const send = hold.getByRole("button", { name: "Write to SLAPorSLOP staff" });
  await expect(send).toBeDisabled();
  await box.fill("pong.exe is our own test harness, built in CI from tools/pong. It matches the test signature on purpose.");
  await page.keyboard.press("Tab");
  await expect(send).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(hold.getByText("Sent. SLAPorSLOP staff are told, and you'll get a notice when they answer.")).toBeVisible();
  await expect(box).toHaveValue("");
  const log = hold.getByRole("list", { name: "Conversation with SLAPorSLOP staff about version 1.1.0" });
  await expect(log.getByRole("listitem").last()).toContainText("pong.exe is our own test harness");
  await expect(log.getByRole("listitem").last()).toContainText("You");
  await expect(hold).toContainText("they haven't answered yet");

  // Staff see it marked as Bo's words, and answer from its card.
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue&box=releases");
  const row = admin.locator(".inbox-row").filter({ hasText: "Pocket Golf 1.1.0" });
  await expect(row.locator(".inbox-blurb")).toContainText("the maker wrote");
  await row.getByRole("link").first().click();
  const card = admin.locator(".inbox-detail");
  await expect(card.getByText("MAKER WROTE", { exact: true })).toBeVisible();
  await expect(card.getByRole("list", { name: "Conversation about Pocket Golf 1.1.0" })).toContainText(
    "Bo Builder, the maker",
  );
  await card.getByRole("textbox", { name: "Reply to the maker" }).fill("Thanks Bo: we're looking at tools/pong now.");
  await card.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(card.getByText("Sent to the maker.")).toBeVisible();
  await admin.close();

  // Bo is told, and reads the answer on his Releases page.
  const notices = await call<{ items: Array<{ title: string; link: string | null }> }>("maker2", "GET", "/v1/me/notifications");
  expect(notices.items.find((n) => n.title === "SLAPorSLOP staff answered about Pocket Golf 1.1.0")?.link).toBe(`/publish/${golf}/releases`);
  await page.reload();
  await expect(log).toContainText("Thanks Bo: we're looking at tools/pong now.");
  await expect(hold).not.toContainText("they haven't answered yet");
  // Still on hold: staff decide it.
  expect((await call<Row[]>("maker2", "GET", `/v1/apps/${golf}/releases`)).find((r) => r.version === "1.1.0")!.status).toBe(
    "awaiting_review",
  );
});

test("Scan again: the re-runs left, when the next is allowed, and what happens once asked", async ({ page }) => {
  const id = await heldId();
  const real = await call<Record<string, unknown>>("maker2", "GET", `/v1/apps/${golf}/releases/${id}/hold`);
  // As a scanning server answers: a re-run is allowed now.
  const allowed = { ...real, you_can: ["rescan", "write_to_staff", "new_release", "withdraw"], rescans_left: 3, next_rescan_at: null };
  await answer(page, `/v1/apps/${golf}/releases/${id}/hold`, allowed, 200, "GET");
  await signIn(page, "maker2");
  await page.goto(`/publish/${golf}/releases`);
  const hold = page.locator("ul.release-list > li").filter({ hasText: "1.1.0" }).getByRole("region", { name: "Why it's on hold" });
  await expect(hold).toContainText("Scan again downloads the same files from your release page, checks they haven't changed");
  await expect(hold).toContainText("It can't run Microsoft Defender again.");
  const again = hold.getByRole("button", { name: "Scan again" });
  await expect(again).toHaveAccessibleDescription("3 of 3 re-runs left.");

  // This test server's own answer: its scanner is off. Said, and nothing changes.
  await again.click();
  await expect(hold.getByRole("alert")).toHaveText("SLAPorSLOP's scanner is off right now, so it can't scan again.");
  await expect(again).toBeVisible();

  // A scanning server takes it: the re-run waits for the scanner, and there's no button meanwhile.
  await answer(
    page,
    `/v1/apps/${golf}/releases/${id}/rescan`,
    { ...allowed, you_can: ["write_to_staff", "new_release", "withdraw"], rescans_left: 2, rescan_requested: true },
    200,
    "POST",
  );
  await again.click();
  await expect(hold.getByRole("status").filter({ hasText: "SLAPorSLOP will scan the held files again shortly. You'll be told the result." })).toBeVisible();
  await expect(hold.getByRole("button", { name: "Scan again" })).toHaveCount(0);

  // Six hours apart: when the next one is allowed; and none left after three.
  await page.unroute(`**/v1/apps/${golf}/releases/${id}/hold`);
  await answer(page, `/v1/apps/${golf}/releases/${id}/hold`, { ...allowed, rescans_left: 2, next_rescan_at: "2026-11-14T18:00:00Z" }, 200, "GET");
  await page.reload();
  await expect(hold).toContainText(/2 of 3 re-runs left\. The next is allowed from /);
  await expect(hold.getByRole("button", { name: "Scan again" })).toHaveCount(0);
  await page.unroute(`**/v1/apps/${golf}/releases/${id}/hold`);
  await answer(page, `/v1/apps/${golf}/releases/${id}/hold`, { ...allowed, you_can: ["write_to_staff"], rescans_left: 0 }, 200, "GET");
  await page.reload();
  await expect(hold).toContainText("No re-runs left: SLAPorSLOP runs the scans again up to 3 times per release. Write to SLAPorSLOP staff, or send a new version.");
  // And what's left to do is right there.
  await expect(hold.getByRole("textbox", { name: "Your message to SLAPorSLOP staff" })).toBeVisible();
});

test("a failed release: its maker asks SLAPorSLOP staff, who answer, and can ask again", async ({ page, browser }) => {
  const slug = uniq("pub-ask");
  const repo = `${server.fake}bo/${slug}`;
  await createApp("maker2", slug, `Ask Away ${slug.slice(-4)}`, repo);
  // On the right site, but not there: its checks fail.
  await call("maker2", "POST", `/v1/apps/${slug}/releases`, {
    version: "1.0.0",
    commit_sha: "c".repeat(40),
    files: [{ platform: "windows", kind: "archive", url: `${repo}/files/missing.zip`, exec: "pong.exe", args: [] }],
  });
  await signIn(page, "maker2");
  await page.goto(`/publish/${slug}/releases`);
  const release = page.locator("ul.release-list > li").filter({ hasText: "1.0.0" });
  await expect(release.getByText("FAILED", { exact: true })).toBeVisible({ timeout: 20_000 });
  await release.getByRole("button", { name: "Ask SLAPorSLOP staff" }).click();
  const ask = release.getByRole("region", { name: "Ask SLAPorSLOP staff" });
  const box = ask.getByRole("textbox", { name: "Your question for SLAPorSLOP staff" });
  await box.fill("The file is attached on GitHub: why couldn't you download it?");
  await ask.getByRole("button", { name: "Ask SLAPorSLOP staff" }).click();
  await expect(ask.getByText("Sent. SLAPorSLOP staff are told, and you'll get a notice when they answer.")).toBeVisible();
  await expect(ask).toContainText("You asked SLAPorSLOP staff about it.");

  // Staff: a question about a failed release, answered (that closes it); nothing to accept.
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue&box=releases");
  const row = admin.locator(".inbox-row").filter({ hasText: `${slug.slice(-4)} 1.0.0` });
  await expect(row.getByText("QUESTION")).toBeVisible();
  await expect(row.getByRole("button", { name: /^Accept / })).toHaveCount(0);
  await row.getByRole("link").first().click();
  const card = admin.locator(".inbox-detail");
  await expect(card.getByText(/The maker asks about this failed release\./)).toBeVisible();
  await expect(card.getByRole("button", { name: "Accept" })).toHaveCount(0);
  await card.getByRole("textbox", { name: "Answer the maker" }).fill("It answered 404 at files/missing.zip: attach the build to the release, then send a new version.");
  await card.getByRole("button", { name: "Answer", exact: true }).click();
  await expect(admin.locator(".inbox-row").filter({ hasText: `${slug.slice(-4)} 1.0.0` })).toHaveCount(0);
  await admin.close();

  await page.reload();
  const thread = release.getByRole("region", { name: "Ask SLAPorSLOP staff" });
  await expect(thread).toContainText("SLAPorSLOP staff answered your question. You can ask again below.");
  await expect(thread.getByRole("list", { name: /Conversation with SLAPorSLOP staff/ })).toContainText("attach the build to the release");
  await expect(thread.getByRole("textbox", { name: "Your question for SLAPorSLOP staff" })).toBeVisible();

  // Too long, or with too many links: said by the box.
  await answer(page, `/v1/apps/${slug}/releases/*/hold/messages`, apiError("invalid", "at most 3 links", "body"), 422, "POST");
  await thread.getByRole("textbox", { name: "Your question for SLAPorSLOP staff" }).fill("a b c d");
  await thread.getByRole("button", { name: "Ask SLAPorSLOP staff" }).click();
  await expect(thread.getByRole("alert")).toHaveText("at most 3 links");
  await readNotices("maker2", slug);
});
