// SPDX-License-Identifier: AGPL-3.0-or-later
// Browser tests (web/e2e): the real website against a throwaway SLAPorSLOP (crates/slop-api/examples/
// e2e_server/), with GitHub and Stripe faked and everything seeded (e2e/fixtures.ts lists it).
// `npm run e2e` (build the website first: `npm run build`).
import { readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, devices } from "@playwright/test";

const use = { ...devices["Desktop Chrome"], trace: "retain-on-failure", screenshot: "only-on-failure" } as const;

/**
 * The files in another order, to find tests that only pass because of what an earlier file left
 * behind (or fail because of it). SLOP_E2E_ORDER=reverse, or shuffle:<seed> (the same seed gives
 * the same order). Playwright itself always runs files alphabetically, so each file becomes a
 * project of its own, run in the order listed (.github/workflows/order-check.yml).
 */
function reordered(order: string | undefined) {
  if (!order) return undefined;
  const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), "e2e");
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".e2e.ts"))
    .sort();
  if (order === "reverse") {
    files.reverse();
  } else if (order.startsWith("shuffle:")) {
    // A small seeded generator (mulberry32) and a Fisher-Yates shuffle.
    let seed = Number(order.slice("shuffle:".length)) >>> 0;
    const next = () => {
      seed = (seed + 0x6d2b79f5) >>> 0;
      let t = seed;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    for (let i = files.length - 1; i > 0; i--) {
      const j = Math.floor(next() * (i + 1));
      [files[i], files[j]] = [files[j], files[i]];
    }
  } else {
    throw new Error(`SLOP_E2E_ORDER must be "reverse" or "shuffle:<seed>", not ${order}`);
  }
  return files.map((f) => ({ name: f, testMatch: f, use }));
}

const order = process.env.SLOP_E2E_ORDER;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.e2e.ts",
  globalSetup: "./e2e/setup.ts",
  // One server, one database: tests share it, so they run one file at a time.
  workers: 1,
  fullyParallel: false,
  // On GitHub (CI) a test that fails gets one more try; one that then passes is reported as
  // flaky, named on the run page to be fixed (.github/workflows/pipeline.yml). Locally, and in
  // the order checks (which are there to find exactly those tests), none.
  retries: process.env.CI && !order ? 1 : 0,
  timeout: 30_000,
  reporter: [["list"]],
  use,
  projects: reordered(order),
  outputDir: "e2e/.results",
});
