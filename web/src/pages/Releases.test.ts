// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { GithubRelease } from "../api";
import { fitsPlatform, pickRows } from "./Releases";

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
