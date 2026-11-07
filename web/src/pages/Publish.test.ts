// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { chooseDelivery, samePlatforms } from "./Publish";

describe("how players get it: a download or the browser", () => {
  it("In the browser stands alone; Download takes it away and keeps the builds", () => {
    expect(chooseDelivery(["windows", "linux", "linux-deb"], true)).toEqual(["browser"]);
    expect(chooseDelivery([], true)).toEqual(["browser"]);
    expect(chooseDelivery(["browser"], false)).toEqual([]);
    expect(chooseDelivery(["windows", "linux", "linux-rpm"], false)).toEqual(["windows", "linux", "linux-rpm"]);
  });

  it("the Edit page sends platforms only when they changed", () => {
    expect(samePlatforms(["windows", "linux", "linux-deb"], ["linux-deb", "windows", "linux"])).toBe(true);
    expect(samePlatforms(["browser"], ["browser"])).toBe(true);
    expect(samePlatforms(["windows"], ["windows", "linux"])).toBe(false);
    expect(samePlatforms(["browser"], ["windows"])).toBe(false);
  });
});
