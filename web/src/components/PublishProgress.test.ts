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

  it("counts code from outside Slop and undeclared online play as notes", () => {
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
    expect(textOf(files.detail)).toContain("Slop couldn't check the files: download failed: HTTP 404.");
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

  it("says why a release waits for a person, and when Slop approved it", () => {
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
    expect(textOf(review.detail)).toBe("Waiting for review by a person at Slop. It usually takes a day or two.");
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
  it("shows each file while Slop checks them, with an honest estimate", () => {
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
    expect(files.detail).toContain("Slop checks one release at a time.");
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
    expect(textOf(scan.more)).toContain("a person at Slop looks");
    expect(states(steps)["Maker signature"]).toBe("done");
    expect(textOf(steps.find((s) => s.title === "Review by a person")!.detail)).toContain("looks at the file on hold first");
  });

  it("refreshes often while files are checked, now and then during review, and stops when done", () => {
    expect(refreshEvery([release({ status: "fetching" })])).toBe(10_000);
    expect(refreshEvery([release({ status: "awaiting_review" })])).toBe(60_000);
    // Fetched, but Slop hasn't read the source yet: that shows within seconds.
    expect(refreshEvery([release({ status: "awaiting_review", repo_scan: null })])).toBe(10_000);
    expect(refreshEvery([release({ status: "approved" })])).toBeNull();
    expect(refreshEvery([release({ status: "withdrawn" }), release({ status: "rejected" })])).toBeNull();
    expect(refreshEvery([])).toBeNull();
    expect(refreshEvery(undefined)).toBeNull();
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
