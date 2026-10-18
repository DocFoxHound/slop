import { describe, expect, it } from "vitest";

import { query } from "./api";
import { artFor, deadlineText } from "./format";

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
