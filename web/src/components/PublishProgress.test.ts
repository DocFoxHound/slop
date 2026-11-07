// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { isValidElement, type ReactNode } from "react";
import type { AppDetail, Release } from "../api";
import { about, ago, currentStep, fileLabel, progressSteps, refreshEvery } from "./PublishProgress";

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

  it("shows a release waiting for review on a test server", () => {
    expect(states(progressSteps(app(), [release()], testServer))).toEqual({
      "Store page": "done",
      Release: "done",
      "Files checked": "done",
      "Maker signature": "skipped",
      Scan: "skipped",
      "Source check": "done",
      "Review by a person": "active",
      "In the store": "waiting",
    });
  });

  it("flags source-check notes until a person has looked", () => {
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
    const waiting = progressSteps(app(), [release({ repo_scan })], testServer);
    expect(waiting.find((s) => s.title === "Source check")).toMatchObject({
      state: "problem",
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
    expect(step.state).toBe("problem");
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
      textOf(rejected.find((s) => s.title === "Review by a person")!.detail),
    ).toContain("loot boxes");
  });

  it("leaves out a release withdrawn before review", () => {
    const steps = progressSteps(
      app(),
      [
        release({ id: "r2", version: "1.1.0", status: "withdrawn" }),
        release({ status: "rejected", status_reason: "loot boxes" }),
      ],
      testServer,
    );
    expect(steps.find((s) => s.title === "Release")!.detail).toBe("Version 1.0.0 received.");
    expect(textOf(steps.find((s) => s.title === "Review by a person")!.detail)).toContain("loot boxes");
    const only = progressSteps(app(), [release({ status: "withdrawn" })], testServer);
    expect(only.find((s) => s.title === "Release")!.state).toBe("waiting");
  });

  it("is waiting for staff once a release is approved", () => {
    expect(
      states(
        progressSteps(app(), [release({ status: "approved" })], testServer),
      ),
    ).toMatchObject({
      "Review by a person": "done",
      "In the store": "active",
    });
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

  it("says why a release waits for a person, and when SLAPorSLOP approved it", () => {
    const waiting = progressSteps(
      app(),
      [
        release({
          status_reason:
            "Waiting for a person: an app's first release is always reviewed by a person",
        }),
      ],
      testServer,
    );
    const review = waiting.find((s) => s.title === "Review by a person")!;
    expect(textOf(review.more)).toContain("Why a person: an app's first release is always reviewed by a person.");
    expect(textOf(review.detail)).toBe("Waiting for review by a person at SLAPorSLOP. It usually takes a day or two.");
    expect(textOf(review.more)).toContain("you get a notification");
    expect(review.stateLabel).toBe("Waiting for a person");
    const auto = progressSteps(
      app({ status: "listed" }),
      [release({ status: "approved", auto_approved: true })],
      testServer,
    );
    expect(textOf(auto.find((s) => s.title === "Review by a person")?.detail)).toContain(
      "automatically",
    );
  });

  it("points at the step an app is stuck on or waiting for", () => {
    expect(currentStep(progressSteps(app(), [], testServer)).title).toBe(
      "Release",
    );
    expect(
      currentStep(progressSteps(app(), [release()], testServer)).title,
    ).toBe("Review by a person");
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
    expect(states(steps)).toMatchObject({ Scan: "waiting", "Source check": "waiting", "Review by a person": "waiting" });
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
    expect(textOf(scan.more)).toContain("a person at SLAPorSLOP looks");
    expect(states(steps)["Maker signature"]).toBe("done");
    expect(textOf(steps.find((s) => s.title === "Review by a person")!.detail)).toContain("looks at the file on hold first");
  });

  it("refreshes often while files are checked, now and then during review, and stops when done", () => {
    expect(refreshEvery([release({ status: "fetching" })])).toBe(10_000);
    expect(refreshEvery([release({ status: "awaiting_review" })])).toBe(60_000);
    // Fetched, but SLAPorSLOP hasn't read the source yet: that shows within seconds.
    expect(refreshEvery([release({ status: "awaiting_review", repo_scan: null })])).toBe(10_000);
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
      expect(none.find((s) => s.title === "In the store")!.detail).toBe("After the play address is approved.");
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
        "Review by a person": "active",
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
      expect(textOf(steps.find((s) => s.title === "Review by a person")!.more)).toContain("Play opens this address");
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

    it("once approved, says Play opens it, and never that it's signed", () => {
      const done = progressSteps(game({ status: "listed" }), [address({ status: "approved" })], testServer);
      expect(states(done)).toMatchObject({ "Review by a person": "done", "In the store": "done" });
      expect(textOf(done.find((s) => s.title === "Review by a person")!.detail)).toBe("Version 1.0.0 approved by a person at SLAPorSLOP.");
      expect(all(done)).not.toMatch(/\bsigned\b|\bscanned\b/i);
      const auto = progressSteps(game(), [address({ status: "approved", auto_approved: true })], testServer);
      expect(textOf(auto.find((s) => s.title === "Review by a person")!.detail)).toContain("a person approved this address before");
      const revoked = progressSteps(game(), [address({ status: "revoked", status_reason: "it asks for passwords" })], testServer);
      expect(textOf(revoked.find((s) => s.title === "Review by a person")!.detail)).toBe(
        "Withdrawn by SLAPorSLOP: it asks for passwords. Play opens nothing until a person approves a new address: submit one on the Releases page.",
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
        expect(states(steps)).toMatchObject({ "Review by a person": "problem", "In the store": "problem" });
        const review = steps.find((s) => s.title === "Review by a person")!;
        expect(review.stateLabel).toBe("Withdrawn");
        expect(textOf(review.detail)).toBe(
          "Withdrawn by SLAPorSLOP: it asks for passwords. Play opens nothing until a person approves a new address: submit one on the Releases page.",
        );
        const store = steps.find((s) => s.title === "In the store")!;
        expect(store.stateLabel).toBe("Play opens nothing");
        expect(store.detail).toBe("Players can find it, but Play opens nothing until a person at SLAPorSLOP approves a new play address.");
        expect(currentStep(steps).title).toBe("Review by a person");
        expect(all(steps)).not.toContain("Players can find it.");
      }
    });

    it("a rejected or waiting address after a withdrawal says Play opens nothing meanwhile", () => {
      const rejected = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r3", version: "1.2.0", status: "rejected", status_reason: "it's another game", plays_now: false }), withdrawn(), older],
        testServer,
      );
      const review = rejected.find((s) => s.title === "Review by a person")!;
      expect(review.state).toBe("problem");
      expect(textOf(review.detail)).toContain(
        "Not approved: it's another game. Fix it and submit the address again with a new version. Play opens nothing meanwhile: SLAPorSLOP withdrew version 1.1.0 (it asks for passwords).",
      );
      expect(states(rejected)["In the store"]).toBe("problem");

      const waiting = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r3", version: "1.2.0", plays_now: false }), withdrawn(), older],
        testServer,
      );
      expect(textOf(waiting.find((s) => s.title === "Review by a person")!.more)).toContain(
        "Play opens nothing meanwhile: SLAPorSLOP withdrew version 1.1.0 (it asks for passwords).",
      );
    });

    it("follows the address Play opens, not the newest approved one", () => {
      // An older address is the one Play opens (approved after the newer one, then that was withdrawn).
      const steps = progressSteps(
        game({ status: "listed" }),
        [withdrawn({ status: "approved", status_reason: undefined }), address({ status: "approved", plays_now: true })],
        testServer,
      );
      expect(states(steps)).toMatchObject({ "Review by a person": "done", "In the store": "done" });
      // Withdrawing an older address leaves Play on the current one: still done.
      const current = progressSteps(
        game({ status: "listed" }),
        [address({ id: "r2", version: "1.1.0", status: "approved", plays_now: true }), address({ status: "revoked", plays_now: false })],
        testServer,
      );
      expect(states(current)).toMatchObject({ "Review by a person": "done", "In the store": "done" });
    });

    it("Coming soon: the address is approved before Beta, and Play opens it once the game is in Beta", () => {
      const none = progressSteps(game({ stage: "coming_soon" }), [], testServer);
      expect(none.find((s) => s.title === "Play address")!.detail).toBe(
        "Coming soon pages go in the store without a play address. Add it, and have it approved, before you move to Beta.",
      );
      const waiting = progressSteps(game({ stage: "coming_soon" }), [address()], testServer);
      expect(textOf(waiting.find((s) => s.title === "Review by a person")!.more)).toContain(
        "if it's approved, Play opens this address once the game is in Beta;",
      );
      // A Coming soon page in the store with nothing approved yet is where it should be.
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
        expect(said).toContain("1 note about the licence, selling or code loading in the app, for you and the reviewer");
        expect(said).toContain(
          "A dedicated server may load mods, maps and plugins from other places: the Content Policy's rule on outside code doesn't cover it, so that's a note, not a problem.",
        );
      }
      expect(detail({ tags: ["puzzle"] })).toBe(
        "1 note about the licence or selling in the app, for you and the reviewer: see the release below. See what to fix (with \"Copy for your AI\"), or ask your AI agent to run get_fixes.",
      );
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
