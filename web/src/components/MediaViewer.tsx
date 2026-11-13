// SPDX-License-Identifier: AGPL-3.0-or-later
// The full-size viewer, like Steam's: an app page's screenshots and videos (Gallery.tsx) and a
// creative's portfolio pictures (People.tsx). One item as large as the window allows, never above
// its own size (small pixel art is the exception: it's enlarged with sharp pixels, as on People),
// its caption, "3 of 8", Previous and Next (wrapping around), and Close.
//
// A modal <dialog>, like SLAPorSLOP's other dialogs: the page behind is inert and doesn't scroll,
// focus stays inside (when the focused Play button or picture goes away, focus moves to the
// dialog, never the page), and goes back to what opened it. Arrow keys and a swipe step (not while
// the page is pinch-zoomed: then a swipe pans); Escape and a click outside the picture close. Keys
// pressed inside YouTube's player go to YouTube: Close is the way out from there. Videos stay on YouTube: nothing loads from youtube-nocookie.com
// until someone presses play (here, or on the video that opened the viewer). The desktop app opens
// them in the browser instead, never in a frame.

import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent, type TouchEvent } from "react";

import { isDesktop, openExternal } from "../platform";

export interface ViewerItem {
  id: string;
  kind: "image" | "video";
  /** The picture, or the video's poster (none: a plain card). */
  url: string | null;
  youtubeId?: string | null;
  /** The picture's text alternative and the video's name: its caption, else e.g. "Screenshot 2 of 5". */
  label: string;
  /** Shown under it. */
  title?: string | null;
  caption?: string | null;
  /** Its natural width: under 200 pixels, it's pixel art and keeps its sharp pixels. */
  width?: number | null;
  /** Its natural height: with the width, how big pixel art is drawn. */
  height?: number | null;
}

/** Pictures this narrow are pixel art (People's rule): drawn with sharp pixels, never blurred. */
export const isPixelArt = (width: number | null | undefined) => (width ?? 999) < 200;

/** How far a finger moves sideways before it counts as a swipe, in CSS pixels. */
const SWIPE = 50;

export function MediaViewer({
  items,
  start,
  label,
  play = false,
  returnFocus,
  onClose,
}: {
  items: ViewerItem[];
  /** The item it opens at. */
  start: number;
  /** The dialog's name: "Dungeon: screenshots and videos", "Vera Okafor: portfolio". */
  label: string;
  /** Opened by pressing play on a video: it plays at once (on the website; the desktop app never frames YouTube). */
  play?: boolean;
  /** Where focus goes on close; by default the control that had it when the viewer opened. */
  returnFocus?: () => HTMLElement | null | undefined;
  /** Closed: `index` is the item last viewed. */
  onClose: (index: number) => void;
}) {
  const n = items.length;
  const [index, setIndex] = useState(() => Math.min(Math.max(start, 0), Math.max(n - 1, 0)));
  const [playing, setPlaying] = useState<string | undefined>(() =>
    play && !isDesktop && items[start]?.kind === "video" ? items[start].id : undefined,
  );
  // Said politely when stepping (empty at first: opening says the dialog's name instead).
  const [announce, setAnnounce] = useState("");
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const closed = useRef(false);
  const playButton = useRef<HTMLButtonElement>(null);
  const touch = useRef<{ x: number; y: number }>(undefined);
  // Where the last press began: a click only closes when it began and ended on the dark area.
  const pressedOn = useRef<EventTarget | null>(null);
  const focusBack = useRef(returnFocus);
  focusBack.current = returnFocus;
  const keys = useRef<(e: globalThis.KeyboardEvent) => void>(undefined);
  const ids = useId();
  const empty = n === 0;

  useEffect(() => {
    if (empty) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    closeButton.current?.focus();
    // The page behind keeps its place and doesn't scroll (main.view is the page's scroller). Where
    // it shows a scrollbar, its gutter stays, so the page behind doesn't reflow.
    const html = document.documentElement;
    const scrollers = [...document.querySelectorAll<HTMLElement>(".view")].filter((v) => v.offsetWidth > v.clientWidth);
    scrollers.forEach((v) => v.classList.add("viewer-gutter"));
    html.classList.add("viewer-open");
    // Keys are heard on the whole document, so they still step when focus is on the dialog itself.
    const onKey = (e: globalThis.KeyboardEvent) => keys.current?.(e);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      html.classList.remove("viewer-open");
      scrollers.forEach((v) => v.classList.remove("viewer-gutter"));
      const to = focusBack.current?.() ?? opener;
      if (to?.isConnected) to.focus();
    };
  }, [empty]);

  const i = Math.min(index, Math.max(n - 1, 0));
  const item = items[i] as ViewerItem | undefined;
  const isPlaying = Boolean(item && playing === item.id);

  // The focused Play button (or a picture clicked last) went away when stepping or playing: focus
  // stays in the viewer, on the next video's Play button or else Close (which shows where focus
  // is), never the page behind.
  const first = useRef(true);
  useLayoutEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const d = dialog.current;
    if (!d || d.contains(document.activeElement)) return;
    (playButton.current ?? closeButton.current ?? d).focus();
  }, [i, isPlaying]);

  if (empty || !item) return null;

  const close = () => {
    if (closed.current) return;
    closed.current = true;
    onClose(i);
  };
  const go = (to: number) => {
    if (n < 2) return;
    const next = (to + n) % n;
    const it = items[next];
    setIndex(next);
    // Stepping away from a video removes its player.
    setPlaying(undefined);
    setAnnounce([`${next + 1} of ${n}`, it.title || it.caption || it.label].filter(Boolean).join(". "));
  };
  const playVideo = (it: ViewerItem) => {
    if (!it.youtubeId) return;
    if (isDesktop) void openExternal(`https://www.youtube.com/watch?v=${it.youtubeId}`);
    else setPlaying(it.id);
  };

  keys.current = (e: globalThis.KeyboardEvent) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    if (e.key === "ArrowLeft") go(i - 1);
    else if (e.key === "ArrowRight") go(i + 1);
    else if (e.key === "Escape") close();
    else return;
    e.preventDefault();
    e.stopPropagation();
  };
  // A click on the dark area around the picture closes; on the picture or a control, it doesn't,
  // nor does one that began elsewhere (selecting the caption's words) or that selected text.
  const isBackdrop = (target: EventTarget | null, dialogEl: EventTarget) =>
    target === dialogEl || (target instanceof HTMLElement && target.dataset.backdrop !== undefined);
  const onPointerDown = (e: PointerEvent) => {
    pressedOn.current = e.target;
  };
  const onClick = (e: MouseEvent) => {
    const began = pressedOn.current;
    pressedOn.current = null;
    if (!isBackdrop(e.target, e.currentTarget)) return;
    if (began !== null && !isBackdrop(began, e.currentTarget)) return;
    if (window.getSelection?.()?.toString()) return;
    close();
  };
  const onTouchStart = (e: TouchEvent) => {
    const t = e.touches[0];
    touch.current = e.touches.length === 1 && t ? { x: t.clientX, y: t.clientY } : undefined;
  };
  const onTouchEnd = (e: TouchEvent) => {
    const from = touch.current;
    const t = e.changedTouches[0];
    touch.current = undefined;
    if (!from || !t) return;
    // Pinch-zoomed in, a sideways drag looks across the picture: it isn't a step.
    if ((window.visualViewport?.scale ?? 1) > 1.01) return;
    const dx = t.clientX - from.x;
    const dy = t.clientY - from.y;
    if (Math.abs(dx) >= SWIPE && Math.abs(dx) > Math.abs(dy) * 1.5) go(dx < 0 ? i + 1 : i - 1);
  };

  const countId = `${ids}-count`;
  const captionId = `${ids}-caption`;
  const hasCaption = Boolean(item.title || item.caption);
  // A caption that only repeats the picture's text alternative isn't read twice.
  const captionRepeatsAlt = item.kind === "image" && !item.title && item.caption === item.label;

  let media;
  if (item.kind === "image") {
    const pixelArt = isPixelArt(item.width);
    media = <img key={item.id} className={`viewer-media${pixelArt ? " pixelated" : ""}`} style={pixelArt ? pixelArtSize(item) : undefined} src={item.url ?? ""} alt={item.label} />;
  } else if (isPlaying && !isDesktop && item.youtubeId) {
    media = (
      <iframe
        key={item.id}
        className="viewer-video"
        src={`https://www.youtube-nocookie.com/embed/${item.youtubeId}?autoplay=1&rel=0`}
        title={item.label}
        allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
        allowFullScreen
        referrerPolicy="strict-origin-when-cross-origin"
      />
    );
  } else {
    // Its name holds the words it shows: the card's and where it plays.
    const note = isDesktop ? "Opens on YouTube" : "Plays from YouTube";
    media = (
      <button key={item.id} ref={playButton} type="button" className="viewer-video viewer-play" onClick={() => playVideo(item)} aria-label={`Play video: ${item.label}. ${note}`}>
        {item.url ? (
          <img src={item.url} alt="" />
        ) : (
          <span className="play-card">
            <span>{item.label}</span>
          </span>
        )}
        <span className="play-mark" aria-hidden="true">
          ▶
        </span>
        <span className="play-note">{note}</span>
      </button>
    );
  }

  return (
    <dialog
      ref={dialog}
      className="viewer"
      tabIndex={-1}
      aria-label={label}
      aria-describedby={hasCaption ? `${countId} ${captionId}` : countId}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClose={close}
      onPointerDown={onPointerDown}
      onClick={onClick}
      onTouchStart={onTouchStart}
      onTouchEnd={onTouchEnd}
      onTouchCancel={() => (touch.current = undefined)}
    >
      <div className="viewer-bar" data-backdrop="">
        <p className="viewer-count" id={countId}>
          {i + 1} of {n}
        </p>
        <button ref={closeButton} type="button" className="viewer-close" onClick={close}>
          <span aria-hidden="true">✕</span> Close
        </button>
      </div>
      <div className="viewer-stage" data-backdrop="">
        <div className="viewer-frame" data-backdrop="">
          {media}
        </div>
        {n > 1 && (
          <>
            <button type="button" className="viewer-arrow prev" onClick={() => go(i - 1)} aria-label="Previous">
              ‹
            </button>
            <button type="button" className="viewer-arrow next" onClick={() => go(i + 1)} aria-label="Next">
              ›
            </button>
          </>
        )}
      </div>
      <div className="viewer-foot" data-backdrop="">
        {hasCaption && (
          <p className="viewer-caption" id={captionId} aria-hidden={captionRepeatsAlt || undefined}>
            {item.title && <b>{item.title}</b>}
            {item.title && item.caption && " "}
            {item.caption && <span>{item.caption}</span>}
          </p>
        )}
      </div>
      <p className="sr-only" aria-live="polite" aria-atomic="true">
        {announce}
      </p>
    </dialog>
  );
}

/**
 * Pixel art is enlarged with sharp pixels to fit the window (a 32-pixel sprite would otherwise be
 * smaller than its tile), its box exactly the picture, so the dark area around it still closes.
 * Without its height it's drawn at its own size.
 */
function pixelArtSize(item: ViewerItem): CSSProperties | undefined {
  if (!item.width || !item.height) return undefined;
  const ratio = item.width / item.height;
  return { width: `min(100cqw, calc(100cqh * ${ratio}))`, height: "auto", aspectRatio: `${item.width} / ${item.height}` };
}
