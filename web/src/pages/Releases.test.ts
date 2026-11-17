// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { GithubRelease } from "../api";
import { fitsPlatform, newerOut, nextVersion, pickRows, publishNotes, publishWait, statusText } from "./Releases";

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
    const statuses = ["fetching", "awaiting_review", "ready", "approved", "rejected", "failed", "revoked", "withdrawn"] as const;
    for (const s of statuses) {
      const t = statusText(s, true);
      expect(`${t.label} ${t.help}`).not.toMatch(/sign|scan|files|install/i);
    }
    expect(statusText("fetching", true).label).toBe("CHECKING THE ADDRESS");
    expect(statusText("approved", true).help).toBe("Published: Play opens this address.");
    expect(statusText("ready", true)).toMatchObject({
      label: "PASSED, NOT PUBLISHED",
      help: "It passed the checks. Play opens this address once you publish it.",
    });
    expect(statusText("revoked", true).help).toContain("Play no longer opens it");
    // Downloads keep theirs.
    expect(statusText("fetching", false).label).toBe("CHECKING FILES");
    expect(statusText("approved", false).help).toContain("Signed by SLAPorSLOP when you published it");
  });

  it("names the new statuses: on hold, and passed but not published", () => {
    expect(statusText("awaiting_review", false)).toMatchObject({ label: "ON HOLD", badge: "warn" });
    expect(statusText("ready", false)).toMatchObject({
      label: "PASSED, NOT PUBLISHED",
      help: "It passed every check. Players get it once you publish it.",
    });
    expect(statusText("approved", false).label).toBe("PUBLISHED");
    expect(statusText("withdrawn", false).label).toBe("WITHDRAWN BEFORE PUBLISHING");
    // Nothing says a person reviews it, or that it's approved automatically.
    for (const s of ["fetching", "awaiting_review", "ready", "approved", "rejected", "failed", "revoked", "withdrawn"] as const) {
      for (const browser of [false, true]) {
        const t = statusText(s, browser);
        expect(`${t.label} ${t.help}`).not.toMatch(/a person|automatically|in review/i);
      }
    }
  });

  it("Coming soon, a published address opens once the game is in Beta", () => {
    expect(statusText("approved", true, true)).toMatchObject({
      label: "PUBLISHED",
      help: "Published: Play opens this address once the game is in Beta.",
    });
    expect(statusText("ready", true, true).help).toBe(
      "It passed the checks. Once you publish it, Play opens this address when the game is in Beta.",
    );
    expect(statusText("approved", false, true).help).toContain("Signed by SLAPorSLOP");
  });
});

describe("publishing a release that passed", () => {
  const checked = { status: "checked" as const };
  it("waits for new pictures, and for the source check to have a result, with the reason", () => {
    expect(publishWait({ repo_scan: checked }, false)).toBeUndefined();
    // "Couldn't read it" is a result too: it doesn't wait.
    expect(publishWait({ repo_scan: { status: "not_checked", detail: "the host didn't answer" } }, false)).toBeUndefined();
    expect(publishWait({ repo_scan: checked }, true)).toBe(
      "Its new pictures are waiting for review. You can publish once they're reviewed.",
    );
    expect(publishWait({ repo_scan: null }, false)).toBe(
      "SLAPorSLOP hasn't read this release's source yet. It tries again every hour, and you can publish once it has a result.",
    );
    expect(publishWait({ repo_scan: { status: "failed", detail: "timeout" } }, false)).toContain("hasn't read this release's source");
  });

  it("offers no Publish for a release older than one already out (Publish refuses it)", () => {
    const at = (h: number) => `2026-11-07T${String(h).padStart(2, "0")}:00:00Z`;
    const ready = { id: "a", version: "1.0", status: "ready" as const, created_at: at(10) };
    expect(newerOut(ready, [ready])).toBeUndefined();
    // An older published release, or a newer one not out yet, doesn't stop it.
    expect(newerOut(ready, [{ id: "b", version: "0.9", status: "approved", created_at: at(9) }, ready])).toBeUndefined();
    expect(newerOut(ready, [{ id: "c", version: "1.1", status: "awaiting_review", created_at: at(11) }, ready])).toBeUndefined();
    // A newer one published (or withdrawn by SLAPorSLOP since) does.
    expect(newerOut(ready, [{ id: "d", version: "1.1", status: "approved", created_at: at(11) }, ready])?.version).toBe("1.1");
    expect(newerOut(ready, [{ id: "e", version: "1.2", status: "revoked", created_at: at(12) }, ready])?.version).toBe("1.2");
  });

  it("lists what to know first as information: the hold it ends, a new site, what the source check found", () => {
    const r = { version: "1.4", new_concerns: undefined, new_play_address: undefined, play_address: null };
    expect(publishNotes(r, {})).toEqual([]);
    expect(publishNotes(r, { heldAt: "1.2" })).toEqual(["Players are held at 1.2. Publishing 1.4 ends the hold."]);
    expect(
      publishNotes({ ...r, new_concerns: "the source check found code loading that 1.3 didn't have" }, {}),
    ).toEqual(["The source check found code loading that 1.3 didn't have. See the source check on this release."]);
    expect(
      publishNotes({ ...r, new_play_address: true, play_address: "https://new.example.net/" }, { playsNow: "https://old.example.com/" }),
    ).toEqual(["A new site: Play will open https://new.example.net/ instead of https://old.example.com/."]);
    expect(publishNotes({ ...r, new_play_address: true, play_address: "https://new.example.net/" }, { playsNow: null })).toEqual([
      "A new address: Play will open https://new.example.net/.",
    ]);
  });
});
