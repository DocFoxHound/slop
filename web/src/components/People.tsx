// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared pieces for People (DESIGN §15): accent colours, the AI-use badge, ratings, portfolio
// works and creative cards.

import { useState, type CSSProperties } from "react";
import { Link } from "react-router";

import type { Accent, AiUse, CreativeCard, CreativePage, CreativeRating, Work } from "../api";
import { money } from "../format";
import { isDesktop, openExternal } from "../platform";
import { Avatar } from "./Avatar";

/** Accent colours a creative can pick; each keeps dark text readable on it (A11Y-1). */
export const ACCENTS: Record<Accent, { name: string; color: string; ink: string }> = {
  slime: { name: "Slime", color: "#a3e635", ink: "#1a2e05" },
  ocean: { name: "Ocean", color: "#38bdf8", ink: "#082f49" },
  grape: { name: "Grape", color: "#c084fc", ink: "#2e1065" },
  rose: { name: "Rose", color: "#fb7185", ink: "#4c0519" },
  ember: { name: "Ember", color: "#fb923c", ink: "#431407" },
  sun: { name: "Sun", color: "#facc15", ink: "#422006" },
  forest: { name: "Forest", color: "#4ade80", ink: "#052e16" },
  slate: { name: "Slate", color: "#94a3b8", ink: "#0f172a" },
};

export function accentStyle(accent: Accent): CSSProperties {
  const a = ACCENTS[accent] ?? ACCENTS.slime;
  return { "--accent": a.color, "--accent-ink": a.ink } as CSSProperties;
}

/**
 * How a creative uses AI. Every way of working is welcome on People, so each has its own neutral
 * colour and mark (blue, teal, green): none of them is styled as a warning (owner, 2026-10-25).
 */
export const AI_USE: Record<AiUse, { label: string; short: string; hint: string; icon: string }> = {
  none: { label: "No AI", short: "No AI", hint: "Everything is made by hand.", icon: "✋" },
  assisted: { label: "AI-assisted", short: "AI-assisted", hint: "Uses AI for references or drafts, finished by hand.", icon: "🤝" },
  heavy: { label: "Mostly AI-generated", short: "Mostly AI", hint: "Output is largely generated, then edited.", icon: "✨" },
};

export function AiBadge({ ai }: { ai: AiUse }) {
  const a = AI_USE[ai];
  return (
    <span className={`badge ai-${ai}`} title={a.hint}>
      <span aria-hidden="true">{a.icon}</span> {a.label.toUpperCase()}
    </span>
  );
}

export function RatingLine({ rating, clients }: { rating: CreativeRating; clients: number }) {
  if (rating.percent === null) {
    const n = rating.not_slop + rating.slop;
    return (
      <span className="muted small">
        {n === 0 ? "No reviews yet" : `${n} review${n === 1 ? "" : "s"}`} · {clients} verified client{clients === 1 ? "" : "s"}
      </span>
    );
  }
  const cls = rating.percent >= 70 ? "good" : rating.percent < 40 ? "bad" : "mixed";
  return (
    <span className="small">
      <b className={`tile-rating ${cls}`}>{rating.percent}% Slaps</b>{" "}
      <span className="muted">
        · {rating.label} · {clients} verified client{clients === 1 ? "" : "s"}
      </span>
    </span>
  );
}

export function PersonCard({ c }: { c: CreativeCard }) {
  return (
    <Link className="panel person-card" to={`/people/${c.handle}`} style={accentStyle(c.accent)}>
      <div className="person-top">
        <Avatar name={c.name} url={c.avatar_url} className="p-avatar" />
        <div className="person-who">
          <b>{c.name}</b>
          <span className="muted small">
            {c.kind === "studio" ? `Studio · ${c.team_size ?? 2} people` : "Individual"}
            {c.location && ` · ${c.location}`}
          </span>
        </div>
      </div>
      {c.preview.length > 0 && (
        <div className="person-preview" aria-hidden="true">
          {c.preview.map((u) => (
            <img key={u} src={u} alt="" loading="lazy" />
          ))}
        </div>
      )}
      {c.headline && <span className="person-headline">{c.headline}</span>}
      <div className="chips">
        {c.skills.map((s) => (
          <span className="chip" key={s.slug}>
            {s.name}
          </span>
        ))}
      </div>
      <div className="person-foot">
        <span>{c.from_price ? <b>from {money(c.from_price.minor, c.from_price.currency)}</b> : <span className="muted">Ask for a quote</span>}</span>
        <AiBadge ai={c.ai_use} />
      </div>
      <div className="person-foot">
        <span className="small">
          <span className={`avail ${c.can_hire ? "on" : ""}`} aria-hidden="true" />
          {c.can_hire ? "Taking work" : c.taking_work ? "Not taking paid work yet" : "Fully booked"}
        </span>
        <RatingLine rating={c.rating} clients={c.clients} />
      </div>
    </Link>
  );
}

/** One portfolio work: a picture, a video (YouTube, loaded on press) or a track. */
export function WorkTile({ w, onOpen }: { w: Work; onOpen?: (w: Work) => void }) {
  const [playing, setPlaying] = useState(false);
  const label = w.title ?? (w.kind === "audio" ? "Track" : w.kind === "video" ? "Video" : "Picture");
  return (
    <figure className={`work work-${w.kind}`}>
      {w.kind === "image" && (
        <button type="button" className="work-pic" onClick={() => onOpen?.(w)} aria-label={`View larger: ${label}`}>
          <img src={w.url ?? ""} alt={label} loading="lazy" className={(w.width ?? 999) < 200 ? "pixelated" : ""} />
        </button>
      )}
      {w.kind === "video" &&
        (playing ? (
          <iframe
            src={`https://www.youtube-nocookie.com/embed/${w.youtube_id}?autoplay=1&rel=0`}
            title={label}
            allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
            allowFullScreen
            referrerPolicy="strict-origin-when-cross-origin"
          />
        ) : (
          <button
            type="button"
            className="work-pic"
            onClick={() => (isDesktop ? void openExternal(`https://www.youtube.com/watch?v=${w.youtube_id}`) : setPlaying(true))}
            aria-label={`Play video: ${label}`}
          >
            {w.url ? <img src={w.url} alt="" loading="lazy" /> : <span className="play-card">{label}</span>}
            <span className="play-mark" aria-hidden="true">
              ▶
            </span>
          </button>
        ))}
      {w.kind === "audio" && (
        <div className="work-audio">
          {w.url ? <img src={w.url} alt="" loading="lazy" /> : <span className="audio-mark" aria-hidden="true">♪</span>}
          {/* preload="none": nothing is fetched from GitHub until someone presses play. */}
          <audio controls preload="none" src={w.audio_url ?? undefined} aria-label={`Play: ${label}`} />
        </div>
      )}
      <figcaption>
        <b>{label}</b>
        {w.skill && <span className="faint">{w.skill.name}</span>}
        {w.caption && <span className="muted small">{w.caption}</span>}
      </figcaption>
    </figure>
  );
}

export function Lightbox({ w, onClose }: { w: Work; onClose: () => void }) {
  return (
    <div className="lightbox" role="dialog" aria-modal="true" aria-label={w.title ?? "Picture"} onClick={onClose}>
      <img src={w.url ?? ""} alt={w.title ?? ""} className={(w.width ?? 999) < 200 ? "pixelated" : ""} />
      <button type="button" className="btn" onClick={onClose} autoFocus>
        Close
      </button>
    </div>
  );
}

/** A creative's services; `onHire` adds a Hire button to each (left out on their own page). */
export function ServicesList({ services, onHire }: { services: CreativePage["services"]; onHire?: (s: CreativePage["services"][number]) => void }) {
  return (
    <div className="services">
      {services.map((s) => (
        <div className="panel panel-pad service" key={s.id}>
          <div className="service-head">
            <b>{s.name}</b>
            <span className="service-price">from {money(s.price_minor, s.currency)}</span>
          </div>
          {s.description && <p className="muted small">{s.description}</p>}
          <div className="service-foot">
            <span className="chip">{s.skill_name}</span>
            <span className="muted small">
              about {s.days} day{s.days === 1 ? "" : "s"}
            </span>
            {onHire && (
              <button type="button" className="btn small accent" onClick={() => onHire(s)}>
                Hire for this
              </button>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
