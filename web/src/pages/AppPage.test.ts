// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { Credit } from "../api";
import { creditsByRole, runsOn } from "./AppPage";

const c = (handle: string, role: string): Credit => ({ handle, name: handle, role, skill: { slug: "x", name: "x" }, work: "w" });

describe("creditsByRole", () => {
  it("groups each creative once per role, in order", () => {
    const g = creditsByRole([c("a", "Art"), c("m", "Music"), c("b", "Art"), c("a", "Art")]);
    expect(g.map(([r, p]) => [r, p.map((x) => x.handle)])).toEqual([
      ["Art", ["a", "b"]],
      ["Music", ["m"]],
    ]);
  });
});

describe("runsOn", () => {
  it("names the systems a download runs on, and the browser for a browser game", () => {
    expect(runsOn(["windows", "linux", "linux-deb"])).toBe("Windows · Linux (also as .deb)");
    expect(runsOn(["linux", "linux-deb", "linux-rpm"])).toBe("Linux (also as .deb and .rpm)");
    expect(runsOn(["browser"])).toBe("Your browser");
  });
});
