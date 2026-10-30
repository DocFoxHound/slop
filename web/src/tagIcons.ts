// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's icons for tags (the sub-genres under each category: roguelike, cozy, pixel-art…),
// in the same family as the store icons (storeIcons.ts): drawn by hand on the same 24×24 grid,
// from the same pieces, one touch of slime each, rendered in either style by iconPaths.
//
// Tags have their own namespace: a tag can share its slug with a category or a store icon
// ("family" is both), so tag icons live here, keyed by tag slug. Tags come from the API; one
// without an icon here falls back to its category's (components/StoreIcons.tsx, TagIcon).

import {
  accent,
  accentLine,
  body,
  circle,
  cog,
  dot,
  dripPart,
  iconPaths,
  line,
  rect,
  STORE_ICON_STYLE,
  type IconPath,
  type Part,
  type StoreIconStyle,
} from "./storeIcons";

type Pt = readonly [number, number];

const n = (v: number) => String(Math.round(v * 100) / 100);
const rad = (deg: number) => (deg * Math.PI) / 180;
/** The point `r` from (cx, cy) at `deg` degrees (0 is right, 90 is down). */
const at = (cx: number, cy: number, r: number, deg: number): Pt => [cx + r * Math.cos(rad(deg)), cy + r * Math.sin(rad(deg))];

/** An ellipse as a path, turned `deg` degrees. */
function ellipse(cx: number, cy: number, rx: number, ry: number, deg = 0): string {
  const [x1, y1] = at(cx, cy, rx, deg + 180);
  const [x2, y2] = at(cx, cy, rx, deg);
  const a = `A${n(rx)} ${n(ry)} ${n(deg)} 1 0`;
  return `M${n(x1)} ${n(y1)}${a} ${n(x2)} ${n(y2)}${a} ${n(x1)} ${n(y1)}Z`;
}

/** An arc of the circle at (cx, cy), clockwise from `from` to `to` degrees. */
function arc(cx: number, cy: number, r: number, from: number, to: number): string {
  const [x1, y1] = at(cx, cy, r, from);
  const [x2, y2] = at(cx, cy, r, to);
  return `M${n(x1)} ${n(y1)}A${n(r)} ${n(r)} 0 ${to - from > 180 ? 1 : 0} 1 ${n(x2)} ${n(y2)}`;
}

/** Straight lines through the points, closed unless `open`. */
function poly(pts: readonly Pt[], open = false): string {
  return pts.map(([x, y], i) => `${i === 0 ? "M" : "L"}${n(x)} ${n(y)}`).join("") + (open ? "" : "Z");
}

/** A closed polygon with its corners rounded by `r`. */
function roundPoly(pts: readonly Pt[], r: number): string {
  const k = pts.length;
  let d = "";
  for (let i = 0; i < k; i++) {
    const [px, py] = pts[(i + k - 1) % k];
    const [x, y] = pts[i];
    const [nx, ny] = pts[(i + 1) % k];
    const toward = (tx: number, ty: number): Pt => {
      const len = Math.hypot(tx - x, ty - y);
      const t = Math.min(r, len / 2) / len;
      return [x + (tx - x) * t, y + (ty - y) * t];
    };
    const [ax, ay] = toward(px, py);
    const [bx, by] = toward(nx, ny);
    d += `${i === 0 ? "M" : "L"}${n(ax)} ${n(ay)}Q${n(x)} ${n(y)} ${n(bx)} ${n(by)}`;
  }
  return d + "Z";
}

/** A smooth curve through the points (Catmull-Rom), closed unless `open`. */
function smooth(pts: readonly Pt[], open = false): string {
  const k = pts.length;
  const p = (i: number) => (open ? pts[Math.max(0, Math.min(k - 1, i))] : pts[(i + k) % k]);
  let d = `M${n(pts[0][0])} ${n(pts[0][1])}`;
  for (let i = 0; i < (open ? k - 1 : k); i++) {
    const [x0, y0] = p(i - 1);
    const [x1, y1] = p(i);
    const [x2, y2] = p(i + 1);
    const [x3, y3] = p(i + 2);
    d += `C${n(x1 + (x2 - x0) / 6)} ${n(y1 + (y2 - y0) / 6)} ${n(x2 - (x3 - x1) / 6)} ${n(y2 - (y3 - y1) / 6)} ${n(x2)} ${n(y2)}`;
  }
  return d + (open ? "" : "Z");
}

/** The points moved to (cx, cy), turned `deg` degrees, and mirrored left to right if `flip`. */
function place(pts: readonly Pt[], cx: number, cy: number, deg: number, flip = false): Pt[] {
  const c = Math.cos(rad(deg));
  const s = Math.sin(rad(deg));
  return pts.map(([x0, y]) => {
    const x = flip ? -x0 : x0;
    return [cx + x * c - y * s, cy + x * s + y * c];
  });
}

/** A bare footprint, pointing up from (0, 0): the sole, then three toes. */
const SOLE: Pt[] = [[0.2, -4.4], [2.3, -3.2], [2.5, 0.4], [1.6, 3.8], [0, 5], [-1.7, 4.1], [-1.4, 1.2], [-2.4, -2.2]];
const TOES: Pt[] = [[-1.5, -6.6], [0.6, -7.2], [2.3, -6.2]];

/** A star (or burst): points alternating between the radii, the first straight up. */
function star(cx: number, cy: number, radii: readonly number[], turn = 0): string {
  const step = 360 / radii.length;
  return poly(radii.map((r, i) => at(cx, cy, r, -90 + turn + i * step)));
}

/** A four-pointed sparkle at (cx, cy), `r` to each point. */
function sparkle(cx: number, cy: number, r: number): string {
  const c = r * 0.18;
  return (
    `M${n(cx)} ${n(cy - r)}C${n(cx + c)} ${n(cy - c)} ${n(cx + c)} ${n(cy - c)} ${n(cx + r)} ${n(cy)}` +
    `C${n(cx + c)} ${n(cy + c)} ${n(cx + c)} ${n(cy + c)} ${n(cx)} ${n(cy + r)}` +
    `C${n(cx - c)} ${n(cy + c)} ${n(cx - c)} ${n(cy + c)} ${n(cx - r)} ${n(cy)}` +
    `C${n(cx - c)} ${n(cy - c)} ${n(cx - c)} ${n(cy - c)} ${n(cx)} ${n(cy - r)}Z`
  );
}

/** A heart `w` wide, its dip at (cx, top). */
function heart(cx: number, top: number, w: number): string {
  const s = w / 18; // drawn at 18 wide
  const p = (x: number, y: number) => `${n(cx + x * s)} ${n(top + y * s)}`;
  return (
    `M${p(0, 14.4)}C${p(-6.4, 10.6)} ${p(-9, 7.6)} ${p(-9, 4.4)}C${p(-9, 1.9)} ${p(-7.1, 0)} ${p(-4.7, 0)}` +
    `C${p(-2.8, 0)} ${p(-1.1, 1)} ${p(0, 2.7)}C${p(1.1, 1)} ${p(2.8, 0)} ${p(4.7, 0)}` +
    `C${p(7.1, 0)} ${p(9, 1.9)} ${p(9, 4.4)}C${p(9, 7.6)} ${p(6.4, 10.6)} ${p(0, 14.4)}Z`
  );
}

/** A flower: `petals` round petals about (cx, cy), between `inner` and about `outer`. */
function flower(cx: number, cy: number, petals: number, inner: number, outer: number): string {
  const step = 360 / petals;
  const valleys = Array.from({ length: petals }, (_, i) => at(cx, cy, inner, -90 + step / 2 + i * step));
  const chord = Math.hypot(valleys[1][0] - valleys[0][0], valleys[1][1] - valleys[0][1]);
  const r = Math.max(chord / 2, (outer - inner * Math.cos(rad(step / 2))) * 0.62);
  let d = `M${n(valleys[petals - 1][0])} ${n(valleys[petals - 1][1])}`;
  for (const [x, y] of valleys) d += `A${n(r)} ${n(r)} 0 1 1 ${n(x)} ${n(y)}`;
  return d + "Z";
}

/** A head-and-shoulders person: head of radius `r` at (cx, cy), shoulders `w` wide down to `bottom`. */
function bust(cx: number, cy: number, r: number, w: number, bottom: number): Part[] {
  const top = cy + r + 1.6;
  const rr = Math.min(w / 2, (bottom - top) * 0.9);
  return [
    body(circle(cx, cy, r)),
    body(`M${n(cx - w / 2)} ${n(bottom)}v${n(-(bottom - top - rr))}a${n(rr)} ${n(rr)} 0 0 1 ${n(rr)} ${n(-rr)}h${n(w - 2 * rr)}a${n(rr)} ${n(rr)} 0 0 1 ${n(rr)} ${n(rr)}v${n(bottom - top - rr)}`),
  ];
}

const sq = (cx: number, cy: number, s: number, r: number) => rect(cx - s / 2, cy - s / 2, s, s, r);

/** Every tag's icon, by the tag's slug, grouped by the category the tag is in today. */
const TAG_SHAPES = {
  // ---- games -------------------------------------------------------------
  /** Action: an explosion with a slime core. */
  action: [
    accent(circle(12, 12, 2.4)),
    body(star(12, 12.2, [9.4, 4.6, 7.6, 4.4, 9.2, 4.8, 7.2, 4.4, 8.8, 4.6, 7.8, 4.2, 9, 4.7], 4)),
  ],
  /** Adventure: a folded map, the spot marked in slime. */
  adventure: [
    accentLine("M14.6 10.4l3.2 3.2M17.8 10.4l-3.2 3.2"),
    body("M3 6.4 8.6 4.4 15.4 6.4 21 4.4V17.6L15.4 19.6 8.6 17.6 3 19.6Z"),
    line("M8.6 4.4V17.6M15.4 6.4V19.6"),
  ],
  /** Arcade: an arcade cabinet with a slime screen. */
  arcade: [
    accent(rect(8.6, 5.2, 6.8, 5.2, 0.8)),
    body("M7 2.8h10v8.6l1.6 2.6v7.2H5.4V14L7 11.4Z"),
    line("M5.4 14h13.2"),
    dot(circle(9, 17.2, 1.1)),
    dot(circle(13.4, 17.2, 1.1)),
    dot(circle(15.8, 17.2, 1.1)),
  ],
  /** Board game: a die, its middle pip slime. */
  "board-game": [
    accent(circle(12, 12, 1.7)),
    body(rect(3.5, 3.5, 17, 17, 3.6)),
    dot(circle(8, 8, 1.35)),
    dot(circle(16, 8, 1.35)),
    dot(circle(8, 16, 1.35)),
    dot(circle(16, 16, 1.35)),
  ],
  /** Bullet hell: a little ship under a spray of slime bullets. */
  "bullet-hell": [
    ...[-162, -126, -90, -54, -18].map((a) => accent(circle(...at(12, 12.6, 8.6, a), 1.35))),
    ...[-144, -108, -72, -36].map((a) => dot(circle(...at(12, 12.6, 4.8, a), 0.95))),
    body("M12 12.6c1 1.2 1.6 2.8 1.8 4.4l3.2 2.3v1.5l-3.5-1-1.5 1.4-1.5-1.4-3.5 1v-1.5l3.2-2.3c.2-1.6.8-3.2 1.8-4.4Z"),
  ],
  /** Card game: two cards fanned out, a slime diamond on the front one. */
  "card-game": [
    accent("M14.5 8.6l2.7 3.6-2.7 3.6-2.7-3.6Z"),
    line("M9 5.76 5.17 6.79Q3.6 7.2 4.02 8.75L6.68 18.65Q7.1 20.2 8.65 19.79L9 19.69"),
    body(rect(9, 4.5, 11, 15.5, 2)),
  ],
  /** City builder: a skyline, slime in its windows. */
  "city-builder": [
    accent(rect(10.4, 7, 2.4, 2.4, 0.4)),
    accent(rect(10.4, 11.4, 2.4, 2.4, 0.4)),
    body("M3 20.5V11h5.4V3.8h6.6v5.4H21v11.3Z"),
    dot(circle(5.7, 14.4, 1)),
    dot(circle(17.9, 12.8, 1)),
    dot(circle(17.9, 16.4, 1)),
    line("M2.5 20.5h19"),
  ],
  /** Deckbuilder: a deck, and a slime plus for the card you add. */
  deckbuilder: [
    accentLine("M18.3 2.8v5M15.8 5.3h5"),
    body(rect(3.2, 9.2, 10, 12, 1.8)),
    line("M5.8 9.2v-.8a1.8 1.8 0 0 1 1.8-1.8h6.4a1.8 1.8 0 0 1 1.8 1.8v10.2h-2.6"),
  ],
  /** Endless runner: an infinity loop with a slime runner on it. */
  "endless-runner": [
    accent(circle(17.2, 8.2, 1.9)),
    body("M12 12c-1.8-2.4-3.2-4-5.2-4a4 4 0 0 0 0 8c2 0 3.4-1.6 5.2-4s3.2-4 5.2-4a4 4 0 0 1 0 8c-2 0-3.4-1.6-5.2-4Z"),
  ],
  /** Exploration: binoculars, one lens glinting slime. */
  exploration: [
    accent(circle(17.2, 15.8, 1.7)),
    body(circle(6.8, 15.8, 4.2)),
    body(circle(17.2, 15.8, 4.2)),
    line("M3.2 13.6 4.9 6.4A1.8 1.8 0 0 1 6.7 5h1.6a1.8 1.8 0 0 1 1.8 1.8v6.6M20.8 13.6 19.1 6.4A1.8 1.8 0 0 0 17.3 5h-1.6a1.8 1.8 0 0 0-1.8 1.8v6.6M10.6 10h2.8"),
  ],
  /** Farming: a sprout in the soil, one leaf slime. */
  farming: [
    accent("M12 10.4c0-3.6 2.4-6 6.5-6 0 3.8-2.4 6-6.5 6Z"),
    line("M12 10.4c0-3.6 2.4-6 6.5-6 0 3.8-2.4 6-6.5 6Z"),
    body("M12 13c0-3.4-2.4-5.6-6.5-5.6 0 3.6 2.4 5.6 6.5 5.6Z"),
    line("M12 18V9.6"),
    line("M4 20.5c2-1.6 4.8-2.5 8-2.5s6 .9 8 2.5"),
  ],
  /** Fighting: a boxing glove with a slime cuff. */
  fighting: [
    accent(rect(7.6, 16.4, 9.4, 4.6, 1.2)),
    line(rect(7.6, 16.4, 9.4, 4.6, 1.2)),
    body("M8 16.4 6.6 13.2V8.6a5.6 5.6 0 0 1 5.6-5.6h1.3a5.4 5.4 0 0 1 5.4 5.4v4.4a3.8 3.8 0 0 1-1.4 2.9l-.9.7"),
    line("M6.6 10.6H5.4a1.8 1.8 0 0 0-1.8 1.8v.3a3.6 3.6 0 0 0 3.6 3.6h4.4"),
    line("M11 9.4c1.6.7 3.6.7 5.4 0"),
  ],
  /** Horror: a skull, slime running from one eye. */
  horror: [
    dripPart(15, 12.3, 4.4, 1.15),
    accent(circle(15, 10.8, 1.8)),
    body("M12 3a7.5 7.5 0 0 0-7.5 7.5c0 2.5 1.2 4.4 3 5.6v3A1.5 1.5 0 0 0 9 20.6h6a1.5 1.5 0 0 0 1.5-1.5v-3c1.8-1.2 3-3.1 3-5.6A7.5 7.5 0 0 0 12 3Z"),
    dot(circle(9, 10.8, 1.8)),
    line("M10.6 20.4v-2.2M13.4 20.4v-2.2"),
  ],
  /** Idle: a stack of coins, another dropping on in slime. */
  idle: [
    accent(circle(17.8, 5.4, 2.6)),
    body("M3.6 11.6v7.2c0 1.2 2.7 2.2 6 2.2s6-1 6-2.2v-7.2"),
    body(ellipse(9.6, 11.6, 6, 2.2)),
    line("M3.6 14.4c0 1.2 2.7 2.2 6 2.2s6-1 6-2.2M3.6 17.1c0 1.2 2.7 2.2 6 2.2"),
  ],
  /** Metroidvania: a map of connected rooms, the save room slime. */
  metroidvania: [
    accent(rect(11, 14.6, 4, 5.8, 1)),
    body(rect(2.8, 3.6, 12, 5, 1.2)),
    body(rect(17, 3.6, 4.2, 16.8, 1.2)),
    body(rect(2.8, 12.6, 5.8, 7.8, 1.2)),
    line(rect(11, 14.6, 4, 5.8, 1)),
    line("M14.8 6.1H17M5.7 8.6v4M8.6 17.5H11M15 17.5h2"),
  ],
  /** Open world: a signpost pointing every way, one sign slime. */
  "open-world": [
    accent("M18.4 11.6h-12L3.8 14.1l2.6 2.5h12Z"),
    line("M18.4 11.6h-12L3.8 14.1l2.6 2.5h12Z"),
    body("M5.6 4.2h12l2.6 2.5-2.6 2.5h-12Z"),
    line("M12 2.8v1.4M12 9.2v2.4M12 16.6v4.6M8.6 21.2h6.8"),
  ],
  /** Party: a party popper bursting slime confetti. */
  party: [
    accent(circle(14.6, 4, 1.1)),
    accent(circle(20, 9.4, 1.1)),
    accent(circle(18.6, 4.8, 0.9)),
    body("M3.4 20.6 7.8 8.4l7.8 7.8Z"),
    line("M5.6 14.4l4 4M11.6 11.6c.6-2.6 2.4-4 4.4-4M12.4 12.4c1.8-.4 3.6.4 4.8 2"),
  ],
  /** Physics: a seesaw, a crate on one end and a slime ball on the other. */
  physics: [
    accent(circle(17.1, 7.6, 2.4)),
    line(circle(17.1, 7.6, 2.4)),
    line("M3 15.2 21 10"),
    body("M12 12.6 15.2 19H8.8Z"),
    body(roundPoly(place([[-2.2, -2.2], [2.2, -2.2], [2.2, 2.2], [-2.2, 2.2]], 5.1, 11.3, -16.1), 0.6)),
  ],
  /** Platformer: a jump from one ledge to the next, the player slime. */
  platformer: [
    accent(rect(11, 3.4, 3.6, 3.6, 0.8)),
    body(rect(2.8, 16.4, 7.4, 3.6, 1)),
    body(rect(13.8, 12.4, 7.4, 3.6, 1)),
    line("M5.6 13.6C6.2 9.6 7.6 7.2 9.4 5.9"),
    dot(circle(16.6, 6.2, 0.9)),
    dot(circle(18.2, 8.8, 0.9)),
  ],
  /** Point & click: a pointer, its click in slime. */
  "point-and-click": [
    accentLine("M9 2.8v1.8M4.4 7.4h1.8M5.8 4.2l1.2 1.2M12.2 4.2 11 5.4"),
    body("M9 7.4v12.4l3.2-3.1 2.4 5.1 2.4-1.1-2.4-5h4.6Z"),
  ],
  /** Puzzle: a jigsaw piece, dripping slime. */
  puzzle: [
    dripPart(8.6, 18.6, 3.8, 1.35),
    body("M4 6h4.3a2.4 2.4 0 1 1 4.4 0H17v4.3a2.4 2.4 0 1 1 0 4.4v3.9H4Z"),
  ],
  /** Racing: a steering wheel, its grip slime. */
  racing: [
    body(circle(12, 12, 8.8)),
    accentLine(arc(12, 12, 8.8, 215, 325)),
    line(circle(12, 12.6, 2.4)),
    line("M3.4 11.2 9.6 12M14.4 12l6.2-.8M12 15v5.8"),
  ],
  /** Rhythm: a drum and its sticks, the drum skin slime. */
  rhythm: [
    accent(ellipse(12, 11, 7.6, 2.6)),
    body("M4.4 11v6.6c0 1.4 3.4 2.6 7.6 2.6s7.6-1.2 7.6-2.6V11"),
    line(ellipse(12, 11, 7.6, 2.6)),
    line("M5.4 3.2l5.2 5.6M18.6 3.2l-5.2 5.6M8 15.4v3.6M16 15.4v3.6"),
  ],
  /** Roguelike: a dungeon arch over stairs going down, slime dripping from its keystone. */
  roguelike: [
    dripPart(12, 4, 4.4, 1.4),
    body("M4 20.6V11.2a8 8 0 0 1 16 0v9.4"),
    line("M4 20.6h4v-2.8h4v-2.8h4v-2.8h4"),
    line("M2.6 20.6h18.8"),
  ],
  /** RPG: a round potion, full of slime. */
  rpg: [
    accent("M6.72 14c1.8-1 3.5.9 5.3 0s3.5-1 5.26 0A5.3 5.3 0 1 1 6.72 14Z"),
    body("M10 4.4v4.3A6.2 6.2 0 1 0 14 8.7V4.4"),
    line(rect(8.8, 2.4, 6.4, 2.4, 0.9)),
  ],
  /** Sandbox: a bucket and a spade, the spade slime. */
  sandbox: [
    accent("M15.8 12h4.4v3.4a2.2 2.2 0 0 1-4.4 0Z"),
    line("M15.8 12h4.4v3.4a2.2 2.2 0 0 1-4.4 0Z"),
    body("M3 10h10l-1.4 10.6H4.4Z"),
    line("M4.4 10c0-4.2 7.2-4.2 7.2 0M18 3v9"),
  ],
  /** Shooter: a crosshair on a slime dot. */
  shooter: [accent(circle(12, 12, 2)), body(circle(12, 12, 7.4)), line("M12 2.8v4.6M12 16.6v4.6M2.8 12h4.6M16.6 12h4.6")],
  /** Simulation: a little world in a snow globe, its house slime. */
  simulation: [
    accent("M9.4 14.4v-2.6L12 9.6l2.6 2.2v2.6Z"),
    body(circle(12, 10.2, 7.4)),
    line("M7 15.6 5.6 21h12.8L17 15.6"),
    line("M6.8 14.4h10.4"),
  ],
  /** Sports: a ball, bouncing off a drop of slime. */
  sports: [
    dripPart(12, 19, 3.4, 1.35),
    body(circle(12, 11, 8.2)),
    line("M12 2.8v16.4M3.8 11h16.4M6.2 5.2c3.2 3.2 3.2 8.4 0 11.6M17.8 5.2c-3.2 3.2-3.2 8.4 0 11.6"),
  ],
  /** Stealth: footprints tiptoeing, the newest in slime. */
  stealth: [
    accent(smooth(place(SOLE, 16, 10, 12))),
    ...place(TOES, 16, 10, 12).map((p) => accent(circle(p[0], p[1], 0.95))),
    body(smooth(place(SOLE, 8, 15, -8, true))),
    ...place(TOES, 8, 15, -8, true).map((p) => dot(circle(p[0], p[1], 0.95))),
  ],
  /** Strategy: a chess knight with a slime eye. */
  strategy: [
    accent(circle(12.3, 8, 1.1)),
    body("M8.4 18c.4-2.2 1.8-3.8 3.2-5-1.4-.3-2.6.2-3.8.8-.9.5-2 .3-2.6-.5l-.5-.6c-.5-.7-.5-1.6.1-2.3L9.2 6c.8-.8 1.6-1.4 2.6-1.7l.5-1.7 1.4 1.5c2.9.7 4.6 3.5 4.6 7.6V18"),
    body(rect(6, 18, 12.4, 3, 1)),
  ],
  /** Survival: a campfire with a slime heart to its flame. */
  survival: [
    accent("M12 15.8a2 2 0 0 0 2-2c0-1.2-.8-2.2-2-3.2-1.2 1-2 2-2 3.2a2 2 0 0 0 2 2Z"),
    body("M12 3c3.5 3 5 5.5 5 8.4a5 5 0 0 1-10 0c0-1.6.7-2.8 1.6-3.8.2 1.3.9 2.2 2 2.6-.4-2.8.3-5.2 1.4-7.2Z"),
    line("M4 20.8l16-3.6M20 20.8 4 17.2"),
  ],
  /** Tower defense: a tower with a slime window. */
  "tower-defense": [
    accent("M10.6 15.2v-2.6a1.4 1.4 0 0 1 2.8 0v2.6Z"),
    body("M6 3.2h2.4v2.4h2.4V3.2h2.4v2.4h2.4V3.2H18V9l-1.6 1.6v10.2H7.6V10.6L6 9Z"),
    line("M6.4 9h11.2M10.2 20.8v-2.4a1.8 1.8 0 0 1 3.6 0v2.4"),
  ],
  /** Trivia: a quiz buzzer with a slime button. */
  trivia: [
    accent("M6.2 15.6a5.8 5.8 0 0 1 11.6 0Z"),
    line("M6.2 15.6a5.8 5.8 0 0 1 11.6 0"),
    body(rect(3.6, 15.6, 16.8, 4.4, 1.4)),
    line("M12 3.2v2.6M5.4 5.6l1.6 1.6M18.6 5.6 17 7.2"),
  ],
  /** Tycoon: a top hat with a slime band. */
  tycoon: [
    accent("M7.6 11.6h8.8v3.4H7.6Z"),
    body("M7.6 15.4V5.4A1.6 1.6 0 0 1 9.2 3.8h5.6a1.6 1.6 0 0 1 1.6 1.6v10"),
    body(rect(3, 15.4, 18, 3.6, 1.8)),
    line("M7.6 11.6h8.8"),
  ],
  /** Visual novel: a scene and its dialogue box in slime. */
  "visual-novel": [
    accent(rect(5, 13.8, 14, 4.4, 1)),
    body(rect(2.8, 3.6, 18.4, 16.6, 2.4)),
    line(circle(12, 8.4, 2.4)),
    line("M8.2 13.6c.5-1.6 2-2.4 3.8-2.4s3.3.8 3.8 2.4"),
    line(rect(5, 13.8, 14, 4.4, 1)),
    line("M7.6 16h5.6"),
  ],
  /** Word game: crossing words in a grid, one square slime. */
  "word-game": [
    accent("M9 9h6v6H9Z"),
    dot("M15 3h3.8A2.2 2.2 0 0 1 21 5.2V9h-6Z"),
    dot("M3 15h6v6H5.2A2.2 2.2 0 0 1 3 18.8Z"),
    body(rect(3, 3, 18, 18, 2.2)),
    line("M9 3v18M15 3v18M3 9h18M3 15h18"),
  ],

  // ---- kids & family -----------------------------------------------------
  /** All ages: a rosette with a slime tick. */
  "all-ages": [
    accentLine("M9.4 9.6l1.8 1.8 3.6-3.6"),
    body(circle(12, 9.6, 6.2)),
    line("M8.6 14.8 7.2 21l4.8-2.4 4.8 2.4-1.4-6.2"),
  ],
  /** Educational: a light bulb with a slime filament. */
  educational: [
    accentLine("M10.4 16.2v-3.4a1.6 1.6 0 0 1 3.2 0v3.4"),
    body("M9 16.4c0-2-3.4-3.6-3.4-7.4a6.4 6.4 0 0 1 12.8 0c0 3.8-3.4 5.4-3.4 7.4Z"),
    line("M9.2 19h5.6M10.2 21.4h3.6"),
  ],
  /** Family: a grown-up and a child, the child slime. */
  family: [
    accent(circle(16.6, 11.4, 2)),
    body(circle(8, 5.6, 2.5)),
    body("M4 21v-7a4 4 0 0 1 8 0v7"),
    line(circle(16.6, 11.4, 2)),
    body("M13.6 21v-3.4a3 3 0 0 1 6 0V21"),
  ],
  /** Kids: a kite, one quarter slime, its tail flying. */
  kids: [
    accent("M12 2.8 12 9.2 6.4 9.2Z"),
    body("M12 2.8 17.6 9.2 12 16.4 6.4 9.2Z"),
    line("M12 2.8v13.6M6.4 9.2h11.2"),
    line("M12 16.4c-1.4 1.4.6 2.4-.8 3.8s-2.6.6-3.6 1"),
  ],

  // ---- tools -------------------------------------------------------------
  /** AI assistant: a robot with a slime light. */
  "ai-assistant": [
    accent(circle(12, 3.8, 1.6)),
    body(rect(5, 8.4, 14, 11.6, 3.2)),
    line("M12 8.4V5.4M3 12.4v3.4M21 12.4v3.4M10 16.4h4"),
    dot(circle(9.2, 12.8, 1.35)),
    dot(circle(14.8, 12.8, 1.35)),
  ],
  /** Automation: a flowchart, the decision slime. */
  automation: [
    accent("M12 9.8 15.4 13.2 12 16.6 8.6 13.2Z"),
    body(rect(8, 2.8, 8, 4.4, 1.4)),
    line("M12 7.2v2.6M12 9.8 15.4 13.2 12 16.6 8.6 13.2Z"),
    line("M8.6 13.2H5.4v3.4M15.4 13.2h3.2v3.4"),
    body(circle(5.4, 18.8, 2)),
    body(circle(18.6, 18.8, 2)),
  ],
  /** Calendar: a page of days, one slime. */
  calendar: [
    accent(rect(13.4, 13.2, 3.6, 3.6, 0.8)),
    body(rect(3.5, 5, 17, 15.6, 2.4)),
    line("M3.5 9.8h17M8 3v4M16 3v4"),
    dot(circle(8, 15, 1)),
  ],
  /** Communication: a headset with a slime mic. */
  communication: [
    accent(rect(10.4, 18.8, 3.6, 2.4, 1.2)),
    line("M4.5 13V11.6a7.5 7.5 0 0 1 15 0V13"),
    body(rect(3.2, 11.8, 4, 6, 1.6)),
    body(rect(16.8, 11.8, 4, 6, 1.6)),
    line("M18.8 17.8c0 1.6-1.4 2.2-3 2.2H14"),
  ],
  /** Developer tools: a terminal with a slime cursor. */
  "developer-tools": [accentLine("M12.2 15h4.6"), body(rect(2.8, 4, 18.4, 16, 2.4)), line("M7 9l3 2.6-3 2.6")],
  /** File management: a folder, dripping slime. */
  "file-management": [
    dripPart(15.4, 18.6, 3.4, 1.3),
    body("M3 6a2 2 0 0 1 2-2h3.8l2 2.4H19a2 2 0 0 1 2 2v8.2a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"),
    line("M3 9.4h18"),
  ],
  /** Finance: a wallet with a slime coin at the clasp. */
  finance: [
    accent(circle(16.6, 13.2, 1.3)),
    body(rect(3, 6.6, 17.6, 13.4, 2.4)),
    line("M20.6 10.4H16.4a2.8 2.8 0 0 0 0 5.6h4.2"),
    line("M5.4 6.6 14 3.6a1.6 1.6 0 0 1 2 1.1l.5 1.9"),
  ],
  /** Health & fitness: a dumbbell with a slime grip. */
  "health-fitness": [
    accentLine("M8.6 12h6.8"),
    body(rect(5, 6.4, 3.6, 11.2, 1.4)),
    body(rect(15.4, 6.4, 3.6, 11.2, 1.4)),
    line("M5 9.4H3.8a1 1 0 0 0-1 1v3.2a1 1 0 0 0 1 1H5M19 9.4h1.2a1 1 0 0 1 1 1v3.2a1 1 0 0 1-1 1H19"),
  ],
  /** Note-taking: a spiral notepad, the last line slime. */
  "note-taking": [
    accentLine("M8.4 17.4h4.4"),
    body(rect(4.6, 4.6, 14.8, 16.6, 2.2)),
    line("M8.4 2.8v3.6M12 2.8v3.6M15.6 2.8v3.6M8.4 10.4h7.2M8.4 13.9h7.2"),
  ],
  /** Office: a briefcase with a slime clasp. */
  office: [
    accent(rect(10.4, 11.2, 3.2, 3.2, 0.8)),
    body(rect(3, 7.4, 18, 12.8, 2.4)),
    line("M9 7.4V5.6A1.6 1.6 0 0 1 10.6 4h2.8A1.6 1.6 0 0 1 15 5.6v1.8M3 12.8h7.4M13.6 12.8H21"),
  ],
  /** Productivity: a checklist, ticked in slime. */
  productivity: [
    accentLine("M4.6 6.8l1.6 1.6 3.4-3.6"),
    body(rect(3.4, 4.6, 5.4, 5.4, 1.2)),
    body(rect(3.4, 14, 5.4, 5.4, 1.2)),
    line("M12 7.3h8.6M12 16.7h8.6"),
  ],

  // ---- creative ----------------------------------------------------------
  /** 3D: a cube, its top face slime. */
  "3d": [
    accent("M12 2.8 20 7.4 12 12 4 7.4Z"),
    body("M12 2.8 20 7.4v9.2L12 21.2 4 16.6V7.4Z"),
    line("M4 7.4 12 12l8-4.6M12 12v9.2"),
  ],
  /** Animation: a ball moving, drawn frame by frame, the latest slime. */
  animation: [
    accent(circle(16.6, 8.2, 3.6)),
    line(arc(11, 11.8, 3.6, 66, 236)),
    line(arc(5.6, 15.4, 3.6, 76, 226)),
    body(circle(16.6, 8.2, 3.6)),
  ],
  /** Audio editing: a waveform with a stretch selected in slime. */
  "audio-editing": [
    accentLine("M13.7 5v14M17.1 8.4v7.2"),
    line("M3.5 10.2v3.6M6.9 7v10M10.3 3.4v17.2M20.5 10.6v2.8"),
  ],
  /** Design: shapes laid out, the triangle slime. */
  design: [
    accent("M17.3 3.4 21 10.2H13.6Z"),
    line("M17.3 3.4 21 10.2H13.6Z"),
    body(circle(7.8, 7.8, 4.6)),
    body(rect(8, 13.4, 8, 7.6, 1.4)),
  ],
  /** Drawing: a pencil and its slime scribble. */
  drawing: [
    accentLine("M12.6 20.4c1.4-1.2 2.4-1.2 3.4 0s2.2 1.2 3.6 0"),
    body("M15.4 3.8a2.1 2.1 0 0 1 3 0l1.6 1.6a2.1 2.1 0 0 1 0 3L8.4 20 3.2 20.8 4 15.6Z"),
    line("M13.4 5.8l4.8 4.8M4 15.6l4.4 4.4"),
  ],
  /** Game making: a d-pad and a slime cog. */
  "game-making": [
    accent(cog(17.8, 17.8, 3.6, 2.5, 6)),
    body(roundPoly([[9.4, 2.8], [14.6, 2.8], [14.6, 9.4], [21.2, 9.4], [21.2, 14.6], [14.6, 14.6], [14.6, 21.2], [9.4, 21.2], [9.4, 14.6], [2.8, 14.6], [2.8, 9.4], [9.4, 9.4]], 1.2)),
    dot(circle(12, 12, 1.2)),
  ],
  /** Generative art: a magic wand with a slime sparkle. */
  "generative-art": [
    accent(sparkle(16.6, 7.4, 4.4)),
    line("M3.6 20.4 12.6 11.4"),
    dot(circle(6.4, 6, 1)),
    dot(circle(19.4, 16.2, 1)),
  ],
  /** Music: two notes, their beam slime. */
  music: [
    accent("M9 5.2 19.2 2.8v3.2L9 8.4Z"),
    body(circle(6.6, 18, 2.4)),
    body(circle(16.8, 15.6, 2.4)),
    line("M9 18V6M19.2 15.6V3.6"),
  ],
  /** Photo editing: a picture with a slime sun. */
  "photo-editing": [
    accent(circle(15.6, 9, 1.9)),
    body(rect(2.8, 4.4, 18.4, 15.2, 2.4)),
    line("M3.4 17.4l5.2-5.6 4 4 2.6-2.6 5.4 5.4"),
  ],
  /** Pixel art: a pixel heart with a slime highlight. */
  "pixel-art": [
    accent(rect(5.5, 6.8, 2.6, 2.6, 0)),
    body(poly([[5.5, 4.2], [10.7, 4.2], [10.7, 6.8], [13.3, 6.8], [13.3, 4.2], [18.5, 4.2], [18.5, 6.8], [21.1, 6.8], [21.1, 12], [18.5, 12], [18.5, 14.6], [15.9, 14.6], [15.9, 17.2], [13.3, 17.2], [13.3, 19.8], [10.7, 19.8], [10.7, 17.2], [8.1, 17.2], [8.1, 14.6], [5.5, 14.6], [5.5, 12], [2.9, 12], [2.9, 6.8], [5.5, 6.8]])),
  ],
  /** Video: a video camera, recording in slime. */
  video: [
    accent(circle(6.6, 10, 1.4)),
    body(rect(2.8, 6.4, 12.6, 11.2, 2.4)),
    line("M15.4 10.4l5.8-3.2v9.6l-5.8-3.2"),
  ],
  /** Writing: a quill, ink dripping in slime. */
  writing: [
    dripPart(5, 18.4, 3.6, 1.3),
    body("M7.2 16.6C7.8 9.6 12.4 4.6 20.2 3.6c-.8 7.8-5.8 12.6-13 13Z"),
    line("M5 18.8 14.4 9.4"),
  ],

  // ---- utilities ---------------------------------------------------------
  /** Accessibility: a person with open arms in a circle, their head slime. */
  accessibility: [
    accent(circle(12, 7.2, 1.7)),
    body(circle(12, 12, 9)),
    line("M7.2 10.2 12 11.2l4.8-1M12 11.2v3.4l-2.6 4.4M12 14.6l2.6 4.4"),
  ],
  /** Backup: a cloud, a slime arrow up into it. */
  backup: [
    accentLine("M12 19.6V12.4M9 15.2l3-3 3 3"),
    body("M7.4 17.6H6.6a4 4 0 0 1-.4-8 6 6 0 0 1 11.4-1.2 4.6 4.6 0 0 1 .2 9.2h-1.2"),
  ],
  /** Customization: sliders, one knob slime. */
  customization: [
    accent(circle(15.4, 12, 2.2)),
    line("M3.2 6h3.2M11 6h9.8M3.2 12h9.8M17.8 12h3M3.2 18h6.2M14 18h6.8"),
    body(circle(8.6, 6, 2.2)),
    line(circle(15.4, 12, 2.2)),
    body(circle(11.6, 18, 2.2)),
  ],
  /** Hardware: a chip with a slime core. */
  hardware: [
    accent(rect(9.6, 9.6, 4.8, 4.8, 1)),
    body(rect(5.8, 5.8, 12.4, 12.4, 2)),
    line("M9.6 2.8v3M14.4 2.8v3M9.6 18.2v3M14.4 18.2v3M2.8 9.6h3M2.8 14.4h3M18.2 9.6h3M18.2 14.4h3"),
  ],
  /** Launcher: a rocket lifting off on slime. */
  launcher: [
    dripPart(12, 16.6, 5, 1.6),
    body("M12 2.8c3 2.2 4.4 5.4 4.4 9.2v4.6H7.6V12c0-3.8 1.4-7 4.4-9.2Z"),
    line("M7.6 12.4 4.6 15.4v3.2l3-1.6M16.4 12.4l3 3v3.2l-3-1.6"),
    line(circle(12, 9.4, 1.7)),
  ],
  /** Media player: a play button, slime. */
  "media-player": [accent("M10 8.2 16 12l-6 3.8Z"), body(circle(12, 12, 9)), line("M10 8.2 16 12l-6 3.8Z")],
  /** Networking: a hub and three machines, the hub slime. */
  networking: [
    accent(circle(12, 12.6, 2.6)),
    line(circle(12, 12.6, 2.6)),
    body(circle(12, 4.6, 2)),
    body(circle(4.8, 18.4, 2)),
    body(circle(19.2, 18.4, 2)),
    line("M12 6.6V10M6.4 17.1l3.5-2.8M17.6 17.1l-3.5-2.8"),
  ],
  /** Privacy: an eye, crossed out in slime. */
  privacy: [
    body("M2.8 12C5 7.8 8.2 5.6 12 5.6s7 2.2 9.2 6.4c-2.2 4.2-5.4 6.4-9.2 6.4S5 16.2 2.8 12Z"),
    line(circle(12, 12, 2.6)),
    accentLine("M4.4 4.4l15.2 15.2"),
  ],
  /** Screenshots: a viewfinder's corners round a slime shutter. */
  screenshots: [
    accent(circle(12, 12, 3)),
    line("M3 8.4V5.2A2.2 2.2 0 0 1 5.2 3h3.2M15.6 3h3.2A2.2 2.2 0 0 1 21 5.2v3.2M21 15.6v3.2a2.2 2.2 0 0 1-2.2 2.2h-3.2M8.4 21H5.2A2.2 2.2 0 0 1 3 18.8v-3.2"),
  ],
  /** Security: a padlock with a slime keyhole. */
  security: [
    accent(circle(12, 14.6, 1.6)),
    accent("M11.2 15.2h1.6v2.6h-1.6Z"),
    body(rect(4.6, 10.4, 14.8, 10.6, 2.4)),
    line("M7.6 10.4V7.4a4.4 4.4 0 0 1 8.8 0v3"),
  ],
  /** System monitor: a screen with a slime pulse. */
  "system-monitor": [
    accentLine("M5.6 10.4h2.8l1.6-3 3 6 1.6-3h3.8"),
    body(rect(2.8, 3.4, 18.4, 13.2, 2.2)),
    line("M8.4 20.8h7.2M12 16.6v4.2"),
  ],

  // ---- learning ----------------------------------------------------------
  /** Coding: a laptop with code on it, one line slime. */
  coding: [
    accentLine("M9.8 11h5"),
    body(rect(4.4, 4, 15.2, 11, 1.8)),
    line("M2.6 19.2h18.8M7.4 7.8h4.6"),
    line("M4.4 15 2.6 19.2M19.6 15l1.8 4.2"),
  ],
  /** Geography: a desk globe, its equator slime. */
  geography: [
    accentLine("M5.4 9.6h13.2"),
    body(circle(12, 9.6, 6.6)),
    line(ellipse(12, 9.6, 6.6, 2.8, 90)),
    line("M18.4 3.8a8.6 8.6 0 0 1-11.6 12.4M12 18.2v2.6M8.4 21h7.2"),
  ],
  /** History: a rolled scroll sealed in slime. */
  history: [
    accent(circle(15.6, 16.4, 2)),
    body("M6.6 6.2V17a2.6 2.6 0 0 0 2.6 2.6h8.4"),
    body("M17.4 17.8V6.4A2.6 2.6 0 0 0 14.8 3.8H5.2a2.6 2.6 0 0 0 0 5.2h1.4"),
    line("M17.6 19.6a2.6 2.6 0 0 0 2.6-2.6v-1.4h-2.8M9.4 8.4h5M9.4 11.4h5"),
  ],
  /** Language: two speech bubbles, one slime. */
  language: [
    accent("M11 9.6h8.2a2 2 0 0 1 2 2v4.6a2 2 0 0 1-2 2h-.8v2.6l-3.4-2.6H11a2 2 0 0 1-2-2v-4.6a2 2 0 0 1 2-2Z"),
    line("M11 9.6h8.2a2 2 0 0 1 2 2v4.6a2 2 0 0 1-2 2h-.8v2.6l-3.4-2.6H11a2 2 0 0 1-2-2v-4.6a2 2 0 0 1 2-2Z"),
    body("M6.8 13.2H6.2l-2.4 2V13.2a2 2 0 0 1-1-1.7V5.6a2 2 0 0 1 2-2h8.8a2 2 0 0 1 2 2v1.8"),
  ],
  /** Math: a calculator with a slime display. */
  math: [
    accent(rect(7.8, 5.6, 8.4, 3.4, 0.8)),
    body(rect(4.8, 2.8, 14.4, 18.4, 2.4)),
    dot(circle(8.6, 12.8, 1.1)),
    dot(circle(12, 12.8, 1.1)),
    dot(circle(15.4, 12.8, 1.1)),
    dot(circle(8.6, 17, 1.1)),
    dot(circle(12, 17, 1.1)),
    dot(circle(15.4, 17, 1.1)),
  ],
  /** Reading: a book with a slime bookmark. */
  reading: [
    accent("M13.6 3.4v6.4l2-1.4 2 1.4V3.4Z"),
    body("M5 5a1.6 1.6 0 0 1 1.6-1.6H19v14.4H6.6A1.6 1.6 0 0 0 5 19.4Z"),
    line("M5 19.4A1.6 1.6 0 0 0 6.6 21H19v-3.2"),
  ],
  /** Science: an atom with a slime nucleus. */
  science: [
    accent(circle(12, 12, 2)),
    body(ellipse(12, 12, 9.2, 3.6, 0)),
    line(ellipse(12, 12, 9.2, 3.6, 60)),
    line(ellipse(12, 12, 9.2, 3.6, 120)),
  ],
  /** Typing: a keyboard with a slime space bar. */
  typing: [
    accentLine("M8.2 15.6h7.6"),
    body(rect(2.6, 6, 18.8, 12.4, 2.2)),
    ...[6.2, 9.4, 12.6, 15.8].map((x) => dot(circle(x + 1, 9.4, 0.95))),
    ...[7.8, 11, 14.2, 17.4].map((x) => dot(circle(x - 1, 12.4, 0.95))),
  ],

  // ---- play style --------------------------------------------------------
  /** Controller support: a plain gamepad on a cable with a slime plug. */
  "controller-support": [
    accent(rect(15.6, 3.2, 4.4, 2.8, 0.8)),
    body(rect(2.8, 9.8, 18.4, 10.4, 3.4)),
    line("M12 9.8V8.2c0-2 1.4-3.6 3.6-3.6"),
    line("M5.8 15h4.4M8 12.8v4.4"),
    dot(circle(15.4, 16.2, 1.2)),
    dot(circle(18, 13.8, 1.2)),
  ],
  /** Co-op: two players and a slime heart between them. */
  "co-op": [accent(heart(12, 4.2, 5)), ...bust(6.4, 7.4, 2.6, 7.6, 20.6), ...bust(17.6, 7.4, 2.6, 7.6, 20.6)],
  /** Keyboard only: the arrow keys, one slime. */
  "keyboard-only": [
    accent(sq(12, 8.6, 5.6, 1.2)),
    line(sq(12, 8.6, 5.6, 1.2)),
    body(sq(5.6, 15.4, 5.6, 1.2)),
    body(sq(12, 15.4, 5.6, 1.2)),
    body(sq(18.4, 15.4, 5.6, 1.2)),
  ],
  /** Multiplayer: a crowd of three, the one in front slime. */
  multiplayer: [
    accent(circle(12, 9.6, 2.8)),
    line(circle(12, 9.6, 2.8)),
    line("M7 20.6v-.6a4.4 4.4 0 0 1 4.4-4.4h1.2a4.4 4.4 0 0 1 4.4 4.4v.6"),
    body(circle(5.2, 6.4, 2.2)),
    body(circle(18.8, 6.4, 2.2)),
    line("M2.6 15.6v-.6a3.6 3.6 0 0 1 3.6-3.6h1M21.4 15.6v-.6a3.6 3.6 0 0 0-3.6-3.6h-1"),
  ],
  /** Offline: no signal, crossed out in slime. */
  offline: [
    line(arc(12, 18, 12, 225, 315)),
    line(arc(12, 18, 7.6, 225, 315)),
    dot(circle(12, 17.4, 1.3)),
    accentLine("M4 4l16 16"),
  ],
  /** PvP: crossed swords, one slime. */
  pvp: [
    line("M3.6 3.6 14.6 14.6M12.6 16.6l4-4M15.4 15.4l3.4 3.4"),
    dot(circle(19.8, 19.8, 1.2)),
    accentLine("M20.4 3.6 9.4 14.6M11.4 16.6l-4-4M8.6 15.4l-3.4 3.4"),
    dot(circle(4.2, 19.8, 1.2)),
  ],
  /** Real-time: a stopwatch running, its hand slime. */
  "real-time": [
    accentLine("M12 13.6 15.2 10"),
    body(circle(12, 13.6, 7.6)),
    line("M9.6 2.8h4.8M12 2.8v3.2M18.6 6.8l1.4-1.4"),
    dot(circle(12, 13.6, 1.2)),
  ],
  /** Short sessions: a little timer, a short slice of it slime. */
  "short-sessions": [accent("M12 12V4.6a7.4 7.4 0 0 1 6.4 3.7Z"), body(circle(12, 12, 8.8)), line("M12 12V3.2M12 12l7.6-4.4")],
  /** Single-player: one player, their head slime. */
  "single-player": [accent(circle(12, 7.6, 3.6)), ...bust(12, 7.6, 3.6, 13.2, 20.8)],
  /** Split-screen: a screen split in two, one half slime. */
  "split-screen": [
    accent("M12 4.6H5a2.2 2.2 0 0 0-2.2 2.2v10.4A2.2 2.2 0 0 0 5 19.4h7Z"),
    body(rect(2.8, 4.6, 18.4, 14.8, 2.2)),
    line("M12 4.6v14.8"),
  ],
  /** Touch controls: a phone, tapped in slime. */
  "touch-controls": [
    accent(circle(12, 10.4, 1.7)),
    accentLine(arc(12, 10.4, 4, 200, 340)),
    body(rect(6.2, 2.8, 11.6, 18.4, 2.4)),
    line("M10.4 18.2h3.2"),
  ],
  /** Turn-based: two arrows taking turns, one slime. */
  "turn-based": [line("M3.6 8h15.6M15.6 4.4 19.2 8l-3.6 3.6"), accentLine("M20.4 16H4.8M8.4 12.4 4.8 16l3.6 3.6")],

  // ---- vibe --------------------------------------------------------------
  /** Atmospheric: gusts of wind, one slime. */
  atmospheric: [
    accentLine("M3.6 13.2h13.4a2.8 2.8 0 1 1-2.8 2.8"),
    line("M3.6 9.2h8.6a2.6 2.6 0 1 0-2.6-2.6M3.6 17.2h6"),
  ],
  /** Chaotic: a scribbled tangle with a slime splat. */
  chaotic: [
    accent(roundPoly(place([[-1.5, -1.5], [1.5, -1.5], [1.5, 1.5], [-1.5, 1.5]], 19.6, 15.2, 24), 0.4)),
    accent(circle(4.8, 15.6, 1.1)),
    line("M3 4.4c4 1 14 1 18 0M5.4 8.6c3 .8 9.6.8 12.4-.2M8.2 12.8c2 .6 5.6.6 7.4-.2M8.8 17c1.4.4 3.4.4 4.6 0M10.6 20.8h1.6"),
  ],
  /** Cozy: a warm mug, its steam slime. */
  cozy: [
    accentLine("M8.4 7.6c-1-1.2.8-1.8 0-3M12.4 7.6c-1-1.2.8-1.8 0-3"),
    body("M4.4 10.4h12v6a4.4 4.4 0 0 1-4.4 4.4H8.8a4.4 4.4 0 0 1-4.4-4.4Z"),
    line("M16.4 12h1.4a2.6 2.6 0 0 1 0 5.2h-1.4"),
  ],
  /** Cute: a cat's face with slime cheeks. */
  cute: [
    accent(ellipse(7, 15.4, 1.5, 1)),
    accent(ellipse(17, 15.4, 1.5, 1)),
    body("M4.6 10V3.8l4.4 3.4a9 9 0 0 1 6 0l4.4-3.4V10c.6 1 1 2.2 1 3.4 0 4.4-3.8 7.4-8.4 7.4s-8.4-3-8.4-7.4c0-1.2.4-2.4 1-3.4Z"),
    dot(circle(9, 12.2, 1.2)),
    dot(circle(15, 12.2, 1.2)),
    line("M10.8 15.8c.4.6.8.8 1.2.4.4.4.8.2 1.2-.4"),
  ],
  /** Dark: a crescent moon with a slime star. */
  dark: [accent(sparkle(17.6, 6.2, 3)), body("M19.4 15A8.4 8.4 0 1 1 9.4 4.2a6.6 6.6 0 0 0 10 10.8Z")],
  /** Funny: a face laughing out loud, mouth slime. */
  funny: [
    accent("M7.6 13.4h8.8a4.4 4.4 0 0 1-8.8 0Z"),
    body(circle(12, 12, 9)),
    line("M7.4 9.6l1.6-1.4 1.6 1.4M13.4 9.6l1.6-1.4 1.6 1.4M7.6 13.4h8.8a4.4 4.4 0 0 1-8.8 0Z"),
  ],
  /** Intense: a lightning bolt, half slime. */
  intense: [accent("M13.6 2.8 5 13.4h6.6Z"), body("M13.6 2.8 5 13.4h6.6l-1.2 7.8L19 10.6h-6.6Z")],
  /** Minimalist: a horizon and a small slime sun. That's all. */
  minimalist: [accent(circle(15.4, 9, 2.4)), line("M3.4 15.4h17.2")],
  /** Relaxing: a hammock of slime between two posts. */
  relaxing: [
    accent("M3.6 7.6c2.4 8.4 14.4 8.4 16.8 0-3.2 5-13.6 5-16.8 0Z"),
    line("M3.6 4.4v16.4M20.4 4.4v16.4M2.4 20.8h2.4M19.2 20.8h2.4"),
  ],
  /** Retro: a cassette tape with a slime window. */
  retro: [
    accent(rect(7.2, 8.4, 9.6, 5.2, 1.2)),
    body(rect(2.8, 4.6, 18.4, 14.8, 2.2)),
    line(circle(9, 11, 1.4)),
    line(circle(15, 11, 1.4)),
    line("M6.8 19.4l1.4-3h7.6l1.4 3"),
  ],
  /** Spooky: a ghost leaving a drop of slime. */
  spooky: [
    dripPart(14.4, 17.6, 3.8, 1.3),
    body("M5 18.6V9.6a7 7 0 0 1 14 0v9l-2.3-1.6-2.3 1.6-2.4-1.6-2.4 1.6-2.3-1.6Z"),
    dot(circle(9.6, 10, 1.3)),
    dot(circle(14.4, 10, 1.3)),
  ],
  /** Weird: a flying saucer with a slime beam. */
  weird: [
    accent("M9.6 13.6h4.8l3 6.4H6.6Z"),
    line("M8.4 8.6a3.6 3.6 0 0 1 7.2 0"),
    body(ellipse(12, 10.8, 9.2, 2.8)),
  ],
  /** Wholesome: a flower with a slime heart. */
  wholesome: [accent(circle(12, 12, 2.4)), body(flower(12, 12, 6, 4.6, 9.2))],
} satisfies Record<string, Part[]>;

/**
 * People's skills (crates/slop-api/src/people/mod.rs SKILLS), plus "everyone". Skills that are
 * also tags use the tag's drawing, so a skill looks the same wherever it shows.
 */
const SKILL_SHAPES = {
  /** Everyone: two people, the one behind with a slime head. */
  everyone: [
    accent(circle(16.6, 7.4, 2.6)),
    line(circle(16.6, 7.4, 2.6)),
    line("M14.4 12.4h4.4a3.2 3.2 0 0 1 3.2 3.2V19"),
    ...bust(9, 8.2, 3.2, 11.4, 20.6),
  ],
  /** 2D art: a palette with a dab of slime paint. */
  "2d-art": [
    accent(circle(16.9, 10.3, 1.6)),
    body("M12 3.2c-5 0-9 3.7-9 8.4 0 4.8 3.9 8.4 8.6 8.4 1.4 0 2-.8 2-1.7 0-1.3-1.2-1.7-1.2-2.9 0-1 .8-1.7 1.9-1.7h2.4c2.9 0 5.3-2 5.3-4.8 0-3.6-4.4-5.7-10-5.7Z"),
    dot(circle(7.3, 11.2, 1.3)),
    dot(circle(9.2, 7.4, 1.3)),
    dot(circle(13.6, 6.6, 1.3)),
  ],
  "pixel-art": TAG_SHAPES["pixel-art"],
  "3d": TAG_SHAPES["3d"],
  animation: TAG_SHAPES.animation,
  /** UI / UX: a window with a slime button, and the pointer about to press it. */
  "ui-ux": [
    accent(rect(5.8, 11, 7.4, 3.4, 1.2)),
    body(rect(2.8, 3.8, 18.4, 14.4, 2.2)),
    line("M2.8 7.8h18.4"),
    dot(circle(5.4, 5.8, 0.7)),
    body("M14.6 12.6 20.8 16.4 17.9 17.2 19.4 20.4 17.9 21.1 16.4 17.9 14.6 19.8Z"),
  ],
  music: TAG_SHAPES.music,
  /** Sound design: a speaker sending out slime waves. */
  sound: [
    accentLine(arc(12.2, 12, 4, -48, 48)),
    line(arc(12.2, 12, 7.8, -52, 52)),
    body("M2.8 9.2h3.8L11.6 5v14l-5-4.2H2.8Z"),
  ],
  writing: TAG_SHAPES.writing,
  /** Voice acting: a microphone, its grille slime. */
  voice: [
    accent(rect(10.2, 4, 3.6, 5.4, 1.6)),
    body(rect(8.6, 2.6, 6.8, 11.6, 3.4)),
    line("M5.4 11.2a6.6 6.6 0 0 0 13.2 0M12 17.8v3.4M8.6 21.2h6.8"),
  ],
  /** Localization: two speech bubbles, an A in one and the other slime. */
  localization: [
    accent(rect(10.6, 10.2, 10.6, 7.8, 2.2)),
    line(rect(10.6, 10.2, 10.6, 7.8, 2.2)),
    line("M17.6 18v3l-3-3"),
    body("M5 3.2h7.6a2.2 2.2 0 0 1 2.2 2.2v4.8H12.8a2.2 2.2 0 0 0-2.2 2.2v.4H8L5 15.4v-2.6a2.2 2.2 0 0 1-2.2-2.2V5.4A2.2 2.2 0 0 1 5 3.2Z"),
    line("M6.4 10.4l2.4-5 2.4 5M7.2 8.8h3.2"),
  ],
  /** Playtesting: a gamepad with a slime tick. */
  playtesting: [
    accentLine("M13.6 12.2l1.7 1.7 3.1-3.3"),
    body("M7.4 6.8h9.2a4.6 4.6 0 0 1 4.5 3.8l.9 5.2a2.4 2.4 0 0 1-4.1 2.1l-2.2-2.5H8.3l-2.2 2.5a2.4 2.4 0 0 1-4.1-2.1l.9-5.2a4.6 4.6 0 0 1 4.5-3.8Z"),
    line("M7.4 9.8v3.6M5.6 11.6h3.6"),
  ],
} satisfies Record<string, Part[]>;

export type SkillIconName = keyof typeof SKILL_SHAPES;

/** A People skill's icon by its slug; "everyone" for the whole directory, or a skill without one. */
export function skillIconPaths(slug: string, style: StoreIconStyle = STORE_ICON_STYLE): IconPath[] {
  const name = (slug in SKILL_SHAPES ? slug : "everyone") as SkillIconName;
  return iconPaths(SKILL_SHAPES[name], style);
}

/** Every People skill icon, for tests and previews. */
export const SKILL_ICON_NAMES = Object.keys(SKILL_SHAPES) as SkillIconName[];

export type TagIconName = keyof typeof TAG_SHAPES;

/** Every tag that has an icon of its own, in the order above. */
export const TAG_ICON_NAMES = Object.keys(TAG_SHAPES) as TagIconName[];

const KNOWN = new Set<string>(TAG_ICON_NAMES);

/** A tag's own icon, by its slug; null for a tag without one (it shows its category's). */
export function tagIconName(slug: string): TagIconName | null {
  return KNOWN.has(slug) ? (slug as TagIconName) : null;
}

/** The paths that draw a tag's icon in `style`, back to front. */
export function tagIconPaths(name: TagIconName, style: StoreIconStyle = STORE_ICON_STYLE): IconPath[] {
  return iconPaths(TAG_SHAPES[name], style);
}
