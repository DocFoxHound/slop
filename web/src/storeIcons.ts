// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's own store icons: the categories and the store's navigation (StoreNav.tsx), drawn
// by hand on a 24×24 grid. Plain data, no React, so the components (components/StoreIcons.tsx)
// and anything else can draw the same shapes. The tags' icons (tagIcons.ts) are drawn with the
// same pieces and helpers, and rendered the same way (iconPaths).
//
// One family, two styles drawn from the same shapes:
//   "drip": fine round lines with one touch of slime per icon, usually a drip hanging off an edge.
//   "goo":  chunky lines over a soft slime shadow of the icon's body, slightly off-register.
// The app uses STORE_ICON_STYLE; switching styles is that one line.
//
// Colours: lines are currentColor (they follow the text, hover and the "on" state); slime is
// var(--icon-accent) and the goo shadow var(--icon-tone), both set in styles.css per theme.

export type StoreIconStyle = "drip" | "goo";

/** The style the app draws. */
export const STORE_ICON_STYLE: StoreIconStyle = "drip";

/**
 * A piece of an icon:
 * - line: stroked in the text colour; `body` marks the closed outline the goo style shadows.
 * - dot: filled in the text colour.
 * - accent: filled in slime (the goo style fills it in the text colour).
 * - accentLine: stroked in slime (the goo style strokes it in the text colour).
 * - drip: slime hanging off an edge, drawn before the lines so the edge covers its top (drip style only).
 */
export type Part =
  | { as: "line"; d: string; body?: true }
  | { as: "dot" | "accent" | "accentLine" | "drip"; d: string };

/** A path to draw: what React or plain SVG needs. */
export interface IconPath {
  d: string;
  fill: string;
  stroke: string;
  transform?: string;
}

const n = (v: number) => String(Math.round(v * 100) / 100);

/** A circle as a path. */
export function circle(cx: number, cy: number, r: number): string {
  return `M${n(cx - r)} ${n(cy)}a${n(r)} ${n(r)} 0 1 0 ${n(2 * r)} 0a${n(r)} ${n(r)} 0 1 0 ${n(-2 * r)} 0Z`;
}

/** A rounded rectangle as a path. */
export function rect(x: number, y: number, w: number, h: number, r: number): string {
  return (
    `M${n(x + r)} ${n(y)}h${n(w - 2 * r)}a${n(r)} ${n(r)} 0 0 1 ${n(r)} ${n(r)}v${n(h - 2 * r)}` +
    `a${n(r)} ${n(r)} 0 0 1 ${n(-r)} ${n(r)}h${n(-(w - 2 * r))}a${n(r)} ${n(r)} 0 0 1 ${n(-r)} ${n(-r)}v${n(-(h - 2 * r))}` +
    `a${n(r)} ${n(r)} 0 0 1 ${n(r)} ${n(-r)}Z`
  );
}

/** A drop of slime hanging from (x, y), `h` long with a bulb of radius `r`: a pinched neck, then the drop. */
export function drip(x: number, y: number, h = 5, r = 1.65): string {
  const top = y - 0.6; // starts inside the edge's stroke, so it grows out of it
  const cy = y + h - r;
  const neck = r * 0.72;
  const mid = top + (cy - top) * 0.55;
  return (
    `M${n(x - neck)} ${n(top)}C${n(x - neck * 0.75)} ${n(mid)} ${n(x - r)} ${n(cy - r * 1.1)} ${n(x - r)} ${n(cy)}` +
    `A${n(r)} ${n(r)} 0 0 0 ${n(x + r)} ${n(cy)}C${n(x + r)} ${n(cy - r * 1.1)} ${n(x + neck * 0.75)} ${n(mid)} ${n(x + neck)} ${n(top)}Z`
  );
}

/** A cog: `teeth` teeth between radius `inner` and `outer`, flat-topped. */
export function cog(cx: number, cy: number, outer: number, inner: number, teeth: number): string {
  const step = (Math.PI * 2) / teeth;
  const top = step * 0.17; // half a tooth's top
  const foot = step * 0.29; // half a tooth's foot
  const at = (r: number, a: number) => `${n(cx + r * Math.cos(a))} ${n(cy + r * Math.sin(a))}`;
  let d = "";
  for (let i = 0; i < teeth; i++) {
    const a = i * step - Math.PI / 2;
    d += `${i === 0 ? "M" : "L"}${at(inner, a - foot)}L${at(outer, a - top)}L${at(outer, a + top)}L${at(inner, a + foot)}`;
    d += `A${n(inner)} ${n(inner)} 0 0 1 ${at(inner, a + step - foot)}`;
  }
  return d + "Z";
}

export const line = (d: string): Part => ({ as: "line", d });
export const body = (d: string): Part => ({ as: "line", d, body: true });
export const dot = (d: string): Part => ({ as: "dot", d });
export const accent = (d: string): Part => ({ as: "accent", d });
export const accentLine = (d: string): Part => ({ as: "accentLine", d });
export const dripPart = (x: number, y: number, h?: number, r?: number): Part => ({ as: "drip", d: drip(x, y, h, r) });

/** Every icon, by name. Category icons are named after their category's slug. */
const SHAPES = {
  // ---- categories --------------------------------------------------------
  /** Games: a gamepad, slime dripping from its middle. */
  games: [
    dripPart(12, 14.2, 4.8),
    body("M7.5 6.8h9a5.5 5.5 0 0 1 5.5 5.5v1.2a3.4 3.4 0 0 1-6.1 2.1l-1.4-1.4h-5l-1.4 1.4A3.4 3.4 0 0 1 2 13.5v-1.2a5.5 5.5 0 0 1 5.5-5.5Z"),
    line("M5.4 11.2h4M7.4 9.2v4"),
    dot(circle(15.9, 11.9, 1.15)),
    dot(circle(18.2, 9.7, 1.15)),
  ],
  /** Kids & Family: a home with a slime heart. */
  family: [
    accent("M12 18c-2.4-1.5-3.8-2.9-3.8-4.5 0-1.15.85-2 1.9-2 .82 0 1.48.42 1.9 1.08.42-.66 1.08-1.08 1.9-1.08 1.05 0 1.9.85 1.9 2 0 1.6-1.4 3-3.8 4.5Z"),
    body("M5 10v9a1.5 1.5 0 0 0 1.5 1.5h11A1.5 1.5 0 0 0 19 19v-9L12 4.5Z"),
    line("M2.8 11.6 12 3.8l9.2 7.8"),
  ],
  /** Tools: a wrench, drip off its handle. */
  tools: [
    dripPart(9.4, 17.1, 4.3, 1.4),
    body("M14.3 3.6a5 5 0 0 0-4.6 6.9l-6 6a2.1 2.1 0 0 0 3 3l6-6a5 5 0 0 0 6.9-4.6l-2.8 2.8-2.9-.5-.5-2.9Z"),
  ],
  /** Utilities: a cog with a slime hub. */
  utilities: [accent(circle(12, 12, 2.6)), body(cog(12, 12, 9.4, 6.9, 8)), line(circle(12, 12, 2.6))],
  /** Creative: a paintbrush dipped in slime. */
  creative: [
    accent("M10.6 10.6 13.4 13.4c.3 3.6-2.6 6.6-8.9 7.3.6-6.2 3.5-9.3 6.1-10.1Z"),
    line("M10.6 10.6 13.4 13.4c.3 3.6-2.6 6.6-8.9 7.3.6-6.2 3.5-9.3 6.1-10.1Z"),
    body("M11.4 9.8l8-6.6a1.4 1.4 0 0 1 1.4 1.4l-6.6 8Z"),
  ],
  /** Learning: a mortarboard, its tassel a drip. */
  learning: [
    dripPart(19.6, 15.6, 4.2, 1.45),
    body("M2.5 9.5 12 5l9.5 4.5L12 14Z"),
    line("M6.2 11.6v4.3c0 1.5 2.6 2.7 5.8 2.7s5.8-1.2 5.8-2.7v-4.3M19.6 10.6v5"),
  ],
  /** Play style: an arcade stick with a slime ball. */
  "play-style": [
    accent(circle(12, 6.6, 3.1)),
    line(circle(12, 6.6, 3.1)),
    line("M12 9.7v5.8"),
    body(rect(3.5, 15.5, 17, 5, 1.6)),
    dot(circle(16.8, 13.4, 1.05)),
  ],
  /** Vibe: a lava lamp with slime blobs. */
  vibe: [
    accent("M10.2 9.4c1.4-.6 2.6.2 2.4 1.4-.2 1.1-1.5 1.5-2.4 1-.9-.5-1-1.9 0-2.4Z"),
    accent("M12.6 13.2c1.6-.3 2.6.9 2.2 2.1-.4 1.1-1.2 1.4-2.4 1.6-1.3.2-2.4-.6-2.2-1.7.2-1 1-1.8 2.4-2Z"),
    body("M9.3 6.5h5.4l2.6 7.5-1.8 4h-7l-1.8-4Z"),
    line("M10 6.5l.6-3.5h2.8l.6 3.5M8.5 18l-1.2 3h9.4l-1.2-3"),
  ],
  /** An unknown category: four tiles, one slime. */
  category: [
    accent(rect(13.5, 13.5, 6.5, 6.5, 1.8)),
    body(rect(4, 4, 6.5, 6.5, 1.8)),
    body(rect(13.5, 4, 6.5, 6.5, 1.8)),
    body(rect(4, 13.5, 6.5, 6.5, 1.8)),
    line(rect(13.5, 13.5, 6.5, 6.5, 1.8)),
  ],

  // ---- the store bar, More and the sidebar -------------------------------
  /** Browse and All categories: a compass, its needle half slime. */
  browse: [
    accent("M15.6 8.4 13.3 13.3 10.7 10.7Z"),
    body(circle(12, 12, 9)),
    line("M15.6 8.4 13.3 13.3 8.4 15.6 10.7 10.7Z"),
  ],
  /** Recommended: a heart dripping slime. */
  recommended: [
    dripPart(12, 19.3, 3.6, 1.35),
    body("M12 19.4C5.6 15.6 3 12.6 3 9.4 3 6.9 4.9 5 7.3 5c1.9 0 3.6 1 4.7 2.7C13.1 6 14.8 5 16.7 5 19.1 5 21 6.9 21 9.4c0 3.2-2.6 6.2-9 10Z"),
  ],
  /** Coming soon & beta: an hourglass running slime. */
  upcoming: [
    accent("M9.1 7h5.8c-.5 1.2-1.4 2-2.9 3.2C10.5 9 9.6 8.2 9.1 7Z"),
    accent("M8.4 19.2c.5-1.9 1.6-2.9 3.6-3.9 2 1 3.1 2 3.6 3.9Z"),
    accentLine("M12 12.6v2.2"),
    body("M7 3.5v2c0 2.6 2 4.2 5 6.5 3-2.3 5-3.9 5-6.5v-2M7 20.5v-2c0-2.6 2-4.2 5-6.5 3 2.3 5 3.9 5 6.5v2"),
    line("M5.5 3.5h13M5.5 20.5h13"),
  ],
  /** Everything: a sparkle, with a small slime one. */
  everything: [
    accent("M18.3 2.6c.3 1.6 1 2.3 2.6 2.6-1.6.3-2.3 1-2.6 2.6-.3-1.6-1-2.3-2.6-2.6 1.6-.3 2.3-1 2.6-2.6Z"),
    body("M10 5c.8 4.3 2.7 6.2 7 7-4.3.8-6.2 2.7-7 7-.8-4.3-2.7-6.2-7-7 4.3-.8 6.2-2.7 7-7Z"),
    dot(circle(18, 18, 1.2)),
  ],
  /** Made with people: an open hand, one fingertip slime. */
  made: [
    accent("M10.1 5.5a1.5 1.5 0 0 1 3 0v1.2h-3Z"),
    body(
      "M7.1 14V8a1.5 1.5 0 0 1 3 0v3.5V5.5a1.5 1.5 0 0 1 3 0v6V6.6a1.5 1.5 0 0 1 3 0v5.4V9a1.5 1.5 0 0 1 3 0v5.2a6.8 6.8 0 0 1-6.8 6.8h-.6a5.8 5.8 0 0 1-4.7-2.4l-2.6-3.5a1.6 1.6 0 0 1 2.5-2L7.1 14.6Z",
    ),
  ],
  /** Community builds: a fork, one branch slime. */
  community: [
    accent(circle(17.5, 18.5, 2.3)),
    body(circle(12, 5.5, 2.3)),
    line(circle(6.5, 18.5, 2.3)),
    line(circle(17.5, 18.5, 2.3)),
    line("M12 7.8v2.4c0 2-1.4 2.8-3 3.3-1.6.5-2.5 1.4-2.5 2.7M12 10.2c0 2 1.4 2.8 3 3.3 1.6.5 2.5 1.4 2.5 2.7"),
  ],
  /** Teams: two people, the one behind in slime. */
  teams: [
    accentLine("M15 4.3a3.2 3.2 0 0 1 0 6.3M16.8 14.2a4.6 4.6 0 0 1 4.2 4.6v1.2"),
    body(circle(9, 7.5, 3.4)),
    body("M3 20v-1.2A4.8 4.8 0 0 1 7.8 14h2.4a4.8 4.8 0 0 1 4.8 4.8V20"),
  ],
  /** Publish your app: a slime arrow up out of a tray. */
  publish: [
    accentLine("M12 14.5V3.8M7.6 8.2 12 3.8l4.4 4.4"),
    body("M4 14v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4"),
  ],
  /** How to publish: an open book, a slime bookmark. */
  guide: [
    dripPart(16.5, 18.1, 4, 1.4),
    body("M12 6.6C10.2 5.2 7.4 4.6 3 5v12.6c4.4-.4 7.2.2 9 1.5 1.8-1.3 4.6-1.9 9-1.5V5c-4.4-.4-7.2.2-9 1.6Z"),
    line("M12 6.6v12.5"),
  ],
  /** Hire artists & composers: a palette, one well slime. */
  hire: [
    accent(circle(15.4, 7.6, 1.55)),
    body(
      "M12 3a9 9 0 0 0 0 18c1.2 0 1.9-.8 1.9-1.8 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.2 0-1 .8-1.7 1.7-1.7h2.1A4.3 4.3 0 0 0 21 10.8C21 6.5 17 3 12 3Z",
    ),
    dot(circle(7.4, 11, 1.3)),
    dot(circle(10.2, 7.2, 1.3)),
    line(circle(15.4, 7.6, 1.55)),
  ],
  /** Fix bounties: a money bag with a slime coin sign. */
  bounties: [
    accentLine("M14.2 12.3c-.4-.7-1.2-1.1-2.2-1.1-1.2 0-2.1.6-2.1 1.5 0 2 4.5 1.1 4.5 3.3 0 .9-1 1.6-2.4 1.6-1 0-1.9-.4-2.3-1.1M12 9.9v1.3M12 17.6v1.2"),
    body("M9.2 7.5C5.8 9.6 4 12.6 4 15.6 4 19 6.6 21 12 21s8-2 8-5.4c0-3-1.8-6-5.2-8.1Z"),
    line("M9.2 7.5 7.9 4.3c-.2-.6.2-1.3.9-1.3h6.4c.7 0 1.1.7.9 1.3l-1.3 3.2"),
  ],

  // ---- the footer --------------------------------------------------------
  /** Terms & policies: a page with a slime-folded corner. */
  terms: [
    accent("M14 3v4.2c0 .5.3.8.8.8H19Z"),
    body("M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z"),
    line("M14 3v4.2c0 .5.3.8.8.8H19M8.6 12.5h6.8M8.6 16.2h4.6"),
  ],
  /** Content Policy & reports: a shield, half slime. */
  policy: [
    accent("M12 3.2v17.6c-4.1-1.6-7-5.2-7-9.6V6.1Z"),
    body("M12 3.2 19 6.1v5.1c0 4.4-2.9 8-7 9.6-4.1-1.6-7-5.2-7-9.6V6.1Z"),
  ],
  /** Check a file: a magnifier with a slime tick. */
  verify: [
    accentLine("M7.7 10.7l1.9 1.9 3.6-3.8"),
    body(circle(10.5, 10.5, 6.8)),
    line("M15.4 15.4 20.6 20.6"),
  ],
  /** Source code: angle brackets with a slime slash. */
  source: [accentLine("M13.6 4.5 10.4 19.5"), line("M7.6 7.2 2.8 12l4.8 4.8M16.4 7.2l4.8 4.8-4.8 4.8")],
  /** Code signing policy: a pen nib leaving a drop of slime. */
  signing: [
    dripPart(12, 18.4, 4.2, 1.45),
    body("M8.2 3.5h7.6v3.4l2.7 3.6L12 18.6l-6.5-8.1 2.7-3.6Z"),
    line("M8.2 6.9h7.6M12 18.6v-6.2"),
    dot(circle(12, 11.1, 1.25)),
  ],
  /** Contact: an envelope, a drop of slime underneath. */
  contact: [
    dripPart(16.5, 18.6, 3.8, 1.35),
    body(rect(3, 5.4, 18, 13.2, 2.2)),
    line("M3.8 7.2 12 13.2l8.2-6"),
  ],
} satisfies Record<string, Part[]>;

export type StoreIconName = keyof typeof SHAPES;

/** Every icon's name, in the order above. */
export const STORE_ICON_NAMES = Object.keys(SHAPES) as StoreIconName[];

/** The stroke width each style draws its lines with. */
export const STROKE: Record<StoreIconStyle, number> = { drip: 1.8, goo: 2.4 };

const LINE = "currentColor";
const SLIME = "var(--icon-accent, #a3e635)";
const TONE = "var(--icon-tone, rgba(163, 230, 53, 0.3))";
/** How far the goo style's slime shadow sits from its line. */
const GOO_OFFSET = "translate(1.5 1.5)";

/** The paths that draw `name` in `style`, back to front. */
export function storeIconPaths(name: StoreIconName, style: StoreIconStyle = STORE_ICON_STYLE): IconPath[] {
  return iconPaths(SHAPES[name], style);
}

/** The paths that draw an icon's parts in `style`, back to front: store icons and tag icons alike. */
export function iconPaths(parts: readonly Part[], style: StoreIconStyle = STORE_ICON_STYLE): IconPath[] {
  if (style === "drip") {
    return parts.map((p) => {
      switch (p.as) {
        case "line":
          return { d: p.d, fill: "none", stroke: LINE };
        case "dot":
          return { d: p.d, fill: LINE, stroke: "none" };
        case "accent":
        case "drip":
          return { d: p.d, fill: SLIME, stroke: "none" };
        case "accentLine":
          return { d: p.d, fill: "none", stroke: SLIME };
      }
    });
  }
  // Goo: the slime shadows go first, then everything in the text colour, without the drips.
  const shadows: IconPath[] = parts
    .filter((p) => p.as === "line" && p.body)
    .map((p) => ({ d: p.d, fill: TONE, stroke: "none", transform: GOO_OFFSET }));
  const shapes: IconPath[] = parts.flatMap((p): IconPath[] => {
    switch (p.as) {
      case "line":
      case "accentLine":
        return [{ d: p.d, fill: "none", stroke: LINE }];
      case "dot":
      case "accent":
        return [{ d: p.d, fill: LINE, stroke: "none" }];
      case "drip":
        return [];
    }
  });
  return [...shadows, ...shapes];
}

/** Category slugs with an icon of their own; any other category gets `category`. */
export const CATEGORY_ICON_SLUGS: readonly StoreIconName[] = ["games", "family", "tools", "creative", "utilities", "learning", "play-style", "vibe"];
const CATEGORY_ICONS = new Set<string>(CATEGORY_ICON_SLUGS);

/** The icon for a category, by its slug. Categories come from the API, so an unknown one gets a neutral icon. */
export function categoryIconName(slug: string): StoreIconName {
  return CATEGORY_ICONS.has(slug) ? (slug as StoreIconName) : "category";
}
