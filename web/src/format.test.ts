// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { query } from "./api";
import { accountId, artFor, deadlineText, money, toMinor } from "./format";

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
