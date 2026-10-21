// SPDX-License-Identifier: AGPL-3.0-or-later
// Browser tests (web/e2e): the real website against a throwaway Slop (crates/slop-api/examples/
// e2e_server.rs), with GitHub faked. `npm run e2e` (build the website first: `npm run build`).
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.e2e.ts",
  globalSetup: "./e2e/setup.ts",
  // One server, one database: tests share it, so they run one file at a time.
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 30_000,
  reporter: [["list"]],
  use: { ...devices["Desktop Chrome"], trace: "retain-on-failure", screenshot: "only-on-failure" },
  outputDir: "e2e/.results",
});
