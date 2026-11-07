// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { addressCheckItems, addressChecksLine, compareAddress, NO_LISTS } from "./addressChecks";
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

  it("says a site's name couldn't be looked up, never that a test setup skipped it, on a real server", () => {
    const lookup = addressCheckItems(
      checks({
        ok: false,
        public_host: null,
        public_host_skipped: "lookup_failed",
        status: null,
        final_address: null,
        problem: "SLAPorSLOP can't find the site no-such-site.invalid (failed to lookup address information)",
      }),
    );
    expect(lookup[1]).toMatchObject({ state: "problem", status: "No: the site's name couldn't be looked up" });
    expect(JSON.stringify(lookup)).not.toContain("test setup");
    expect(lookup[3]).toMatchObject({ state: "problem", status: "No answer", detail: expect.stringContaining("can't find the site") });
    // A test setup says so, whether its page loaded or not.
    for (const ok of [true, false]) {
      expect(addressCheckItems(checks({ ok, public_host: null, public_host_skipped: "test_setup", status: ok ? 200 : 404 }))[1]).toMatchObject({
        state: "skipped",
        status: "Not checked",
        detail: "A test setup of SLAPorSLOP doesn't look up where a site is.",
      });
    }
    // An older check that doesn't say why, and didn't pass, claims neither.
    expect(addressCheckItems(checks({ ok: false, public_host: null, status: null }))[1]).toEqual({
      key: "public",
      label: "A public site",
      state: "skipped",
      status: "Not known",
    });
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

  it("knows a new site, the same host and the same address apart", () => {
    expect(compareAddress("https://other.example.net/", "https://games.example.com/play/")).toBe("new_site");
    // Another page on the same host is never "the same site": on a shared host it can be another
    // person's game.
    expect(compareAddress("https://games.example.com/v2/", "https://games.example.com/play/")).toBe("same_host");
    expect(compareAddress("https://html.itch.zone/html/222/", "https://html.itch.zone/html/111/")).toBe("same_host");
    expect(compareAddress("https://games.example.com/play/?level=2", "https://games.example.com/play/")).toBe("same_host");
    expect(compareAddress("https://games.example.com/play/#b", "https://games.example.com/play/#a")).toBe("same_host");
    expect(compareAddress("https://games.example.com/play/", "https://games.example.com/play/")).toBe("same_address");
    // Hosts as the server stores them: case and a dot at the end don't matter; a port does.
    expect(compareAddress("https://Games.Example.com./play/", "https://games.example.com/play/")).toBe("same_address");
    expect(compareAddress("https://games.example.com:8443/play/", "https://games.example.com/play/")).toBe("new_site");
    // With nothing to compare it with, there's no answer.
    expect(compareAddress("https://games.example.com/", null)).toBeNull();
    expect(compareAddress("not an address", "https://games.example.com/")).toBeNull();
  });
});
