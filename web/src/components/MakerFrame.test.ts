// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { advanceHint, canPutInStore, moveEffects, releaseDateNote } from "./MakerFrame";

describe("moveEffects", () => {
  it("says who's told and that it's one way", () => {
    const beta = moveEffects("beta", 3);
    expect(beta).toContain("The 3 people who marked it Interested get a notice.");
    expect(beta).toContain("It has a year to be released.");
    expect(beta.at(-1)).toMatch(/^Only forward/);
    expect(moveEffects("released", 1)).toContain("The 1 person who marked it Interested gets a notice.");
  });
  it("with no one Interested yet, says what happens later", () => {
    expect(moveEffects("released", 0)[1]).toMatch(/^Anyone who marks it Interested/);
  });
});

describe("advanceHint", () => {
  it("says what moving on waits for: a published release, or a browser game's play address", () => {
    expect(advanceHint(["beta", "released"], false)).toEqual({
      text: "Move to Beta or Release it once you've published a release.",
      link: "Submit a release",
    });
    expect(advanceHint(["beta", "released"], true)).toEqual({
      text: "Move to Beta or Release it once you've published a play address.",
      link: "Add the play address",
    });
    expect(advanceHint(["released"], true).text).toBe("Release it once you've published a play address.");
  });
});

describe("canPutInStore", () => {
  it("lets its maker list a draft: a Coming soon page, or once a release is published; never a private app", () => {
    expect(canPutInStore({ status: "draft", private: false, stage: "coming_soon" }, false)).toBe(true);
    expect(canPutInStore({ status: "draft", private: false, stage: "beta" }, true)).toBe(true);
    expect(canPutInStore({ status: "draft", private: false, stage: "beta" }, false)).toBe(false);
    // Still loading: no button yet.
    expect(canPutInStore({ status: "draft", private: false, stage: "beta" }, undefined)).toBe(false);
    expect(canPutInStore({ status: "draft", private: true, stage: "coming_soon" }, true)).toBe(false);
    // Already listed, or taken out by staff (an appeal brings it back).
    expect(canPutInStore({ status: "listed", private: false, stage: "beta" }, true)).toBe(false);
    expect(canPutInStore({ status: "delisted", private: false, stage: "beta" }, true)).toBe(false);
    expect(canPutInStore({ status: undefined, private: false, stage: "beta" }, true)).toBe(false);
  });
});

describe("releaseDateNote", () => {
  const now = Date.parse("2026-11-19T12:00:00Z");
  it("says what players see, or that the date has passed and needs updating", () => {
    expect(releaseDateNote({ stage: "beta", release_date: "2026-12-05", release_date_precision: "day" }, now)).toEqual({
      text: "Players see “Releases Dec 5”.",
      passed: false,
    });
    const passed = releaseDateNote(
      { stage: "coming_soon", release_date: "2026-10-01", release_date_precision: "month", release_date_passed: true },
      now,
    );
    expect(passed?.passed).toBe(true);
    expect(passed?.text).toMatch(/has passed, so players no longer see it/);
  });
  it("has nothing to say without a date, or once released", () => {
    expect(releaseDateNote({ stage: "beta", release_date: null, release_date_precision: null }, now)).toBeNull();
    expect(releaseDateNote({ stage: "released", release_date: "2026-12-05", release_date_precision: "day" }, now)).toBeNull();
  });
});
