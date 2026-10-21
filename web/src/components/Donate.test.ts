import { describe, expect, it } from "vitest";

import { shareFor } from "./Donate";

describe("shareFor", () => {
  it("gives the original 30% of the two payments together, like the server", () => {
    expect(shareFor(700, 30)).toBe(300);
    expect(shareFor(100, 30)).toBe(43);
    const share = shareFor(333, 30);
    expect(share * 70).toBeGreaterThanOrEqual(333 * 30);
  });
});
