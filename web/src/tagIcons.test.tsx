// SPDX-License-Identifier: AGPL-3.0-or-later
// Tag icons (tagIcons.ts, TagIcon in components/StoreIcons.tsx): every tag in the catalog has one,
// drawn like the store icons and inside the 24×24 box, none alike, and an unknown tag falls back.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { CATEGORY_ICON_SLUGS, storeIconPaths } from "./storeIcons";
import { TAG_ICON_NAMES, tagIconName, tagIconPaths } from "./tagIcons";
import { TagIcon } from "./components/StoreIcons";

/** Every tag slug the catalog's migrations insert (crates/slop-api/migrations). */
function catalogTags(): string[] {
  const dir = join(__dirname, "../../crates/slop-api/migrations");
  const slugs = new Set<string>();
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".sql"))) {
    const sql = readFileSync(join(dir, file), "utf8");
    if (!/insert into tags/i.test(sql)) continue;
    // Each insert into tags, up to its semicolon. Tag rows are ('slug', 'Name', category id or slug).
    for (const insert of sql.match(/insert into tags[^;]*;/gi) ?? [])
      for (const m of insert.matchAll(/\('([a-z0-9-]+)', '[^']+', (?:\d+|'[a-z-]+')\)/g)) slugs.add(m[1]);
  }
  return [...slugs];
}

/** Points along a path (ends, control points and samples along arcs), to bound it roughly. */
function pathPoints(d: string): Array<[number, number]> {
  const tokens = d.match(/[a-zA-Z]|-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?/g) ?? [];
  const pts: Array<[number, number]> = [];
  let i = 0;
  let cmd = "";
  let x = 0;
  let y = 0;
  let sx = 0;
  let sy = 0;
  const num = () => Number(tokens[i++]);
  while (i < tokens.length) {
    if (/[a-zA-Z]/.test(tokens[i])) cmd = tokens[i++];
    const rel = cmd === cmd.toLowerCase();
    const ox = rel ? x : 0;
    const oy = rel ? y : 0;
    switch (cmd.toUpperCase()) {
      case "M":
      case "L":
        x = ox + num();
        y = oy + num();
        if (cmd.toUpperCase() === "M") [sx, sy] = [x, y];
        cmd = rel ? "l" : "L";
        break;
      case "H":
        x = ox + num();
        break;
      case "V":
        y = oy + num();
        break;
      case "C":
        pts.push([ox + num(), oy + num()], [ox + num(), oy + num()]);
        x = ox + num();
        y = oy + num();
        break;
      case "S":
        pts.push([ox + num(), oy + num()]);
        x = ox + num();
        y = oy + num();
        break;
      case "Q":
        pts.push([ox + num(), oy + num()]);
        x = ox + num();
        y = oy + num();
        break;
      case "A": {
        const [rx, ry, rot, large, sweep] = [num(), num(), num(), num(), num()];
        const [x2, y2] = [ox + num(), oy + num()];
        pts.push(...arcSamples(x, y, rx, ry, rot, large, sweep, x2, y2));
        [x, y] = [x2, y2];
        break;
      }
      case "Z":
        [x, y] = [sx, sy];
        break;
      default:
        throw new Error(`unexpected ${cmd} in ${d}`);
    }
    pts.push([x, y]);
  }
  return pts;
}

/** Points along an SVG arc (the spec's endpoint-to-centre conversion). */
function arcSamples(x1: number, y1: number, rx: number, ry: number, rotDeg: number, large: number, sweep: number, x2: number, y2: number) {
  const phi = (rotDeg * Math.PI) / 180;
  const [c, s] = [Math.cos(phi), Math.sin(phi)];
  const dx = (x1 - x2) / 2;
  const dy = (y1 - y2) / 2;
  const x1p = c * dx + s * dy;
  const y1p = -s * dx + c * dy;
  const scale = Math.max(1, Math.sqrt((x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)));
  rx *= scale;
  ry *= scale;
  const num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
  const k = (large === sweep ? -1 : 1) * Math.sqrt(Math.max(0, num / (rx * rx * y1p * y1p + ry * ry * x1p * x1p)));
  const cxp = (k * rx * y1p) / ry;
  const cyp = (-k * ry * x1p) / rx;
  const cx = c * cxp - s * cyp + (x1 + x2) / 2;
  const cy = s * cxp + c * cyp + (y1 + y2) / 2;
  const angle = (ux: number, uy: number) => Math.atan2(uy, ux);
  const t1 = angle((x1p - cxp) / rx, (y1p - cyp) / ry);
  let dt = angle((-x1p - cxp) / rx, (-y1p - cyp) / ry) - t1;
  if (sweep && dt < 0) dt += 2 * Math.PI;
  if (!sweep && dt > 0) dt -= 2 * Math.PI;
  return Array.from({ length: 25 }, (_, j): [number, number] => {
    const t = t1 + (dt * j) / 24;
    return [cx + rx * Math.cos(t) * c - ry * Math.sin(t) * s, cy + rx * Math.cos(t) * s + ry * Math.sin(t) * c];
  });
}

describe("tag icons", () => {
  afterEach(cleanup);

  it("every tag in the catalog has an icon of its own, and every icon is a tag's", () => {
    const tags = catalogTags();
    expect(tags.length).toBeGreaterThanOrEqual(108);
    for (const slug of tags) expect(tagIconName(slug), slug).toBe(slug);
    for (const name of TAG_ICON_NAMES) expect(tags, name).toContain(name);
  });

  it("draws every icon in both styles, in the text colour or slime only", () => {
    const colours = new Set(["none", "currentColor", "var(--icon-accent, #a3e635)", "var(--icon-tone, rgba(163, 230, 53, 0.3))"]);
    for (const name of TAG_ICON_NAMES) {
      for (const style of ["drip", "goo"] as const) {
        const paths = tagIconPaths(name, style);
        expect(paths.length, `${name} ${style}`).toBeGreaterThan(0);
        for (const p of paths) {
          expect(p.d).toMatch(/^M/);
          expect(p.d).not.toMatch(/NaN|undefined|Infinity/);
          expect(colours.has(p.fill), p.fill).toBe(true);
          expect(colours.has(p.stroke), p.stroke).toBe(true);
          expect(p.fill === "none" && p.stroke === "none").toBe(false);
        }
      }
    }
  });

  it("drip has slime in every icon, goo none", () => {
    for (const name of TAG_ICON_NAMES) {
      const slime = (p: { fill: string; stroke: string }) => p.fill.includes("--icon-accent") || p.stroke.includes("--icon-accent");
      expect(tagIconPaths(name, "drip").some(slime), name).toBe(true);
      const goo = tagIconPaths(name, "goo");
      expect(goo.some(slime), name).toBe(false);
    }
  });

  it("stays inside the 24×24 box, with room for the stroke", () => {
    const out: string[] = [];
    for (const name of TAG_ICON_NAMES) {
      const pts = tagIconPaths(name, "drip").flatMap((p) => pathPoints(p.d));
      const xs = pts.map(([x]) => x);
      const ys = pts.map(([, y]) => y);
      const box = [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)].map((v) => Math.round(v * 10) / 10);
      if (box[0] < 1.5 || box[1] < 1.5 || box[2] > 22.5 || box[3] > 22.5) out.push(`${name} [${box.join(", ")}]`);
    }
    expect(out).toEqual([]);
  });

  it("no two tags look the same, and none is a category's icon", () => {
    const drawn = (paths: { d: string }[]) => paths.map((p) => p.d).join("|");
    const seen = new Map<string, string>();
    for (const name of TAG_ICON_NAMES) {
      const key = drawn(tagIconPaths(name, "drip"));
      expect(seen.get(key), `${name} draws like ${seen.get(key)}`).toBeUndefined();
      seen.set(key, name);
    }
    for (const category of CATEGORY_ICON_SLUGS) expect(seen.get(drawn(storeIconPaths(category, "drip"))), category).toBeUndefined();
  });

  it("TagIcon draws a tag's own icon; an unknown tag gets its category's, else the neutral one", () => {
    const icon = (el: HTMLElement) => el.querySelector("svg")!;
    const own = render(<TagIcon slug="roguelike" category="games" size={20} />).container;
    expect(icon(own).getAttribute("data-tag")).toBe("roguelike");
    expect(icon(own).getAttribute("width")).toBe("20");
    expect(icon(own).getAttribute("aria-hidden")).toBe("true");
    expect(icon(own).getAttribute("class")).toBe("store-icon");
    // "family" is a tag and a category: the tag's icon, not the category's.
    const family = render(<TagIcon slug="family" category="family" />).container;
    expect(icon(family).getAttribute("data-tag")).toBe("family");
    expect(icon(family).getAttribute("data-icon")).toBeNull();
    const fallback = render(<TagIcon slug="brand-new-tag" category="vibe" className="x" />).container;
    expect(icon(fallback).getAttribute("data-icon")).toBe("vibe");
    expect(icon(fallback).getAttribute("class")).toBe("store-icon x");
    const neutral = render(<TagIcon slug="brand-new-tag" />).container;
    expect(icon(neutral).getAttribute("data-icon")).toBe("category");
    expect(tagIconName("brand-new-tag")).toBeNull();
    expect(tagIconName("browse")).toBeNull();
  });
});
