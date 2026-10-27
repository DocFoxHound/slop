// SPDX-License-Identifier: AGPL-3.0-or-later
// The Featured carousel at the top of the front page (DESIGN §5), picked by SLAPorSLOP staff.
// It never moves by itself (WCAG 2.2.2): players page through it with the buttons or arrow keys.

import { useState, type KeyboardEvent } from "react";
import { Link } from "react-router";

import type { FeaturedApp } from "../api";
import { Art, StageBadge } from "./ui";

export function FeaturedCarousel({ slides }: { slides: FeaturedApp[] }) {
  const [index, setIndex] = useState(0);
  if (slides.length === 0) return null;
  const at = Math.min(index, slides.length - 1);
  const app = slides[at];
  const go = (i: number) => setIndex((i + slides.length) % slides.length);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowRight") go(at + 1);
    if (e.key === "ArrowLeft") go(at - 1);
  };
  return (
    <section className="featured panel" aria-roledescription="carousel" aria-label="Featured" onKeyDown={onKey}>
      <div className="featured-slide" role="group" aria-roledescription="slide" aria-label={`${at + 1} of ${slides.length}: ${app.title}`}>
        <Link to={`/app/${app.slug}`} className="featured-art" tabIndex={-1} aria-hidden="true">
          <Art app={app} withName={false} />
        </Link>
        <div className="featured-text">
          <span className="eyebrow">Featured</span>
          <h2>
            <Link to={`/app/${app.slug}`}>{app.title}</Link> <StageBadge app={app} />
          </h2>
          {app.blurb && <p className="featured-blurb">{app.blurb}</p>}
          <p className="muted">{app.summary}</p>
          <p className="small">
            {app.not_slop_percent !== null && (
              <span className={app.not_slop_percent >= 70 ? "good" : app.not_slop_percent < 40 ? "bad" : ""}>
                {app.not_slop_percent}% Slaps
              </span>
            )}
            {app.players_now !== null && app.players_now > 0 && <span className="muted"> · {app.players_now.toLocaleString()} playing now</span>}
          </p>
        </div>
      </div>
      {slides.length > 1 && (
        <div className="featured-nav">
          <button className="btn small ghost" onClick={() => go(at - 1)} aria-label="Previous featured app">
            ‹
          </button>
          {slides.map((s, i) => (
            <button
              key={s.slug}
              className={`dot ${i === at ? "on" : ""}`}
              aria-label={`Show ${s.title}`}
              aria-current={i === at ? "true" : undefined}
              onClick={() => go(i)}
            />
          ))}
          <button className="btn small ghost" onClick={() => go(at + 1)} aria-label="Next featured app">
            ›
          </button>
        </div>
      )}
    </section>
  );
}
