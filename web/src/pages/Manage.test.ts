// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { describeWaiting } from "./Manage";

describe("releases not reviewed yet", () => {
  it("say where each one is and where it came from", () => {
    expect(describeWaiting({ status: "fetching", source: "slop_build" })).toBe("Checking files, from Slop Build");
    expect(describeWaiting({ status: "awaiting_review", source: "maker" })).toBe(
      "Waiting for review, submitted from your account",
    );
    expect(describeWaiting({ status: "awaiting_review", source: "github_release" })).toBe(
      "Waiting for review, picked up from GitHub",
    );
  });

  it("a browser game's is its play address being checked", () => {
    expect(describeWaiting({ status: "fetching", source: "maker" }, true)).toBe("Checking the address, submitted from your account");
    expect(describeWaiting({ status: "awaiting_review", source: "maker" }, true)).toBe(
      "Waiting for review, submitted from your account",
    );
  });
});
