// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's source check on a browser game's release: only what applies to a game on its maker's
// website, and nothing that sounds like a check of the site.
import { cleanup, render } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { RepoScan } from "../api";
import { RepoScanList } from "./Checks";

afterEach(cleanup);

const hit = (rule: string) => ({ rule, label: rule, count: 1, examples: ["src/game.js:3"] });

const scan: RepoScan = {
  status: "checked",
  licence: { declared: "MIT", files: [{ path: "LICENSE", spdx: "MIT" }], headers: [], problems: [] },
  monetization: [hit("checkout_link")],
  code_loading: [hit("eval")],
  network: [hit("websocket")],
  online_undeclared: true,
  saves: [hit("appdata")],
  save_folder: "elsewhere",
  pulse_missing: true,
  design: [hit("premium_currency")],
};

const text = (browser: boolean) =>
  render(
    <MemoryRouter>
      <RepoScanList scan={scan} advisories={[]} audience="maker" browser={browser} />
    </MemoryRouter>,
  ).container.textContent ?? "";

describe("the source check on a browser game's release", () => {
  it("keeps the licence, selling and pay-to-win notes", () => {
    const words = text(true);
    expect(words).toContain("LICENCE FILE MATCHES");
    expect(words).toContain("POSSIBLE PURCHASES OR LOOT BOXES (1)");
    expect(words).toContain("PAY-TO-WIN DESIGN? (1)");
  });

  it("leaves out what's about a download: outside code, going online, the save folder, Pulse", () => {
    const words = text(true);
    for (const gone of ["OUTSIDE CODE", "CODE FROM OUTSIDE", "GOES ONLINE", "SAVE FOLDER", "SAVES OUTSIDE", "SLOP PULSE"]) {
      expect(words).not.toContain(gone);
    }
    // A download's check still says all of it.
    cleanup();
    const download = text(false);
    for (const there of ["CODE FROM OUTSIDE SLAPorSLOP? (1)", "GOES ONLINE, NOT ON THE STORE PAGE", "SAVES OUTSIDE SLAPorSLOP'S SAVE FOLDER", "NO SLOP PULSE"]) {
      expect(download).toContain(there);
    }
  });
});
