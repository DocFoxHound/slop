// The screenshots-and-videos carousel on an app's page: one big view and a strip of thumbnails,
// like Steam's. Pictures come from the maker's repository (crates/slop-api/src/media.rs). Videos
// stay on YouTube: the page shows the maker's poster (or a plain card), and only loads YouTube's
// player (from youtube-nocookie.com) once someone presses play. The desktop app opens the video
// in the browser instead.

import { useState, type KeyboardEvent } from "react";

import type { GalleryItem } from "../api";
import { isDesktop, openExternal } from "../platform";

export function Gallery({ items, title }: { items: GalleryItem[]; title: string }) {
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState<string>();
  if (items.length === 0) return null;
  const current = items[Math.min(index, items.length - 1)];
  const go = (i: number) => {
    setIndex((i + items.length) % items.length);
    setPlaying(undefined);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowLeft") go(index - 1);
    else if (e.key === "ArrowRight") go(index + 1);
    else return;
    e.preventDefault();
  };
  const label = (item: GalleryItem, i: number) =>
    item.caption || `${item.kind === "video" ? "Video" : "Screenshot"} ${i + 1} of ${items.length}`;
  const play = (item: GalleryItem) => {
    if (!item.youtube_id) return;
    if (isDesktop) void openExternal(`https://www.youtube.com/watch?v=${item.youtube_id}`);
    else setPlaying(item.id);
  };

  return (
    <section className="gallery" aria-roledescription="carousel" aria-label={`${title}: screenshots and videos`} onKeyDown={onKey}>
      <div className="gallery-stage">
        {current.kind === "video" && playing === current.id ? (
          <iframe
            src={`https://www.youtube-nocookie.com/embed/${current.youtube_id}?autoplay=1&rel=0`}
            title={label(current, index)}
            allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
            allowFullScreen
            referrerPolicy="strict-origin-when-cross-origin"
          />
        ) : current.kind === "video" ? (
          <button type="button" className="gallery-play" onClick={() => play(current)} aria-label={`Play video: ${label(current, index)}`}>
            {current.url ? <img src={current.url} alt="" /> : <span className="play-card">{current.caption ?? title}</span>}
            <span className="play-mark" aria-hidden="true">
              ▶
            </span>
            <span className="play-note">{isDesktop ? "Opens on YouTube" : "Plays from YouTube"}</span>
          </button>
        ) : (
          <img src={current.url ?? ""} alt={label(current, index)} />
        )}
        {items.length > 1 && (
          <>
            <button type="button" className="gallery-arrow prev" onClick={() => go(index - 1)} aria-label="Previous">
              ‹
            </button>
            <button type="button" className="gallery-arrow next" onClick={() => go(index + 1)} aria-label="Next">
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
              aria-selected={i === index}
              aria-label={label(item, i)}
              className={i === index ? "on" : ""}
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
  );
}
