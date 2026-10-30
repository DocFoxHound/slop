// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's own icons for the store's categories, links and tags (the shapes are in
// ../storeIcons.ts and ../tagIcons.ts; the style the store uses is STORE_ICON_STYLE there).
// Decorative: the words beside them say what they are, so screen readers skip them.

import { categoryIconName, STORE_ICON_STYLE, STROKE, storeIconPaths, type IconPath, type StoreIconName, type StoreIconStyle } from "../storeIcons";
import { skillIconPaths, tagIconName, tagIconPaths } from "../tagIcons";

/** The SVG every store and tag icon is drawn in. */
function IconSvg({
  paths,
  size,
  className,
  style,
  data,
}: {
  paths: IconPath[];
  size: number;
  className?: string;
  style: StoreIconStyle;
  data: { "data-icon"?: string; "data-tag"?: string; "data-skill"?: string };
}) {
  return (
    <svg
      className={className ? `store-icon ${className}` : "store-icon"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      strokeWidth={STROKE[style]}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...data}
    >
      {paths.map((p, i) => (
        <path key={i} d={p.d} style={{ fill: p.fill, stroke: p.stroke }} transform={p.transform} />
      ))}
    </svg>
  );
}

export function StoreIcon({
  name,
  size = 16,
  className,
  style = STORE_ICON_STYLE,
}: {
  name: StoreIconName;
  size?: number;
  className?: string;
  style?: StoreIconStyle;
}) {
  return <IconSvg paths={storeIconPaths(name, style)} size={size} className={className} style={style} data={{ "data-icon": name }} />;
}

/** A category's icon, by its slug; one the store doesn't know gets a neutral icon. */
export function CategoryIcon({ slug, size, className }: { slug: string; size?: number; className?: string }) {
  return <StoreIcon name={categoryIconName(slug)} size={size} className={className} />;
}

/**
 * A tag's icon (a sub-genre: roguelike, cozy…), by its slug (the shapes are in ../tagIcons.ts).
 * A tag without an icon of its own (a new one staff added) gets its category's, or the neutral one.
 */
/** A People skill's icon (tagIcons.ts SKILL_SHAPES); "everyone" for the whole directory. */
export function SkillIcon({ slug, size = 20, className }: { slug: string; size?: number; className?: string }) {
  return <IconSvg paths={skillIconPaths(slug)} size={size} className={className} style={STORE_ICON_STYLE} data={{ "data-skill": slug }} />;
}

export function TagIcon({ slug, category, size = 16, className }: { slug: string; category?: string; size?: number; className?: string }) {
  const name = tagIconName(slug);
  if (!name) return <CategoryIcon slug={category ?? "category"} size={size} className={className} />;
  return <IconSvg paths={tagIconPaths(name)} size={size} className={className} style={STORE_ICON_STYLE} data={{ "data-tag": name }} />;
}
