// SPDX-License-Identifier: AGPL-3.0-or-later
// The Hot reel near the top of the front page (DESIGN §5): apps taking off this week (hot.rs).
// Unlike Featured, it moves on by itself, every ROTATE_MS, so it meets WCAG 2.2.2 (Pause, Stop,
// Hide) the way the WAI-ARIA carousel pattern does:
// - a Pause button always shows while it can move (Play starts it again);
// - it waits while the pointer is over it or keyboard focus is in it (except on Pause itself);
// - it never moves by itself when the person asked for reduced motion;
// - Previous and Next page through it, and each slide says where it is ("2 of 8: …");
// - screen readers aren't told of each change while it moves (aria-live off), only when it's
//   still and the person moves it themselves.

import { useEffect, useRef, useState, type FocusEvent } from "react";
import { Link } from "react-router";

import type { HotApp } from "../api";
import { useMediaQuery } from "../hooks";
import { Art, ReleaseDate, StageBadge } from "./ui";

/** How long each slide shows while it moves by itself. */
export const ROTATE_MS = 6000;

/** This week's numbers in plain words: "This week: 12 library adds · 5 Slaps · up to 30 playing at once". */
export function hotLine(app: Pick<HotApp, "adds_week" | "slaps_week" | "players_week">): string {
  const parts: string[] = [];
  if (app.adds_week > 0) parts.push(`${app.adds_week.toLocaleString()} library add${app.adds_week === 1 ? "" : "s"}`);
  if (app.slaps_week > 0) parts.push(`${app.slaps_week.toLocaleString()} Slaps`);
  if (app.players_week > 0) parts.push(`up to ${app.players_week.toLocaleString()} playing at once`);
  return parts.length ? `This week: ${parts.join(" · ")}` : "";
}

export function HotReel({ apps }: { apps: HotApp[] }) {
  const reduced = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [index, setIndex] = useState(0);
  // The person pressed Pause.
  const [stopped, setStopped] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const pause = useRef<HTMLButtonElement>(null);
  const count = apps.length;
  const canMove = count > 1 && !reduced;
  const moving = canMove && !stopped && !hovered && !focused;

  // A fresh full turn after every change, so a slide the person just moved to isn't cut short.
  useEffect(() => {
    if (!moving) return;
    const timer = window.setTimeout(() => setIndex((i) => (i + 1) % count), ROTATE_MS);
    return () => window.clearTimeout(timer);
  }, [moving, index, count]);

  if (count === 0) return null;
  const at = Math.min(index, count - 1);
  const app = apps[at];
  const go = (i: number) => setIndex((i + count) % count);
  // Focus on Pause/Play doesn't hold it still: that button is how it's started again.
  const onFocus = (e: FocusEvent) => setFocused(e.target !== pause.current);
  const onBlur = (e: FocusEvent<HTMLElement>) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
  };
  const line = hotLine(app);

  return (
    <section
      className="featured hot panel"
      aria-roledescription="carousel"
      aria-labelledby="hot-title"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={onFocus}
      onBlur={onBlur}
    >
      <div className="hot-head">
        <h2 id="hot-title">Hot this week</h2>
        <span className="muted small">Taking off: library adds, Slaps and players, more than the week before</span>
        {canMove && (
          <button ref={pause} type="button" className="btn small ghost hot-pause" onClick={() => setStopped((s) => !s)}>
            <span aria-hidden="true">{stopped ? "▶" : "❚❚"}</span> {stopped ? "Play" : "Pause"}
          </button>
        )}
      </div>
      <div aria-live={moving ? "off" : "polite"}>
        <div className="featured-slide" role="group" aria-roledescription="slide" aria-label={`${at + 1} of ${count}: ${app.title}`}>
          <Link to={`/app/${app.slug}`} className="featured-art" tabIndex={-1} aria-hidden="true">
            <Art app={app} withName={false} />
          </Link>
          <div className="featured-text">
            <span className="eyebrow">Hot · {at + 1} of {count}</span>
            <h3>
              <Link to={`/app/${app.slug}`}>{app.title}</Link> <StageBadge app={app} /> <ReleaseDate app={app} />
            </h3>
            <p className="muted">{app.summary}</p>
            {line && <p className="small">{line}</p>}
            {app.not_slop_percent !== null && (
              <p className="small">
                <span className={app.not_slop_percent >= 70 ? "good" : app.not_slop_percent < 40 ? "bad" : ""}>
                  {app.not_slop_percent}% Slaps
                </span>
              </p>
            )}
          </div>
        </div>
      </div>
      {count > 1 && (
        <div className="featured-nav">
          <button type="button" className="btn small ghost" onClick={() => go(at - 1)} aria-label="Previous hot app">
            ‹
          </button>
          {apps.map((s, i) => (
            <button
              key={s.slug}
              type="button"
              className={`dot ${i === at ? "on" : ""}`}
              aria-label={`Show ${s.title}`}
              aria-current={i === at ? "true" : undefined}
              onClick={() => go(i)}
            />
          ))}
          <button type="button" className="btn small ghost" onClick={() => go(at + 1)} aria-label="Next hot app">
            ›
          </button>
        </div>
      )}
    </section>
  );
}
