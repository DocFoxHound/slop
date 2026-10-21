// SPDX-License-Identifier: AGPL-3.0-or-later
// The proof-of-work check Slop asks for after a few unfinished donation checkouts
// (crates/slop-api/src/challenge.rs): find a number whose SHA-256 of "prefix:number" starts with
// `difficulty` zero bits. A plain synchronous SHA-256 is far faster here than Web Crypto's
// promise per hash. It runs in short slices so the page stays responsive.

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
  0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
  0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/** SHA-256 of an ASCII string, as eight 32-bit words. */
export function sha256Words(text: string): Uint32Array {
  const len = text.length;
  const blocks = ((len + 9 + 63) >> 6) << 4;
  const w = new Uint32Array(blocks);
  for (let i = 0; i < len; i++) w[i >> 2] |= (text.charCodeAt(i) & 0xff) << (24 - (i & 3) * 8);
  w[len >> 2] |= 0x80 << (24 - (len & 3) * 8);
  w[blocks - 1] = len * 8;
  const h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  const m = new Uint32Array(64);
  for (let b = 0; b < blocks; b += 16) {
    for (let t = 0; t < 16; t++) m[t] = w[b + t];
    for (let t = 16; t < 64; t++) {
      const x = m[t - 15];
      const y = m[t - 2];
      const s0 = ((x >>> 7) | (x << 25)) ^ ((x >>> 18) | (x << 14)) ^ (x >>> 3);
      const s1 = ((y >>> 17) | (y << 15)) ^ ((y >>> 19) | (y << 13)) ^ (y >>> 10);
      m[t] = (m[t - 16] + s0 + m[t - 7] + s1) | 0;
    }
    let [a, bb, c, d, e, f, g, hh] = h;
    for (let t = 0; t < 64; t++) {
      const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
      const ch = (e & f) ^ (~e & g);
      const t1 = (hh + S1 + ch + K[t] + m[t]) | 0;
      const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
      const maj = (a & bb) ^ (a & c) ^ (bb & c);
      const t2 = (S0 + maj) | 0;
      hh = g;
      g = f;
      f = e;
      e = (d + t1) | 0;
      d = c;
      c = bb;
      bb = a;
      a = (t1 + t2) | 0;
    }
    h[0] += a;
    h[1] += bb;
    h[2] += c;
    h[3] += d;
    h[4] += e;
    h[5] += f;
    h[6] += g;
    h[7] += hh;
  }
  return h;
}

/** Whether the hash starts with `difficulty` zero bits. */
export function leadingZeroBits(h: Uint32Array): number {
  let zeros = 0;
  for (const word of h) {
    if (word === 0) {
      zeros += 32;
      continue;
    }
    return zeros + Math.clz32(word);
  }
  return zeros;
}

export function solves(prefix: string, nonce: string, difficulty: number): boolean {
  return leadingZeroBits(sha256Words(`${prefix}:${nonce}`)) >= difficulty;
}

/** Finds an answer, yielding to the page every few thousand tries. */
export async function solve(prefix: string, difficulty: number): Promise<string> {
  for (let n = 0; ; n++) {
    if (solves(prefix, String(n), difficulty)) return String(n);
    if (n % 20000 === 19999) await new Promise((r) => setTimeout(r, 0));
  }
}
