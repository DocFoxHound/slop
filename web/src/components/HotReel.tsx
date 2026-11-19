// SPDX-License-Identifier: AGPL-3.0-or-later
// The Hot reel near the top of the front page (DESIGN §5): apps taking off this week (hot.rs).
// Unlike Featured, it moves on by itself: the app shown changes every ROTATE_MS (a thin bar fills
// up until it does), and while an app shows, its picture changes every PICTURE_MS, going through
// its store page's screenshots and video posters in a random order. Only while it's on screen. It
// meets WCAG 2.2.2 (Pause, Stop, Hide) the way the WAI-ARIA carousel pattern does:
// - a Pause button always shows while it can move (Play starts it again);
// - it waits while the pointer is over it or keyboard focus is in it (except on Pause itself),
//   then carries on where it was;
// - it never moves by itself when the person asked for reduced motion;
// - Previous and Next page through it, and each slide says where it is ("2 of 8: …");
// - screen readers aren't told of each change while it moves (aria-live off), only when it's
//   still and the person moves it themselves.

import { useEffect, useMemo, useRef, useState, type FocusEvent, type RefObject } from "react";
import { Link } from "react-router";

import type { GalleryItem, HotApp } from "../api";
import { useMediaQuery } from "../hooks";
import { Art, ReleaseDate, StageBadge } from "./ui";

/** How long each slide shows while it moves by itself. */
export const ROTATE_MS = 6000;
/** How long each of a slide's pictures shows. */
export const PICTURE_MS = 2000;

/** The pictures the reel can show for an app: its screenshots, and its videos' posters. */
export function reelPictures(app: Pick<HotApp, "gallery">): Array<GalleryItem & { url: string }> {
  return (app.gallery ?? []).filter((g): g is GalleryItem & { url: string } => g.url !== null);
}

/** A fresh random order (Fisher–Yates). */
function shuffle<T>(list: T[]): T[] {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/** True while at least half of `el` is on screen and the tab is in front (always, without IntersectionObserver). */
function useOnScreen(el: RefObject<HTMLElement | null>): boolean {
  const [inView, setInView] = useState(true);
  const [shown, setShown] = useState(() => typeof document === "undefined" || document.visibilityState !== "hidden");
  useEffect(() => {
    const node = el.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(([entry]) => setInView(entry.isIntersecting), { threshold: 0.5 });
    io.observe(node);
    return () => io.disconnect();
  }, [el]);
  useEffect(() => {
    const onChange = () => setShown(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);
  return inView && shown;
}

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
  const reel = useRef<HTMLElement>(null);
  const onScreen = useOnScreen(reel);
  const count = apps.length;
  const at = Math.min(index, Math.max(count - 1, 0));
  const app = apps[at] as HotApp | undefined;
  const canMove = !reduced && (count > 1 || apps.some((a) => reelPictures(a).length > 1));
  const moving = canMove && !stopped && !hovered && !focused && onScreen;
  const slidesMove = moving && count > 1;

  // This slide's pictures in a new random order each time it shows.
  const pictures = useMemo(() => (app ? shuffle(reelPictures(app)) : []), [app]);
  const [frame, setFrame] = useState(0);
  useEffect(() => setFrame(0), [pictures]);
  const picturesMove = moving && pictures.length > 1;
  useEffect(() => {
    if (!picturesMove) return;
    const timer = window.setInterval(() => setFrame((f) => f + 1), PICTURE_MS);
    return () => window.clearInterval(timer);
  }, [picturesMove, pictures]);

  // The slide's turn: a pause (pointer over it, focus in it, off screen) holds what's left of it,
  // and a new slide (by itself or the person's choice) gets a full one.
  const spent = useRef({ at: 0, ms: 0 });
  useEffect(() => {
    if (!slidesMove) return;
    if (spent.current.at !== at) spent.current = { at, ms: 0 };
    const start = Date.now();
    const timer = window.setTimeout(() => setIndex((i) => (i + 1) % count), Math.max(0, ROTATE_MS - spent.current.ms));
    return () => {
      window.clearTimeout(timer);
      if (spent.current.at === at) spent.current.ms += Date.now() - start;
    };
  }, [slidesMove, at, count]);

  if (!app) return null;
  const picture = pictures.length ? pictures[frame % pictures.length] : undefined;
  const go = (i: number) => setIndex((i + count) % count);
  // Focus on Pause/Play doesn't hold it still: that button is how it's started again.
  const onFocus = (e: FocusEvent) => setFocused(e.target !== pause.current);
  const onBlur = (e: FocusEvent<HTMLElement>) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
  };
  const line = hotLine(app);

  return (
    <section
      ref={reel}
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
          <Link to={`/app/${app.slug}`} className="featured-art hot-art" tabIndex={-1} aria-hidden="true">
            <Art key={picture?.url} app={picture ? { ...app, thumbnail_url: picture.url } : app} withName={false} />
            {picture?.kind === "video" && <span className="hot-video">▶ Video</span>}
            {count > 1 && canMove && (
              <span
                key={at}
                className="hot-timer"
                style={{ animationDuration: `${ROTATE_MS}ms`, animationPlayState: slidesMove ? "running" : "paused" }}
              />
            )}
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
