// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { query } from "./api";
import { accountId, addressHost, artFor, dayGroup, deadlineText, money, playsInBrowser, timeAgo, toMinor } from "./format";
import { canOpenPlayAddress } from "./platform";

const now = Date.parse("2026-10-18T12:00:00Z");

describe("deadlineText", () => {
  it("counts down to beta", () => {
    expect(deadlineText({ stage: "coming_soon", stage_deadline: "2026-10-30T12:00:00Z" }, now)).toBe("Beta due in 12 days");
    expect(deadlineText({ stage: "coming_soon", stage_deadline: "2026-10-19T12:00:00Z" }, now)).toBe("Beta due in 1 day");
  });

  it("reports missed deadlines", () => {
    expect(deadlineText({ stage: "coming_soon", stage_deadline: "2026-10-01T00:00:00Z" }, now)).toBe("Missed its beta deadline");
    expect(deadlineText({ stage: "beta", stage_deadline: "2026-10-01T00:00:00Z" }, now)).toBe("Missed its release deadline");
  });

  it("has nothing to say once released", () => {
    expect(deadlineText({ stage: "released", stage_deadline: null }, now)).toBeNull();
  });
});

describe("artFor", () => {
  it("is stable per slug", () => {
    expect(artFor("pixel-dungeon")).toEqual(artFor("pixel-dungeon"));
    expect(artFor("pixel-dungeon")).not.toEqual(artFor("tidy-notes"));
  });
});

describe("query", () => {
  it("drops empty values and encodes the rest", () => {
    expect(query({ q: "a b&c", tag: undefined, stage: "", limit: 2 })).toBe("?q=a+b%26c&limit=2");
    expect(query({})).toBe("");
  });
});

describe("money", () => {
  it("formats like the server", () => {
    expect(money(12001, "usd")).toBe("$120.01");
    expect(money(12000, "usd")).toBe("$120");
    expect(money(1234500, "gbp")).toBe("£12,345");
    expect(money(950, "eur")).toBe("€9.50");
  });
  it("reads amounts people type", () => {
    expect(toMinor("120")).toBe(12000);
    expect(toMinor("$1,200.5")).toBe(120050);
    expect(toMinor("12.345")).toBeNull();
    expect(toMinor("ten")).toBeNull();
  });
});

describe("accountId", () => {
  const id = "01a16951-24a3-735a-aec1-8a29b00986cd";
  it("takes an ID or a link to the profile", () => {
    expect(accountId(id)).toBe(id);
    expect(accountId(`  ${id.toUpperCase()} `)).toBe(id);
    expect(accountId(`https://slop.example/u/${id}`)).toBe(id);
    expect(accountId(`/u/${id}?tab=apps`)).toBe(id);
  });

  it("refuses anything else", () => {
    expect(accountId("")).toBeUndefined();
    expect(accountId("Olly Overdue")).toBeUndefined();
    expect(accountId("https://slop.example/people/vera-pixels")).toBeUndefined();
    expect(accountId(id.slice(0, -1))).toBeUndefined();
  });
});

describe("timeAgo", () => {
  const at = (minutesAgo: number) => new Date(now - minutesAgo * 60_000).toISOString();
  it("says how long ago, then the date", () => {
    expect(timeAgo(at(0), now)).toBe("just now");
    expect(timeAgo(at(-5), now)).toBe("just now");
    expect(timeAgo(at(5), now)).toBe("5 min ago");
    expect(timeAgo(at(59), now)).toBe("59 min ago");
    expect(timeAgo(at(180), now)).toBe("3 h ago");
    expect(timeAgo(at(60 * 24), now)).toBe("1 day ago");
    expect(timeAgo(at(60 * 24 * 6), now)).toBe("6 days ago");
    expect(timeAgo("2026-10-03T12:00:00Z", now)).toBe(new Date("2026-10-03T12:00:00Z").toLocaleDateString(undefined, { month: "short", day: "numeric" }));
    expect(timeAgo("2025-10-03T12:00:00Z", now)).toMatch(/2025/);
  });
});

describe("dayGroup", () => {
  // Local times, as the viewer's calendar has them.
  const local = (d: number, h: number) => new Date(2026, 9, d, h).getTime();
  const iso = (t: number) => new Date(t).toISOString();
  it("groups by calendar day", () => {
    const noon = local(18, 12);
    expect(dayGroup(iso(local(18, 0)), noon)).toBe("Today");
    expect(dayGroup(iso(local(18, 13)), noon)).toBe("Today");
    expect(dayGroup(iso(local(17, 23)), noon)).toBe("Yesterday");
    expect(dayGroup(iso(local(17, 0)), noon)).toBe("Yesterday");
    expect(dayGroup(iso(local(16, 23)), noon)).toBe("This week");
    expect(dayGroup(iso(local(12, 0)), noon)).toBe("This week");
    expect(dayGroup(iso(local(11, 23)), noon)).toBe("Earlier");
    expect(dayGroup("2025-01-01T00:00:00Z", noon)).toBe("Earlier");
  });
});

describe("games that play in the browser", () => {
  it("are told apart by the summary's flag or the store page's platforms", () => {
    expect(playsInBrowser({ plays_in_browser: true })).toBe(true);
    expect(playsInBrowser({ plays_in_browser: false, platforms: ["browser"] })).toBe(true);
    expect(playsInBrowser({ plays_in_browser: false, platforms: ["windows", "linux"] })).toBe(false);
    expect(playsInBrowser(undefined)).toBe(false);
  });

  it("name the site Play opens, and nothing for what isn't an address", () => {
    expect(addressHost("https://play.example.com/orbit/?level=2#top")).toBe("play.example.com");
    expect(addressHost("http://127.0.0.1:4000/e2e/orbit-tap/play/")).toBe("127.0.0.1:4000");
    expect(addressHost("not an address")).toBe("");
    expect(addressHost(null)).toBe("");
  });

  it("open only web addresses: https in the SLAPorSLOP app, http too on the website (test servers)", () => {
    expect(canOpenPlayAddress("https://play.example.com/", true)).toBe(true);
    expect(canOpenPlayAddress("https://play.example.com/", false)).toBe(true);
    expect(canOpenPlayAddress("http://127.0.0.1:4000/play/", false)).toBe(true);
    expect(canOpenPlayAddress("http://127.0.0.1:4000/play/", true)).toBe(false);
    for (const bad of ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,hi", "ftp://example.com/", "play.example.com", "", null])
      for (const desktop of [true, false]) expect(canOpenPlayAddress(bad, desktop), `${bad} ${desktop}`).toBe(false);
  });
});
