// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { addressCheckItems, addressChecksLine, isNewSite, NO_LISTS } from "./addressChecks";
import type { AddressChecks } from "./api";

const checks = (over: Partial<AddressChecks> = {}): AddressChecks => ({
  https: true,
  public_host: true,
  bad_links: { listed_by: null, lists_loaded: true },
  status: 200,
  final_address: "https://games.example.com/play/",
  redirects: 0,
  content_type: "text/html; charset=utf-8",
  checked_at: "2026-11-07T10:00:00Z",
  ok: true,
  problem: null,
  ...over,
});

const states = (c: AddressChecks | null, status?: "fetching" | "awaiting_review" | "failed") =>
  addressCheckItems(c, status).map((i) => [i.key, i.state, i.status]);

describe("a play address's checks in words", () => {
  it("says each check passed, one line each, in order", () => {
    expect(states(checks())).toEqual([
      ["https", "done", "Yes"],
      ["public", "done", "Yes"],
      ["bad_links", "done", "Not on them"],
      ["page", "done", "Yes (HTTP 200)"],
    ]);
    expect(addressCheckItems(checks())[3].detail).toBe("A text/html page");
    expect(addressChecksLine(addressCheckItems(checks()))).toBe(
      "✓ https: Yes · ✓ A public site: Yes · ✓ Bad-link lists: Not on them · ✓ The page loads: Yes (HTTP 200)",
    );
  });

  it("while the address is being checked, every line is under way", () => {
    for (const items of [addressCheckItems(null), addressCheckItems(checks(), "fetching")]) {
      expect(items.map((i) => [i.state, i.status])).toEqual(Array(4).fill(["active", "Checking"]));
    }
  });

  it("never claims a check a test setup skips, or a bad-link list that isn't loaded", () => {
    const test = addressCheckItems(checks({ https: false, public_host: null }));
    expect(test.map((i) => [i.key, i.state, i.status])).toEqual([
      ["https", "skipped", "No: plain http"],
      ["public", "skipped", "Not checked"],
      ["bad_links", "done", "Not on them"],
      ["page", "done", "Yes (HTTP 200)"],
    ]);
    const none = addressCheckItems(checks({ bad_links: { listed_by: null, lists_loaded: false } }))[2];
    expect(none).toMatchObject({ state: "skipped", status: "Not checked", detail: NO_LISTS });
    expect(NO_LISTS).toContain("No bad-link lists are loaded on this server");
    expect(JSON.stringify(none)).not.toMatch(/not on any/i);
  });

  it("names the list an address is on, and why a page didn't pass", () => {
    const listed = addressCheckItems(checks({ ok: false, bad_links: { listed_by: "URLhaus", lists_loaded: true }, problem: "on URLhaus" }));
    expect(listed[2]).toMatchObject({ state: "problem", status: "On URLhaus" });
    // The page itself loaded: that line still says so.
    expect(listed[3]).toMatchObject({ state: "done", status: "Yes (HTTP 200)" });

    const missing = addressCheckItems(checks({ ok: false, status: 404, problem: "the page didn't load: the site answered HTTP 404" }));
    expect(missing[3]).toMatchObject({
      state: "problem",
      status: "No (HTTP 404)",
      detail: "the page didn't load: the site answered HTTP 404",
    });
    const download = addressCheckItems(checks({ ok: false, problem: "it downloads a file instead of opening a page" }))[3];
    expect(download).toMatchObject({ state: "problem", status: "No", detail: "it downloads a file instead of opening a page" });
    const silent = addressCheckItems(checks({ ok: false, status: null, problem: "the site didn't answer" }))[3];
    expect(silent).toMatchObject({ state: "problem", status: "No answer" });
    const private_ = addressCheckItems(checks({ ok: false, public_host: false, status: null, problem: "it leads to a private network" }));
    expect(private_[1]).toMatchObject({ state: "problem", status: "No: it's on a private network" });
  });

  it("says where a redirect ended", () => {
    const moved = addressCheckItems(checks({ redirects: 2, final_address: "https://cdn.example.com/game/" }))[3];
    expect(moved.detail).toBe("After 2 redirects, at https://cdn.example.com/game/; a text/html page");
  });

  it("knows a new site from the same one", () => {
    expect(isNewSite("https://games.example.com/v2/", "https://games.example.com/play/")).toBe(false);
    expect(isNewSite("https://other.example.net/", "https://games.example.com/play/")).toBe(true);
    // A first address isn't a "new site": there's nothing to compare it with.
    expect(isNewSite("https://games.example.com/", null)).toBe(false);
  });
});
