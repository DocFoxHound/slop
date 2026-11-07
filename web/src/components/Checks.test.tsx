// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's source check on a browser game's release: only what applies to a game on its maker's
// website, and nothing that sounds like a check of the site. A dedicated server's code loading is a
// note, as the Content Policy has it, never a rule it breaks.
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

describe("the source check on a dedicated server's release", () => {
  const server: RepoScan = {
    ...scan,
    code_loading: [hit("load_native"), { ...hit("unsafe_format"), label: "A data format that can rebuild any object" }],
  };
  const words = (audience: "maker" | "reviewer", props: { server?: boolean; scan?: RepoScan } = {}) => {
    const out = render(
      <MemoryRouter>
        <RepoScanList scan={props.scan ?? server} advisories={[]} audience={audience} server={props.server} />
      </MemoryRouter>,
    ).container.textContent ?? "";
    cleanup();
    return out;
  };

  it("says loading mods, maps and plugins from elsewhere is a note, for the maker and the reviewer", () => {
    for (const audience of ["maker", "reviewer"] as const) {
      const said = words(audience, { server: true });
      expect(said).toContain("LOADS CODE FROM OUTSIDE SLAPorSLOP (2)");
      expect(said).toContain("dedicated servers may load mods, maps and plugins from other places");
      expect(said).not.toContain("a mod loader outside SLAPorSLOP's mods breaks the Content Policy");
      expect(said).not.toContain("never programs, scripts or mods from a server");
      expect(said).not.toContain("CODE FROM OUTSIDE SLAPorSLOP?");
      // The hits are still listed, and an unsafe data format is still worth replacing.
      expect(said).toContain("load_native (1)");
      expect(said).toMatch(/format that can rebuild objects is still worth replacing|unsafe format .* is a note asking the maker to replace it/);
    }
    // The reviewer is told to check it really is one.
    expect(words("reviewer", { server: true })).toContain("Check it really is a dedicated server");
  });

  it("goes by what the scan recorded when it isn't told, and a download keeps the rule", () => {
    expect(words("reviewer", { scan: { ...server, dedicated_server: true } })).toContain("LOADS CODE FROM OUTSIDE SLAPorSLOP (2)");
    const download = words("reviewer", { scan: { ...server, dedicated_server: false } });
    expect(download).toContain("CODE FROM OUTSIDE SLAPorSLOP? (2)");
    expect(download).toContain("a mod loader outside SLAPorSLOP's mods breaks the Content Policy");
    // Told it isn't one now, whatever the scan recorded.
    expect(words("maker", { scan: { ...server, dedicated_server: true }, server: false })).toContain("CODE FROM OUTSIDE SLAPorSLOP? (2)");
  });
});
