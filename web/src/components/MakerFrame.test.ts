// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { moveEffects } from "./MakerFrame";

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
