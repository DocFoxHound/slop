// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { advanceHint, moveEffects } from "./MakerFrame";

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
  it("says what moving on waits for: an approved release, or a browser game's play address", () => {
    expect(advanceHint(["beta", "released"], false)).toEqual({
      text: "Move to Beta or Release it once a release is approved.",
      link: "Submit a release",
    });
    expect(advanceHint(["beta", "released"], true)).toEqual({
      text: "Move to Beta or Release it once a play address is approved.",
      link: "Add the play address",
    });
    expect(advanceHint(["released"], true).text).toBe("Release it once a play address is approved.");
  });
});
