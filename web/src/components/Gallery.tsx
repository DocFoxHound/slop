// SPDX-License-Identifier: AGPL-3.0-or-later
// The screenshots-and-videos carousel on an app's page: one big view and a strip of thumbnails,
// like Steam's. Pictures come from the maker's repository (crates/slop-api/src/media.rs). Pressing
// the big view opens it full size (MediaViewer.tsx), where the arrows step through everything.
// Videos stay on YouTube: the page shows the maker's poster (or a plain card), and only loads
// YouTube's player (from youtube-nocookie.com) once someone presses play, which plays it in the
// full-size viewer. The desktop app opens the video in the browser instead.

import { useRef, useState, type KeyboardEvent } from "react";

import type { GalleryItem } from "../api";
import { isDesktop, openExternal } from "../platform";
import { MediaViewer, type ViewerItem } from "./MediaViewer";

export function Gallery({ items, title }: { items: GalleryItem[]; title: string }) {
  const [index, setIndex] = useState(0);
  // The full-size viewer, open at an item; `play`: opened by pressing play on a video.
  const [viewing, setViewing] = useState<{ start: number; play: boolean }>();
  const stage = useRef<HTMLButtonElement>(null);
  if (items.length === 0) return null;
  const at = Math.min(index, items.length - 1);
  const current = items[at];
  const go = (i: number) => setIndex((i + items.length) % items.length);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowLeft") go(at - 1);
    else if (e.key === "ArrowRight") go(at + 1);
    else return;
    e.preventDefault();
  };
  const label = (item: GalleryItem, i: number) =>
    item.caption || `${item.kind === "video" ? "Video" : "Screenshot"} ${i + 1} of ${items.length}`;
  const play = (item: GalleryItem) => {
    if (!item.youtube_id) return;
    if (isDesktop) void openExternal(`https://www.youtube.com/watch?v=${item.youtube_id}`);
    else setViewing({ start: at, play: true });
  };
  const shown: ViewerItem[] = items.map((item, i) => ({
    id: item.id,
    kind: item.kind,
    url: item.url,
    youtubeId: item.youtube_id,
    label: label(item, i),
    caption: item.caption,
    width: item.width,
    height: item.height,
  }));
  // Where the video plays; also part of its Play button's name, which holds the words it shows.
  const note = isDesktop ? "Opens on YouTube" : "Plays from YouTube";

  return (
    <>
      <section className="gallery" aria-roledescription="carousel" aria-label={`${title}: screenshots and videos`} onKeyDown={onKey}>
        <div className="gallery-stage">
          {current.kind === "video" ? (
            <button
              ref={stage}
              type="button"
              className="gallery-play"
              onClick={() => play(current)}
              aria-label={`Play video: ${label(current, at)}. ${note}`}
              aria-haspopup={isDesktop ? undefined : "dialog"}
            >
              {current.url ? <img src={current.url} alt="" /> : <span className="play-card">{label(current, at)}</span>}
              <span className="play-mark" aria-hidden="true">
                ▶
              </span>
              <span className="play-note">{note}</span>
            </button>
          ) : (
            <button
              ref={stage}
              type="button"
              className="gallery-zoom"
              onClick={() => setViewing({ start: at, play: false })}
              aria-label={`View full size: ${label(current, at)}`}
              aria-haspopup="dialog"
            >
              <img src={current.url ?? ""} alt={label(current, at)} />
            </button>
          )}
          {items.length > 1 && (
            <>
              <button type="button" className="gallery-arrow prev" onClick={() => go(at - 1)} aria-label="Previous">
                ‹
              </button>
              <button type="button" className="gallery-arrow next" onClick={() => go(at + 1)} aria-label="Next">
                ›
              </button>
            </>
          )}
        </div>
        {current.caption && <p className="gallery-caption muted small">{current.caption}</p>}
        {items.length > 1 && (
          <div className="gallery-strip" role="tablist" aria-label="Choose a screenshot or video">
            {items.map((item, i) => (
              <button
                type="button"
                key={item.id}
                role="tab"
                aria-selected={i === at}
                aria-label={label(item, i)}
                className={i === at ? "on" : ""}
                onClick={() => go(i)}
              >
                {item.url ? <img src={item.url} alt="" loading="lazy" /> : <span className="play-card" />}
                {item.kind === "video" && (
                  <span className="play-mark small" aria-hidden="true">
                    ▶
                  </span>
                )}
              </button>
            ))}
          </div>
        )}
      </section>
      {viewing && (
        <MediaViewer
          items={shown}
          start={viewing.start}
          play={viewing.play}
          label={`${title}: screenshots and videos`}
          returnFocus={() => stage.current}
          onClose={(last) => {
            // The stage shows the item last viewed, and focus goes back to it.
            setIndex(last);
            setViewing(undefined);
          }}
        />
      )}
    </>
  );
}
