// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { GithubRelease } from "../api";
import { autoApprovalStatus, fitsPlatform, nextVersion, pickRows, statusText } from "./Releases";

const release = (names: string[]): GithubRelease => ({
  tag: "v0.1.0",
  name: null,
  version: "0.1.0",
  published_at: null,
  prerelease: false,
  html_url: "https://github.com/me/game/releases/tag/v0.1.0",
  submitted: false,
  assets: names.map((name) => ({
    name,
    url: `https://github.com/me/game/releases/download/v0.1.0/${name}`,
    size: 1,
    platform: null,
    kind: null,
  })),
});

describe("release picking", () => {
  it("knows each platform's file type", () => {
    expect(fitsPlatform("windows", "Game-windows.zip")).toBe(true);
    expect(fitsPlatform("windows", "Game-linux.tar.gz")).toBe(false);
    expect(fitsPlatform("linux", "Game-linux-x86_64.tar.gz")).toBe(true);
    expect(fitsPlatform("linux", "game.tgz")).toBe(true);
    expect(fitsPlatform("linux-deb", "game_0.1.0_amd64.deb")).toBe(true);
    expect(fitsPlatform("linux-rpm", "game_0.1.0_amd64.deb")).toBe(false);
  });

  it("ticks one file per platform the app runs on", () => {
    const rows = pickRows(
      release(["Game-windows.zip", "Game-linux.tar.gz", "game.deb", "checksums.txt", "Game-extra.zip"]),
      ["windows", "linux", "linux-deb"],
    );
    expect(rows.filter((r) => r.include).map((r) => [r.name, r.platform])).toEqual([
      ["Game-windows.zip", "windows"],
      ["Game-linux.tar.gz", "linux"],
      ["game.deb", "linux-deb"],
    ]);
  });

  it("leaves out builds for platforms the app doesn't list", () => {
    const rows = pickRows(release(["Game-windows.zip", "Game-linux.tar.gz"]), ["windows"]);
    expect(rows.filter((r) => r.include).map((r) => r.name)).toEqual(["Game-windows.zip"]);
  });
});

describe("a browser game's releases", () => {
  it("suggests the next version, never one already used", () => {
    expect(nextVersion([])).toBe("1.0.0");
    expect(nextVersion(["1.0.0"])).toBe("1.0.1");
    expect(nextVersion(["1.0.9", "1.0.8"])).toBe("1.0.10");
    expect(nextVersion(["v2"])).toBe("v3");
    expect(nextVersion(["2.0-beta3"])).toBe("2.0-beta4");
    // Versions stay unique whatever became of a release (a withdrawn 1.0.1 still counts).
    expect(nextVersion(["1.0.0", "1.0.1"])).toBe("1.0.2");
    expect(nextVersion(["1.0.0", "1.0.2", "1.0.1"])).toBe("1.0.3");
    expect(nextVersion(["beta"])).toBe("beta.1");
    expect(nextVersion(["x".repeat(40)])).toBe("");
  });

  it("words a play address's status without signing, scanning or files", () => {
    const statuses = ["fetching", "awaiting_review", "approved", "rejected", "failed", "revoked", "withdrawn"] as const;
    for (const s of statuses) {
      const t = statusText(s, true);
      expect(`${t.label} ${t.help}`).not.toMatch(/sign|scan|files|install/i);
    }
    expect(statusText("fetching", true).label).toBe("CHECKING THE ADDRESS");
    expect(statusText("approved", true).help).toBe("Approved: Play opens this address.");
    expect(statusText("revoked", true).help).toContain("Play no longer opens it");
    // Downloads keep theirs.
    expect(statusText("fetching", false).label).toBe("CHECKING FILES");
    expect(statusText("approved", false).help).toContain("Signed by SLAPorSLOP");
  });

  it("Coming soon, an approved address opens once the game is in Beta", () => {
    expect(statusText("approved", true, true)).toMatchObject({
      label: "APPROVED",
      help: "Approved: Play opens this address once the game is in Beta.",
    });
    expect(statusText("approved", false, true).help).toContain("Signed by SLAPorSLOP");
  });

  it("never promises a new address is approved automatically", () => {
    const ready = { auto_approval_ready: true, auto_approval_note: null };
    expect(autoApprovalStatus(ready, true)).toBe(
      "Your next release will be approved automatically if it keeps the address Play opens now and passes every check; a new address always waits for a person.",
    );
    expect(autoApprovalStatus(ready, false)).toBe("Your next release will be approved automatically if it passes every check.");
    expect(autoApprovalStatus({ auto_approval_ready: false, auto_approval_note: "automatic updates are off" }, true)).toBe(
      "Your next release waits for a person: automatic updates are off.",
    );
  });
});
