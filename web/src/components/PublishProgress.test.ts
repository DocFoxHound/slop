// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { isValidElement, type ReactNode } from "react";
import type { AppDetail, Release } from "../api";
import { about, ago, currentStep, fileLabel, holdActions, progressSteps, refreshEvery } from "./PublishProgress";

/** The text in a step's detail, which may hold links. */
function textOf(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) {
    // Our own small components: render them to find their text.
    if (typeof node.type === "function") return textOf((node.type as (p: unknown) => ReactNode)(node.props));
    return textOf(node.props.children);
  }
  return "";
}

const app = (over: Partial<AppDetail> = {}) =>
  ({ slug: "game", stage: "beta", status: "draft", ...over }) as AppDetail;

const file = (
  over: Partial<Release["files"][number]> = {},
): Release["files"][number] => ({
  platform: "windows",
  kind: "archive",
  url: "https://github.com/me/game/releases/download/v1/game.zip",
  file_name: "game.zip",
  exec: "game.exe",
  args: [],
  size_bytes: 10,
  sha256: "ab".repeat(32),
  provenance: null,
  provenance_problem: "no build attestation found",
  maker_key: null,
  scan_result: "skipped",
  scan_detail: "not scanned: scanning is off on this server",
  scan_report: null,
  defender: null,
  cleared: false,
  cleared_reason: null,
  ...over,
});

const release = (over: Partial<Release> = {}): Release => ({
  id: "r1",
  version: "1.0.0",
  commit_sha: "a".repeat(40),
  status: "awaiting_review",
  commit_verified: false,
  created_at: "2026-10-18T00:00:00Z",
  source: "maker",
  auto_approved: false,
  files: [file()],
  source_checks: null,
  repo_scan: {
    status: "checked",
    licence: {
      declared: "MIT",
      files: [{ path: "LICENSE", spdx: "MIT" }],
      headers: [],
      problems: [],
    },
    monetization: [],
  },
  advisories: [],
  ...over,
});

const testServer = { scanning: false, maker_signatures_required: false };
const states = (steps: ReturnType<typeof progressSteps>) =>
  Object.fromEntries(steps.map((s) => [s.title, s.state]));

describe("progressSteps", () => {
  it("asks for a release when there's none", () => {
    expect(states(progressSteps(app(), [], testServer))).toMatchObject({
      Release: "waiting",
      "In the store": "waiting",
    });
  });

  it("shows a release on hold, and one that passed, on a test server", () => {
    expect(states(progressSteps(app(), [release()], testServer))).toEqual({
      "Store page": "done",
      Release: "done",
      "Files checked": "done",
      "Maker signature": "skipped",
      Scan: "skipped",
      "Source check": "done",
      Checks: "problem",
      Publish: "waiting",
      "In the store": "waiting",
    });
    expect(states(progressSteps(app(), [release({ status: "ready" })], testServer))).toMatchObject({
      Checks: "done",
      Publish: "active",
      "In the store": "waiting",
    });
  });

  it("shows source-check notes as information, never as something to wait for", () => {
    const repo_scan = {
      status: "checked" as const,
      licence: {
        declared: "MIT",
        files: [],
        headers: [],
        problems: ["There's no licence file"],
      },
      monetization: [
        {
          rule: "loot_box",
          label: "Loot boxes or gacha",
          count: 2,
          examples: ["src/a.gd:3"],
        },
      ],
    };
    const waiting = progressSteps(app(), [release({ repo_scan, status: "ready" })], testServer);
    expect(waiting.find((s) => s.title === "Source check")).toMatchObject({
      state: "done",
      stateLabel: "Done, with notes",
    });
    expect(
      textOf(waiting.find((s) => s.title === "Source check")!.detail),
    ).toContain("2 notes");
    const approved = progressSteps(
      app(),
      [release({ repo_scan, status: "approved" })],
      testServer,
    );
    expect(states(approved)["Source check"]).toBe("done");
    expect(
      states(progressSteps(app(), [release({ repo_scan: null })], testServer))[
        "Source check"
      ],
    ).toBe("active");
  });

  it("counts code from outside SLAPorSLOP and undeclared online play as notes", () => {
    const repo_scan = {
      status: "checked" as const,
      licence: { declared: "MIT", files: [], headers: [], problems: [] },
      monetization: [],
      code_loading: [
        { rule: "load_script", label: "Runs code from text", count: 1, examples: ["mods.lua:4"] },
      ],
      network: [{ rule: "multiplayer", label: "Multiplayer", count: 1, examples: ["net.gd:1"] }],
      online_undeclared: true,
    };
    const step = progressSteps(app(), [release({ repo_scan })], testServer).find(
      (s) => s.title === "Source check",
    )!;
    expect(step.state).toBe("done");
    expect(textOf(step.detail)).toContain("2 notes");
  });

  it("flags a missing signature when signatures are required", () => {
    const steps = progressSteps(app(), [release()], {
      scanning: false,
      maker_signatures_required: true,
    });
    const signature = steps.find((s) => s.title === "Maker signature")!;
    expect(signature.state).toBe("problem");
    expect(signature.detail).toContain("no build attestation");
  });

  it("shows failed checks and rejections with their reasons", () => {
    const failed = progressSteps(
      app(),
      [
        release({
          status: "failed",
          status_reason: "download failed: HTTP 404",
        }),
      ],
      testServer,
    );
    const files = failed.find((s) => s.title === "Files checked")!;
    expect(files.state).toBe("problem");
    expect(textOf(files.detail)).toContain("SLAPorSLOP couldn't check the files: download failed: HTTP 404.");
    expect(textOf(files.detail)).toContain("See what to fix");
    const rejected = progressSteps(
      app(),
      [release({ status: "rejected", status_reason: "loot boxes" })],
      testServer,
    );
    expect(
      textOf(rejected.find((s) => s.title === "Checks")!.detail),
    ).toContain("Turned down by SLAPorSLOP staff: loot boxes. Fix it and submit a new version, or ask SLAPorSLOP staff about it.");
    // A failed release can always be asked about.
    const checks = failed.find((s) => s.title === "Checks")!;
    expect(checks).toMatchObject({ state: "problem", stateLabel: "Failed" });
    expect(textOf(checks.detail)).toContain("ask SLAPorSLOP staff if you think the checks got it wrong");
  });

  it("leaves out a release withdrawn before it was published", () => {
    const steps = progressSteps(
      app(),
      [
        release({ id: "r2", version: "1.1.0", status: "withdrawn" }),
        release({ status: "rejected", status_reason: "loot boxes" }),
      ],
      testServer,
    );
    expect(steps.find((s) => s.title === "Release")!.detail).toBe("Version 1.0.0 received.");
    expect(textOf(steps.find((s) => s.title === "Checks")!.detail)).toContain("loot boxes");
    const only = progressSteps(app(), [release({ status: "withdrawn" })], testServer);
    expect(only.find((s) => s.title === "Release")!.state).toBe("waiting");
  });

  it("waits for its maker to put it in the store once a release is published", () => {
    const steps = progressSteps(app(), [release({ status: "approved" })], testServer);
    expect(states(steps)).toMatchObject({
      Checks: "done",
      Publish: "done",
      "In the store": "active",
    });
    expect(steps.at(-1)).toMatchObject({
      stateLabel: "Waiting for you",
      detail: "Ready: put it in the store when you want players to find it.",
      action: "list",
    });
    // A private app is shared, never listed by its maker.
    expect(progressSteps(app({ private: true }), [release({ status: "approved" })], testServer).at(-1)).toMatchObject({
      state: "skipped",
      stateLabel: "Private",
    });
    expect(steps.at(-1)!.action).toBe("list");
    expect(
      states(
        progressSteps(
          app({ status: "listed" }),
          [release({ status: "approved" })],
          testServer,
        ),
      ),
    ).toMatchObject({
      "In the store": "done",
    });
    // Listed apps come back from the API without a status field.
    expect(
      states(
        progressSteps(
          app({ status: undefined }),
          [release({ status: "approved" })],
          testServer,
        ),
      ),
    ).toMatchObject({
      "In the store": "done",
    });
  });

  it("needs no release for a Coming soon page", () => {
    expect(
      states(progressSteps(app({ stage: "coming_soon" }), [], testServer)),
    ).toEqual({
      "Store page": "done",
      Release: "skipped",
      "In the store": "active",
    });
  });

  it("says what holds a release, and that one that passed waits for its maker to publish it", () => {
    const summary = "On hold: a scanner flagged the Windows (.zip) build. The Releases page shows what was found.";
    const held = progressSteps(
      app(),
      [
        release({
          status_reason: "On hold: a scanner flagged the Windows (.zip) build",
          progress: { stage: "on_hold", files_total: 1, files_checked: 1, started_at: null, seconds_per_file: null, remaining_seconds: null, summary },
          hold: { status: "open", kinds: ["scan"], needs_staff: true, rescans_left: 3, unanswered: false },
        }),
      ],
      testServer,
    );
    const checks = held.find((s) => s.title === "Checks")!;
    expect(checks).toMatchObject({ state: "problem", stateLabel: "On hold", detail: summary });
    expect(textOf(checks.more)).toBe(
      "See what holds version 1.0.0: scan again, write to SLAPorSLOP staff, or send a new version.",
    );
    // A missing Defender check is the maker's: it goes on by itself once Slop Check reports.
    const missing = progressSteps(
      app(),
      [release({ hold: { status: "open", kinds: ["defender_missing"], needs_staff: false, rescans_left: 3, unanswered: false } })],
      testServer,
    ).find((s) => s.title === "Checks")!;
    expect(missing.stateLabel).toBe("Waiting for Microsoft Defender's check");
    expect(textOf(missing.more)).toContain("run Slop Check, and it goes on by itself");
    // What the maker can do follows what holds it: Scan again only for a scanner's hold with re-runs left.
    const summary1 = (kinds: string[], over: object = {}) =>
      ({ status: "open", kinds, needs_staff: true, rescans_left: 3, unanswered: false, ...over }) as Release["hold"];
    expect(holdActions(summary1(["rating_lowered"]), false)).toBe(": write to SLAPorSLOP staff there, or send a new version.");
    expect(holdActions(summary1(["defender"]), false)).toBe(": write to SLAPorSLOP staff there, or send a new version.");
    expect(holdActions(summary1(["scan"], { rescans_left: 0 }), false)).toBe(": write to SLAPorSLOP staff there, or send a new version.");
    expect(holdActions(summary1(["scan", "rating_lowered"]), false)).toBe(": scan again, write to SLAPorSLOP staff, or send a new version.");
    // Outside GitHub, a missing Defender check is staff's (needs_staff): no Slop Check.
    expect(holdActions(summary1(["defender_missing"]), false)).toBe(": write to SLAPorSLOP staff there, or send a new version.");

    const ready = progressSteps(app({ status: "listed" }), [release({ status: "ready" })], testServer);
    const publish = ready.find((s) => s.title === "Publish")!;
    expect(publish).toMatchObject({ state: "active", stateLabel: "Waiting for you" });
    expect(textOf(publish.detail)).toBe("Version 1.0.0 passed every check. Publish it and players get it.");
    expect(currentStep(ready).title).toBe("Publish");
    // A draft: players get nothing until its maker puts it in the store.
    const draftReady = progressSteps(app(), [release({ status: "ready" })], testServer).find((s) => s.title === "Publish")!;
    expect(textOf(draftReady.detail)).toBe(
      "Version 1.0.0 passed every check. Publish it; players can get it once you put the app in the store.",
    );
    const draftUpdate = progressSteps(app(), [release({ id: "r2", version: "1.1.0", status: "fetching" }), release({ status: "approved" })], testServer);
    expect(draftUpdate.find((s) => s.title === "Publish")!.detail).toBe(
      "Version 1.0.0 is published: players can get it once you put the app in the store.",
    );
    const cleared = progressSteps(app(), [release({ status: "ready", hold: { status: "cleared", kinds: ["scan"], needs_staff: true, rescans_left: 3, unanswered: false } })], testServer);
    expect(cleared.find((s) => s.title === "Checks")!.detail).toBe("Version 1.0.0 passed: SLAPorSLOP staff accepted its hold.");

    const published = progressSteps(app({ status: "listed" }), [release({ status: "approved" })], testServer);
    expect(published.find((s) => s.title === "Publish")!.detail).toBe("You published version 1.0.0.");
    // An update under way leaves the published one with players.
    const update = progressSteps(app({ status: "listed" }), [release({ id: "r2", version: "1.1.0", status: "fetching" }), release({ status: "approved" })], testServer);
    expect(update.find((s) => s.title === "Publish")).toMatchObject({
      state: "done",
      detail: "Version 1.0.0 is published: players keep it until you publish a newer one.",
    });
  });

  it("points at the step an app is stuck on or waiting for", () => {
    expect(currentStep(progressSteps(app(), [], testServer)).title).toBe(
      "Release",
    );
    expect(
      currentStep(progressSteps(app(), [release()], testServer)).title,
    ).toBe("Checks");
    const failed = progressSteps(
      app(),
      [release({ status: "failed", status_reason: "HTTP 404" })],
      testServer,
    );
    expect(currentStep(failed)).toMatchObject({
      title: "Files checked",
      state: "problem",
    });
    const listed = progressSteps(
      app({ status: "listed" }),
      [release({ status: "approved" })],
      testServer,
    );
    expect(currentStep(listed)).toMatchObject({
      title: "In the store",
      state: "done",
    });
  });
  it("shows each file while SLAPorSLOP checks them, with an honest estimate", () => {
    const now = Date.parse("2026-10-25T12:00:00Z");
    const min = (n: number) => new Date(now - n * 60_000).toISOString();
    const checking = release({
      status: "fetching",
      source: "slop_build",
      repo_scan: null,
      files: [
        file({ platform: "linux", file_name: "g.tar.gz", check: "checked", check_started_at: min(12), checked_at: min(7), scan_result: "passed", provenance_problem: null }),
        file({ platform: "linux-deb", file_name: "g.deb", check: "checking", check_stage: "scanning", check_started_at: min(2), sha256: null, scan_result: null }),
        file({ platform: "linux-rpm", file_name: "g.rpm", check: "waiting", sha256: null, scan_result: null }),
        file({ platform: "windows", file_name: "g.zip", check: "waiting", sha256: null, scan_result: null }),
      ],
      progress: {
        stage: "checking_files",
        files_total: 4,
        files_checked: 1,
        started_at: min(12),
        seconds_per_file: 300,
        remaining_seconds: 780,
        summary: "",
      },
    });
    const steps = progressSteps(app(), [checking], { scanning: true, maker_signatures_required: false }, now);
    const files = steps.find((s) => s.title === "Files checked")!;
    expect(files.state).toBe("active");
    expect(files.stateLabel).toBe("1 of 4 done");
    expect(files.detail).toBe("Checking file 2 of 4: Linux (.deb) — Scanning for malware");
    expect(files.more).toBe("About 5 minutes per file; started 12 minutes ago. About 13 minutes left.");
    expect(files.items!.map((i) => [i.label, i.state, i.status])).toEqual([
      ["Linux (.tar.gz)", "done", "Passed"],
      ["Linux (.deb)", "active", "Scanning for malware"],
      ["Linux (.rpm)", "waiting", "Waiting"],
      ["Windows (.zip)", "waiting", "Waiting"],
    ]);
    expect(files.items![0].detail).toBe("Took 5 minutes");
    expect(files.items![1].detail).toBe("Started 2 minutes ago");
    expect(steps.find((s) => s.title === "Release")!.detail).toBe("Version 1.0.0 received from Slop Build.");
    expect(states(steps)).toMatchObject({ Scan: "waiting", "Source check": "waiting", Checks: "waiting", Publish: "waiting" });
    expect(currentStep(steps).title).toBe("Files checked");

    // A file taking longer than usual says so instead of counting below zero.
    const slow = progressSteps(app(), [checking], { scanning: true, maker_signatures_required: false }, now + 6 * 60_000);
    expect(slow.find((s) => s.title === "Files checked")!.more).toContain("This file is taking longer than usual");
  });

  it("says a release is waiting its turn, without counting who is ahead", () => {
    const queued = release({
      status: "fetching",
      files: [file({ check: "waiting", sha256: null, scan_result: null })],
      progress: { stage: "queued", files_total: 1, files_checked: 0, started_at: null, seconds_per_file: 300, remaining_seconds: null, summary: "" },
    });
    const files = progressSteps(app(), [queued], testServer).find((s) => s.title === "Files checked")!;
    expect(files.stateLabel).toBe("Waiting its turn");
    expect(files.detail).toContain("SLAPorSLOP checks one release at a time.");
    expect(files.detail).not.toContain("ahead");
    expect(files.more).toBe("Each file takes about 5 minutes here.");
  });

  it("shows the other checks on each file, and a file on hold in plain words", () => {
    const steps = progressSteps(
      app(),
      [
        release({
          files: [
            file({ check: "checked", scan_result: "on_hold", scan_detail: "YARA: Packed_UPX.", provenance: {} as never, defender: { sha256: "", engine: "1", signatures: "1", signatures_updated: null, detections: [], signed_by: null } }),
            file({ platform: "linux", file_name: "g.tar.gz", check: "checked", scan_result: "note", scan_detail: "It can log keystrokes.", maker_key: { fingerprint: "SHA256:x", key_name: "laptop" } }),
          ],
        }),
      ],
      { scanning: true, maker_signatures_required: true },
    );
    const items = steps.find((s) => s.title === "Files checked")!.items!;
    expect(items[0]).toMatchObject({ label: "Windows (.zip)", state: "problem", status: "On hold" });
    expect(items[0].detail).toBe("YARA: Packed_UPX. · signed by your build · Microsoft Defender found nothing");
    expect(items[1]).toMatchObject({ status: "Passed, with a note" });
    expect(items[1].detail).toContain('signed with your key "laptop"');
    const scan = steps.find((s) => s.title === "Scan")!;
    expect(scan).toMatchObject({ state: "problem", stateLabel: "On hold", detail: "Windows (.zip) is on hold: YARA: Packed_UPX." });
    expect(textOf(scan.more)).toContain("SLAPorSLOP staff look at it, and you can scan again or tell them why it's a false positive");
    expect(states(steps)["Maker signature"]).toBe("done");
    expect(currentStep(steps).title).toBe("Scan");
    // Cleared by staff: done, with their note if they left one.
    const cleared = progressSteps(
      app(),
      [release({ status: "ready", files: [file({ check: "checked", scan_result: "on_hold", scan_detail: "YARA: Packed_UPX.", cleared: true })] })],
      { scanning: true, maker_signatures_required: false },
    );
    expect(cleared.find((s) => s.title === "Files checked")!.items![0]).toMatchObject({
      state: "done",
      status: "Cleared by SLAPorSLOP staff",
    });
    expect(cleared.find((s) => s.title === "Files checked")!.items![0].detail).toMatch(
      /^Found: YARA: Packed_UPX\.; cleared as a false positive · /,
    );
    const noted = progressSteps(
      app(),
      [release({ status: "ready", files: [file({ check: "checked", scan_result: "on_hold", scan_detail: "YARA: Packed_UPX.", cleared: true, cleared_reason: "UPX-packed Godot runtime." })] })],
      { scanning: true, maker_signatures_required: false },
    );
    expect(noted.find((s) => s.title === "Files checked")!.items![0].detail).toContain(
      "cleared as a false positive: UPX-packed Godot runtime.",
    );
    expect(noted.find((s) => s.title === "Scan")!.detail).toBe("Flagged, then cleared by SLAPorSLOP staff as a false positive.");
  });

  it("refreshes often while files are checked, now and then on hold, and stops when it waits for its maker", () => {
    expect(refreshEvery([release({ status: "fetching" })])).toBe(10_000);
    expect(refreshEvery([release({ status: "awaiting_review" })])).toBe(60_000);
    // Passed, but SLAPorSLOP hasn't read the source yet: Publish waits for that.
    expect(refreshEvery([release({ status: "ready", repo_scan: null })])).toBe(60_000);
    expect(refreshEvery([release({ status: "ready" })])).toBeNull();
    expect(refreshEvery([release({ status: "approved" })])).toBeNull();
    expect(refreshEvery([release({ status: "withdrawn" }), release({ status: "rejected" })])).toBeNull();
    expect(refreshEvery([])).toBeNull();
    expect(refreshEvery(undefined)).toBeNull();
  });

  describe("a browser game, whose release is a play address", () => {
    const game = (over: Partial<AppDetail> = {}) => app({ plays_in_browser: true, ...over });
    const address = (over: Partial<Release> = {}) =>
      release({
        files: [],
        play_address: "https://games.example.com/play/",
        address_checks: {
          https: true,
          public_host: true,
          bad_links: { listed_by: null, lists_loaded: false },
          status: 200,
          final_address: "https://games.example.com/play/",
          redirects: 0,
          content_type: "text/html",
          checked_at: "2026-11-07T10:00:00Z",
          ok: true,
          problem: null,
        },
        ...over,
      });
    // Every step's words but the one that says what isn't checked (which names it to say so).
    const all = (steps: ReturnType<typeof progressSteps>) =>
      steps
        .filter((s) => s.title !== "What SLAPorSLOP can't check")
        .map((s) => `${textOf(s.detail)} ${textOf(s.more)}`)
        .join(" ");

    it("asks for the play address, and none for a Coming soon page", () => {
      const none = progressSteps(game(), [], testServer);
      expect(states(none)).toEqual({ "Store page": "done", "Play address": "waiting", "In the store": "waiting" });
      expect(currentStep(none).title).toBe("Play address");
      expect(none.find((s) => s.title === "In the store")!.detail).toBe("Once you've published its play address.");
      expect(states(progressSteps(game({ stage: "coming_soon" }), [], testServer))).toEqual({
        "Store page": "done",
        "Play address": "skipped",
        "In the store": "active",
      });
    });

    it("has no file, signature or scan steps, and says what SLAPorSLOP can't check", () => {
      const steps = progressSteps(game(), [address()], { scanning: true, maker_signatures_required: true });
      expect(states(steps)).toEqual({
        "Store page": "done",
        "Play address": "done",
        "Address checked": "done",
        "What SLAPorSLOP can't check": "skipped",
        "Source check": "done",
        Checks: "problem",
        Publish: "waiting",
        "In the store": "waiting",
      });
      expect(steps.find((s) => s.title === "Play address")!.detail).toBe("Version 1.0.0: https://games.example.com/play/ received.");
      expect(textOf(steps.find((s) => s.title === "What SLAPorSLOP can't check")!.detail)).toContain(
        "SLAPorSLOP checked the address, not what the site serves",
      );
      const checked = steps.find((s) => s.title === "Address checked")!;
      expect(checked.itemName).toBe("check");
      expect(checked.items!.map((i) => [i.label, i.status])).toEqual([
        ["https", "Yes"],
        ["A public site", "Yes"],
        ["Bad-link lists", "Not checked"],
        ["The page loads", "Yes (HTTP 200)"],
      ]);
      expect(checked.items![2].detail).toContain("No bad-link lists are loaded on this server");
      const words = all(steps);
      expect(words).not.toMatch(/\bsigned\b|\bscanned\b|downloaded|hashed/i);
      expect(textOf(steps.find((s) => s.title === "Checks")!.more)).toContain("write to SLAPorSLOP staff there");
      const ready = progressSteps(game({ status: "listed" }), [address({ status: "ready" })], testServer);
      expect(textOf(ready.find((s) => s.title === "Publish")!.detail)).toBe(
        "Version 1.0.0 passed every check. Publish it and Play opens its address.",
      );
      expect(textOf(progressSteps(game(), [address({ status: "ready" })], testServer).find((s) => s.title === "Publish")!.detail)).toBe(
        "Version 1.0.0 passed every check. Publish it; players can play it once you put the game in the store.",
      );
      expect(all(ready)).not.toMatch(/\bsigned\b|\bscanned\b|downloaded|hashed/i);
    });

    it("shows the checks under way, and a failure with its reason and what to fix", () => {
      const checking = progressSteps(game(), [address({ status: "fetching", address_checks: null, repo_scan: null })], testServer);
      const step = checking.find((s) => s.title === "Address checked")!;
      expect(step).toMatchObject({ state: "active", stateLabel: "Checking" });
      expect(step.items!.every((i) => i.state === "active")).toBe(true);
      expect(currentStep(checking).title).toBe("Address checked");
      expect(refreshEvery([address({ status: "fetching" })])).toBe(10_000);

      const failed = progressSteps(
        game(),
        [address({ status: "failed", status_reason: "the page didn't load: the site answered HTTP 404", address_checks: null })],
        testServer,
      );
      const problem = failed.find((s) => s.title === "Address checked")!;
      expect(problem.state).toBe("problem");
      expect(textOf(problem.detail)).toContain(
        "SLAPorSLOP couldn't check the address: the page didn't load: the site answered HTTP 404. Fix it and submit the address again.",
      );
      expect(textOf(problem.detail)).toContain("See what to fix");
      expect(currentStep(failed).title).toBe("Address checked");
    });

    it("doesn't count a site's networking or code loading as notes", () => {
      const repo_scan = {
        status: "checked" as const,
        licence: { declared: "MIT", files: [], headers: [], problems: [] },
        monetization: [],
        code_loading: [{ rule: "load_script", label: "Runs code from text", count: 1, examples: ["game.js:4"] }],
        saves: [{ rule: "appdata", label: "Writes to AppData", count: 1, examples: ["game.js:9"] }],
        online_undeclared: true,
      };
      const steps = progressSteps(game(), [address({ repo_scan: repo_scan as Release["repo_scan"] })], testServer);
      expect(states(steps)["Source check"]).toBe("done");
    });

    it("once published, says so, and never that it's signed", () => {
      const done = progressSteps(game({ status: "listed" }), [address({ status: "approved" })], testServer);
      expect(states(done)).toMatchObject({ Checks: "done", Publish: "done", "In the store": "done" });
      expect(textOf(done.find((s) => s.title === "Publish")!.detail)).toBe("You published version 1.0.0.");
      expect(all(done)).not.toMatch(/\bsigned\b|\bscanned\b/i);
      const revoked = progressSteps(game(), [address({ status: "revoked", status_reason: "it asks for passwords" })], testServer);
      expect(textOf(revoked.find((s) => s.title === "Publish")!.detail)).toBe(
        "Withdrawn by SLAPorSLOP: it asks for passwords. Play opens nothing until you publish a new address: submit one on the Releases page.",
      );
    });

    // SLAPorSLOP withdrew the newest approved address: Play opens nothing, never the older one.
    const withdrawn = (over: Partial<Release> = {}) =>
      address({
        id: "r2",
        version: "1.1.0",
        status: "revoked",
        status_reason: "it asks for passwords.",
        play_address: "https://other.example.net/",
        plays_now: false,
        ...over,
      });
    const older = address({ status: "approved", plays_now: false });

    it("after the newest address is withdrawn, isn't done or in the store, and says Play opens nothing", () => {
      for (const releases of [
        [withdrawn(), older],
        // A server that doesn't say which plays: the newest decided one, withdrawn, decides.
        [withdrawn({ plays_now: undefined }), address({ status: "approved" })],
      ]) {
        const steps = progressSteps(game({ status: "listed" }), releases, testServer);
        expect(states(steps)).toMatchObject({ Publish: "problem", "In the store": "problem" });
        const publish = steps.find((s) => s.title === "Publish")!;
        expect(publish.stateLabel).toBe("Withdrawn");
        expect(textOf(publish.detail)).toBe(
          "Withdrawn by SLAPorSLOP: it asks for passwords. Play opens nothing until you publish a new address: submit one on the Releases page.",
        );
        const store = steps.find((s) => s.title === "In the store")!;
        expect(store.stateLabel).toBe("Play opens nothing");
        expect(store.detail).toBe("Players can find it, but Play opens nothing until you publish a new play address.");
        expect(currentStep(steps).title).toBe("Publish");
        expect(all(steps)).not.toContain("Players can find it.");
      }
    });

    it("a rejected or waiting address after a withdrawal says Play opens nothing meanwhile", () => {
      const rejected = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r3", version: "1.2.0", status: "rejected", status_reason: "it's another game", plays_now: false }), withdrawn(), older],
        testServer,
      );
      const checks = rejected.find((s) => s.title === "Checks")!;
      expect(checks.state).toBe("problem");
      expect(textOf(checks.detail)).toContain("Turned down by SLAPorSLOP staff: it's another game. Fix it and submit a new version, or ask SLAPorSLOP staff about it.");
      expect(textOf(rejected.find((s) => s.title === "Publish")!.detail)).toBe(
        "Version 1.1.0 was withdrawn by SLAPorSLOP: it asks for passwords. Play opens nothing until you publish a new address: submit one on the Releases page.",
      );
      expect(states(rejected)["In the store"]).toBe("problem");

      const waiting = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r3", version: "1.2.0", plays_now: false }), withdrawn(), older],
        testServer,
      );
      expect(waiting.find((s) => s.title === "Publish")!.state).toBe("waiting");
      expect(textOf(waiting.find((s) => s.title === "Publish")!.more)).toContain(
        "Play opens nothing meanwhile: SLAPorSLOP withdrew version 1.1.0 (it asks for passwords).",
      );
    });

    it("follows the address Play opens, not the newest published one", () => {
      // An older address is the one Play opens (approved after the newer one, then that was withdrawn).
      const steps = progressSteps(
        game({ status: "listed" }),
        [withdrawn({ status: "approved", status_reason: undefined }), address({ status: "approved", plays_now: true })],
        testServer,
      );
      expect(states(steps)).toMatchObject({ Publish: "done", "In the store": "done" });
      // Withdrawing an older address leaves Play on the current one: still done.
      const current = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r2", version: "1.1.0", status: "approved", plays_now: true }), address({ status: "revoked", plays_now: false })],
        testServer,
      );
      expect(states(current)).toMatchObject({ Publish: "done", "In the store": "done" });
    });

    it("Coming soon: the address is published before Beta, and Play opens it once the game is in Beta", () => {
      const none = progressSteps(game({ stage: "coming_soon" }), [], testServer);
      expect(none.find((s) => s.title === "Play address")!.detail).toBe(
        "Coming soon pages go in the store without a play address. Add it, and publish it, before you move to Beta.",
      );
      const ready = progressSteps(game({ stage: "coming_soon", status: "listed" }), [address({ status: "ready" })], testServer);
      expect(textOf(ready.find((s) => s.title === "Publish")!.detail)).toBe(
        "Version 1.0.0 passed every check. Publish it and Play opens its address once the game is in Beta.",
      );
      const draft = progressSteps(game({ stage: "coming_soon" }), [address({ status: "ready" })], testServer);
      expect(textOf(draft.find((s) => s.title === "Publish")!.detail)).toBe(
        "Version 1.0.0 passed every check. Publish it; Play opens its address once the game is in the store and in Beta.",
      );
      // A Coming soon page in the store with nothing published yet is where it should be.
      expect(states(progressSteps(game({ stage: "coming_soon", status: "listed" }), [], testServer))["In the store"]).toBe("done");
    });
  });

  describe("a dedicated server", () => {
    const repo_scan = {
      status: "checked" as const,
      licence: { declared: "MIT", files: [{ path: "LICENSE", spdx: "MIT" }], headers: [], problems: [] },
      monetization: [],
      code_loading: [{ rule: "load_native", label: "Loads native libraries at run time", count: 1, examples: ["plugins.c:4"] }],
    };
    const detail = (over: Partial<AppDetail>, scan: Partial<Release["repo_scan"] & object> = {}) =>
      textOf(
        progressSteps(app(over), [release({ repo_scan: { ...repo_scan, ...scan } })], testServer).find((s) => s.title === "Source check")!
          .detail,
      );

    it("calls its code loading a note, not a problem; a download's notes read as before", () => {
      for (const said of [detail({ tags: ["dedicated-server"] }), detail({}, { dedicated_server: true })]) {
        expect(said).toContain("1 note about the licence, selling or code loading in the app: see the release below.");
        expect(said).toContain(
          "A dedicated server may load mods, maps and plugins from other places: the Content Policy's rule on outside code doesn't cover it, so that's a note, not a problem.",
        );
      }
      expect(detail({ tags: ["puzzle"] })).toBe(
        "1 note about the licence or selling in the app: see the release below. See what to fix (with \"Copy for your AI\"), or ask your AI agent to run get_fixes.",
      );
    });

    it("linked to its game, says it's shown on the game's page, with no store page of its own", () => {
      // Linked servers count as servers whatever their tags.
      expect(detail({ tags: [], server_for: "castle" })).toContain("A dedicated server may load mods, maps and plugins");
      const draft = progressSteps(app({ status: "draft", server_for: "castle" }), [], testServer);
      expect(draft[0]).toMatchObject({ title: "Dedicated server", state: "done" });
      expect(draft[0].detail).toBe("Saved. Once it's in the store, players get it on its game's store page.");
      const listed = progressSteps(app({ status: undefined, server_for: "castle" }), [release({ status: "approved" })], testServer);
      expect(listed.at(-1)).toMatchObject({ title: "In the store", detail: "Players can get it on its game's store page." });
    });
  });

  it("words times and files plainly", () => {
    expect(fileLabel({ platform: "linux-rpm", file_name: "Game.RPM" })).toBe("Linux (.rpm)");
    expect(fileLabel({ platform: "any", file_name: "mod.zip" })).toBe("Mod (.zip)");
    expect(about(30)).toBe("under a minute");
    expect(about(299)).toBe("about 5 minutes");
    expect(about(3900)).toBe("about 1 hour 5 minutes");
    const now = Date.parse("2026-10-25T12:00:00Z");
    expect(ago(new Date(now - 20_000).toISOString(), now)).toBe("just now");
    expect(ago(new Date(now - 12 * 60_000).toISOString(), now)).toBe("12 minutes ago");
  });
});
