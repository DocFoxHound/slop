// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { leadingZeroBits, sha256Words, solve, solves } from "./pow";

const hex = (h: Uint32Array) => Array.from(h, (w) => (w >>> 0).toString(16).padStart(8, "0")).join("");

describe("proof of work", () => {
  it("hashes like SHA-256", () => {
    expect(hex(sha256Words("abc"))).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    expect(hex(sha256Words(""))).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    // Longer than one block.
    expect(hex(sha256Words("a".repeat(100)))).toBe("2816597888e4a0d3a36b82b83316ab32680eb8f00f8cd3b904d681246d285a0e");
  });

  it("finds an answer the server accepts", async () => {
    const nonce = await solve("abc", 12);
    expect(solves("abc", nonce, 12)).toBe(true);
    expect(leadingZeroBits(sha256Words(`abc:${nonce}`))).toBeGreaterThanOrEqual(12);
  });
});
