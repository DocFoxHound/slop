// SPDX-License-Identifier: AGPL-3.0-or-later
// An app's small square picture: its approved icon (`icon` in the maker's .slop/media.json,
// crates/slop-api/src/media.rs); without one, a square from the middle of its thumbnail; without
// that, its first letter on a colour made from its name. A picture that doesn't load falls back to
// the next. Decorative: the app's name is always beside it.
//
// Stable API (the Library page imports it):
//   <AppIcon title={app.title} iconUrl={app.icon_url} thumbnailUrl={app.thumbnail_url} size={32} />
//   <AppIcon {...appIconProps(app)} size={32} />

import { useEffect, useState, type CSSProperties } from "react";

import { artFor } from "../format";

export interface AppIconProps {
  title: string;
  /** The approved icon (`icon_url` on app summaries), if any. */
  iconUrl?: string | null;
  /** The store tile (`thumbnail_url`), cropped to a square when there's no icon. */
  thumbnailUrl?: string | null;
  /** Width and height in CSS pixels. Default 32. */
  size?: number;
  /** The app's store address, for the letter's colour (as the store's tiles). Default: the title. */
  slug?: string;
  className?: string;
}

/** Which picture an icon shows: the icon, a crop of the thumbnail, or the letter. */
export type AppIconKind = "icon" | "crop" | "letter";

/** The pictures to try, best first. */
export function iconSources(iconUrl?: string | null, thumbnailUrl?: string | null): Array<{ kind: AppIconKind; url: string }> {
  const out: Array<{ kind: AppIconKind; url: string }> = [];
  if (iconUrl) out.push({ kind: "icon", url: iconUrl });
  if (thumbnailUrl) out.push({ kind: "crop", url: thumbnailUrl });
  return out;
}

/** The props for an app summary (anything with `title` and maybe `slug`, `icon_url`, `thumbnail_url`). */
export function appIconProps(app: {
  title: string;
  slug?: string;
  icon_url?: string | null;
  thumbnail_url?: string | null;
}): Pick<AppIconProps, "title" | "slug" | "iconUrl" | "thumbnailUrl"> {
  return { title: app.title, slug: app.slug, iconUrl: app.icon_url, thumbnailUrl: app.thumbnail_url };
}

export function AppIcon({ title, iconUrl, thumbnailUrl, size = 32, slug, className = "" }: AppIconProps) {
  const sources = iconSources(iconUrl, thumbnailUrl);
  // How many pictures failed to load; a new picture starts again from the best.
  const [failed, setFailed] = useState(0);
  useEffect(() => setFailed(0), [iconUrl, thumbnailUrl]);
  const source = sources[failed];
  const kind: AppIconKind = source?.kind ?? "letter";
  const style: CSSProperties = {
    width: size,
    height: size,
    minWidth: size,
    borderRadius: Math.max(4, Math.round(size * 0.2)),
    fontSize: Math.max(10, Math.round(size * 0.5)),
  };
  if (!source) {
    const { from, to } = artFor(slug ?? title);
    style.background = `linear-gradient(135deg, ${from}, ${to})`;
  }
  return (
    <span className={`app-icon is-${kind} ${className}`.trim()} style={style} aria-hidden="true" data-kind={kind}>
      {source ? (
        <img
          key={source.url}
          src={source.url}
          alt=""
          width={size}
          height={size}
          loading="lazy"
          decoding="async"
          onError={() => setFailed((n) => n + 1)}
        />
      ) : (
        letter(title)
      )}
    </span>
  );
}

function letter(title: string): string {
  const first = title.trim().charAt(0);
  return first ? first.toUpperCase() : "?";
}
