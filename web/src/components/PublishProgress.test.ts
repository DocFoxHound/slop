// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AppDetail, Release } from "../api";
import { currentStep, progressSteps } from "./PublishProgress";

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
      Review: "active",
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
      String(waiting.find((s) => s.title === "Source check")!.detail),
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
    expect(String(step.detail)).toContain("2 notes");
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
    expect(failed.find((s) => s.title === "Files checked")).toMatchObject({
      state: "problem",
      detail: "download failed: HTTP 404",
    });
    const rejected = progressSteps(
      app(),
      [release({ status: "rejected", status_reason: "loot boxes" })],
      testServer,
    );
    expect(
      String(rejected.find((s) => s.title === "Review")!.detail),
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
    expect(String(steps.find((s) => s.title === "Review")!.detail)).toContain("loot boxes");
    const only = progressSteps(app(), [release({ status: "withdrawn" })], testServer);
    expect(only.find((s) => s.title === "Release")!.state).toBe("waiting");
  });

  it("is waiting for staff once a release is approved", () => {
    expect(
      states(
        progressSteps(app(), [release({ status: "approved" })], testServer),
      ),
    ).toMatchObject({
      Review: "done",
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
    expect(String(waiting.find((s) => s.title === "Review")?.detail)).toContain(
      "first release",
    );
    const auto = progressSteps(
      app({ status: "listed" }),
      [release({ status: "approved", auto_approved: true })],
      testServer,
    );
    expect(String(auto.find((s) => s.title === "Review")?.detail)).toContain(
      "automatically",
    );
  });

  it("points at the step an app is stuck on or waiting for", () => {
    expect(currentStep(progressSteps(app(), [], testServer)).title).toBe(
      "Release",
    );
    expect(
      currentStep(progressSteps(app(), [release()], testServer)).title,
    ).toBe("Review");
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
});
