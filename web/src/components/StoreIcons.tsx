// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's own icons for the store's categories and links (the shapes are in ../storeIcons.ts;
// the style the store uses is STORE_ICON_STYLE there). Decorative: the words beside them say what
// they are, so screen readers skip them.

import { categoryIconName, STORE_ICON_STYLE, STROKE, storeIconPaths, type StoreIconName, type StoreIconStyle } from "../storeIcons";

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
      data-icon={name}
    >
      {storeIconPaths(name, style).map((p, i) => (
        <path key={i} d={p.d} style={{ fill: p.fill, stroke: p.stroke }} transform={p.transform} />
      ))}
    </svg>
  );
}

/** A category's icon, by its slug; one the store doesn't know gets a neutral icon. */
export function CategoryIcon({ slug, size, className }: { slug: string; size?: number; className?: string }) {
  return <StoreIcon name={categoryIconName(slug)} size={size} className={className} />;
}
