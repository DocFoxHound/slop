// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { categoryIconName, STORE_ICON_NAMES, storeIconPaths } from "../storeIcons";
import { CategoryIcon, StoreIcon } from "./StoreIcons";

describe("StoreIcon", () => {
  afterEach(cleanup);

  it("is decorative: hidden from screen readers, never focusable, with no text", () => {
    const { container } = render(<StoreIcon name="games" />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("focusable")).toBe("false");
    expect(svg.textContent).toBe("");
    expect(svg.getAttribute("class")).toBe("store-icon");
  });

  it("is sized up front, so nothing moves when it draws", () => {
    const { container } = render(<StoreIcon name="contact" size={40} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("width")).toBe("40");
    expect(svg.getAttribute("height")).toBe("40");
    expect(svg.getAttribute("viewBox")).toBe("0 0 24 24");
  });

  it("gives a category its own icon, and one it doesn't know the neutral one", () => {
    for (const slug of ["games", "family", "tools", "creative", "utilities", "servers", "learning", "play-style", "vibe"]) {
      expect(categoryIconName(slug)).toBe(slug);
    }
    expect(categoryIconName("something-new")).toBe("category");
    expect(categoryIconName("browse")).toBe("category");
    const { container } = render(<CategoryIcon slug="something-new" />);
    expect(container.querySelector("svg")!.getAttribute("data-icon")).toBe("category");
  });
});

describe("storeIconPaths", () => {
  it("draws every icon in both styles, in the text colour or slime only", () => {
    const colours = new Set(["none", "currentColor", "var(--icon-accent, #a3e635)", "var(--icon-tone, rgba(163, 230, 53, 0.3))"]);
    for (const name of STORE_ICON_NAMES) {
      for (const style of ["drip", "goo"] as const) {
        const paths = storeIconPaths(name, style);
        expect(paths.length, `${name} ${style}`).toBeGreaterThan(0);
        for (const p of paths) {
          expect(p.d).toMatch(/^M/);
          expect(p.d).not.toMatch(/NaN|undefined/);
          expect(colours.has(p.fill), p.fill).toBe(true);
          expect(colours.has(p.stroke), p.stroke).toBe(true);
          // Something is drawn: a path never has neither fill nor stroke.
          expect(p.fill === "none" && p.stroke === "none").toBe(false);
        }
      }
    }
  });

  it("drip has slime in every icon; goo has its slime shadow and no drips", () => {
    for (const name of STORE_ICON_NAMES) {
      const drip = storeIconPaths(name, "drip");
      expect(drip.some((p) => p.fill.includes("--icon-accent") || p.stroke.includes("--icon-accent")), name).toBe(true);
      const goo = storeIconPaths(name, "goo");
      expect(goo.some((p) => p.fill.includes("--icon-accent") || p.stroke.includes("--icon-accent")), name).toBe(false);
    }
    // Icons with a closed outline get the shadow, behind everything else.
    const games = storeIconPaths("games", "goo");
    expect(games[0].fill).toContain("--icon-tone");
    expect(games[0].transform).toBeDefined();
  });

  it("draws Servers as a rack with one slime status light, unlike Utilities' cog", () => {
    const drip = storeIconPaths("servers", "drip");
    const slime = drip.filter((p) => p.fill.includes("--icon-accent") || p.stroke.includes("--icon-accent"));
    expect(slime).toHaveLength(1);
    expect(slime[0].stroke).toBe("none");
    // Both servers are bodies the goo style shadows, behind everything else.
    const goo = storeIconPaths("servers", "goo");
    expect(goo.filter((p) => p.fill.includes("--icon-tone"))).toHaveLength(2);
    expect(goo[0].fill).toContain("--icon-tone");
    const drawn = (name: "servers" | "utilities") => storeIconPaths(name, "drip").map((p) => p.d).join("|");
    expect(drawn("servers")).not.toBe(drawn("utilities"));
    const { container } = render(<CategoryIcon slug="servers" />);
    expect(container.querySelector("svg")!.getAttribute("data-icon")).toBe("servers");
  });

  it("draws games that play in the browser as a window with one slime Play triangle, unlike Browse's compass", () => {
    expect(STORE_ICON_NAMES).toContain("browser");
    const drip = storeIconPaths("browser", "drip");
    const slime = drip.filter((p) => p.fill.includes("--icon-accent") || p.stroke.includes("--icon-accent"));
    // One touch of slime: the triangle, filled.
    expect(slime).toHaveLength(1);
    expect(slime[0].stroke).toBe("none");
    // The window's outline is the body the goo style shadows.
    const goo = storeIconPaths("browser", "goo");
    expect(goo.filter((p) => p.fill.includes("--icon-tone"))).toHaveLength(1);
    expect(goo[0].fill).toContain("--icon-tone");
    const drawn = (name: "browser" | "browse") => storeIconPaths(name, "drip").map((p) => p.d).join("|");
    expect(drawn("browser")).not.toBe(drawn("browse"));
    const { container } = render(<StoreIcon name="browser" />);
    expect(container.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });
});
