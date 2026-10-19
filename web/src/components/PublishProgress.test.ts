import { describe, expect, it } from "vitest";

import type { AppDetail, Release } from "../api";
import { currentStep, progressSteps } from "./PublishProgress";

const app = (over: Partial<AppDetail> = {}) =>
  ({ slug: "game", stage: "beta", status: "draft", ...over }) as AppDetail;

const file = (over: Partial<Release["files"][number]> = {}): Release["files"][number] => ({
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
  scan_result: "skipped",
  scan_detail: "not scanned: scanning is off on this server",
  ...over,
});

const release = (over: Partial<Release> = {}): Release => ({
  id: "r1",
  version: "1.0.0",
  commit_sha: "a".repeat(40),
  status: "awaiting_review",
  commit_verified: false,
  created_at: "2026-10-18T00:00:00Z",
  files: [file()],
  ...over,
});

const testServer = { scanning: false, maker_signatures_required: false };
const states = (steps: ReturnType<typeof progressSteps>) => Object.fromEntries(steps.map((s) => [s.title, s.state]));

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
      Review: "active",
      "In the store": "waiting",
    });
  });

  it("flags a missing signature when signatures are required", () => {
    const steps = progressSteps(app(), [release()], { scanning: false, maker_signatures_required: true });
    const signature = steps.find((s) => s.title === "Maker signature")!;
    expect(signature.state).toBe("problem");
    expect(signature.detail).toContain("no build attestation");
  });

  it("shows failed checks and rejections with their reasons", () => {
    const failed = progressSteps(app(), [release({ status: "failed", status_reason: "download failed: HTTP 404" })], testServer);
    expect(failed.find((s) => s.title === "Files checked")).toMatchObject({ state: "problem", detail: "download failed: HTTP 404" });
    const rejected = progressSteps(app(), [release({ status: "rejected", status_reason: "loot boxes" })], testServer);
    expect(String(rejected.find((s) => s.title === "Review")!.detail)).toContain("loot boxes");
  });

  it("is waiting for staff once a release is approved", () => {
    expect(states(progressSteps(app(), [release({ status: "approved" })], testServer))).toMatchObject({
      Review: "done",
      "In the store": "active",
    });
    expect(states(progressSteps(app({ status: "listed" }), [release({ status: "approved" })], testServer))).toMatchObject({
      "In the store": "done",
    });
    // Listed apps come back from the API without a status field.
    expect(states(progressSteps(app({ status: undefined }), [release({ status: "approved" })], testServer))).toMatchObject({
      "In the store": "done",
    });
  });

  it("needs no release for a Coming soon page", () => {
    expect(states(progressSteps(app({ stage: "coming_soon" }), [], testServer))).toEqual({
      "Store page": "done",
      Release: "skipped",
      "In the store": "active",
    });
  });

  it("points at the step an app is stuck on or waiting for", () => {
    expect(currentStep(progressSteps(app(), [], testServer)).title).toBe("Release");
    expect(currentStep(progressSteps(app(), [release()], testServer)).title).toBe("Review");
    const failed = progressSteps(app(), [release({ status: "failed", status_reason: "HTTP 404" })], testServer);
    expect(currentStep(failed)).toMatchObject({ title: "Files checked", state: "problem" });
    const listed = progressSteps(app({ status: "listed" }), [release({ status: "approved" })], testServer);
    expect(currentStep(listed)).toMatchObject({ title: "In the store", state: "done" });
  });
});
