// SPDX-License-Identifier: AGPL-3.0-or-later
// Small shared pieces: app artwork, tiles, badges, links, loading and error states.

import type { MouseEvent, ReactNode } from "react";
import { Link } from "react-router";

import type { AppSummary } from "../api";
import { ApiError } from "../api";
import { ageLabel, artFor, deadlineText } from "../format";
import { openExternal } from "../platform";
import { AppIcon, appIconProps } from "./AppIcon";

/** The maker's thumbnail, or a colour made from the app's address with its initial. */
export function Art({
  app,
  withName = true,
  className = "",
}: {
  app: Pick<AppSummary, "slug" | "title"> & { thumbnail_url?: string | null };
  withName?: boolean;
  className?: string;
}) {
  if (app.thumbnail_url) {
    return (
      <div className={`art has-image ${className}`} aria-hidden="true">
        <img src={app.thumbnail_url} alt="" loading="lazy" decoding="async" />
      </div>
    );
  }
  const { from, to } = artFor(app.slug);
  return (
    <div className={`art ${className}`} style={{ background: `linear-gradient(135deg, ${from}, ${to})` }} aria-hidden="true">
      <span className="initial">{app.title.trim().charAt(0).toUpperCase()}</span>
      {withName && <span className="nm">{app.title}</span>}
    </div>
  );
}

export function StageBadge({ app }: { app: Pick<AppSummary, "stage" | "is_dead"> }) {
  if (app.is_dead) return <span className="badge bad" title="No code changes for 90 days">DEAD PROJECT</span>;
  if (app.stage === "coming_soon") return <span className="badge soon">COMING SOON</span>;
  if (app.stage === "beta") return <span className="badge beta">BETA</span>;
  return null;
}

export function StatusBadge({ app }: { app: Pick<AppSummary, "status"> }) {
  if (app.status === "draft") return <span className="badge neutral">DRAFT</span>;
  if (app.status === "delisted") return <span className="badge bad">DELISTED</span>;
  return null;
}

/** A store tile. `reason` says why it's recommended ("Because you like …"). */
export function AppTile({ app, reason }: { app: AppSummary; reason?: string }) {
  const deadline = deadlineText(app);
  const age = ageLabel(app.min_age);
  return (
    <Link className="tile" to={`/app/${app.slug}`}>
      <div className="tile-art">
        <Art app={app} withName={false} />
        <span className="stage-corner">
          <StageBadge app={app} />
          <StatusBadge app={app} />
        </span>
      </div>
      <div className="tile-body">
        <span className="tile-title">
          <AppIcon {...appIconProps(app)} size={24} />
          <span className="tile-name">{app.title}</span>
        </span>
        {reason && <span className="why">{reason}</span>}
        {app.branch_of && <span className="badge neutral community-badge">COMMUNITY</span>}
        {app.made_with_people && (
          <span className="made-with-people" title="Made with creatives hired through People">
            ✋ Made with people
          </span>
        )}
        <span className="d">{app.summary}</span>
        {app.not_slop_percent !== null && (
          <span className={`tile-rating ${app.not_slop_percent >= 70 ? "good" : app.not_slop_percent < 40 ? "bad" : "mixed"}`}>
            {app.not_slop_percent}% Slaps <span className="faint">· {app.vote_count.toLocaleString()} votes</span>
          </span>
        )}
        {app.players_now !== null && app.players_now > 0 && (
          <div className="tile-stat live">
            <span className="live-dot" aria-hidden="true" /> {app.players_now.toLocaleString()} playing now
          </div>
        )}
        {app.stage !== "released" && app.interested > 0 && (
          <div className="tile-stat">★ {app.interested.toLocaleString()} interested</div>
        )}
        {deadline && !app.is_dead && <div className="tile-stat">{deadline}</div>}
        <div className="tile-foot">
          <div className="chips">
            {app.tags.slice(0, 3).map((t) => (
              <span className="chip" key={t}>
                {t}
              </span>
            ))}
          </div>
          {age && <span className="age" title={`Rated ${age}`}>{age}</span>}
        </div>
      </div>
    </Link>
  );
}

export function Shelf({ apps, empty }: { apps: Array<AppSummary & { reason?: string }> | undefined; empty: ReactNode }) {
  if (!apps) return <div className="shelf">{[0, 1, 2, 3].map((i) => <div className="tile skeleton" key={i} />)}</div>;
  if (apps.length === 0) return <div className="empty-state">{empty}</div>;
  return (
    <div className="shelf">
      {apps.map((a) => (
        <AppTile app={a} key={a.slug} reason={a.reason} />
      ))}
    </div>
  );
}

/** A link that leaves SLAPorSLOP. In the desktop app it opens the system browser. */
export function ExternalLink({ href, children, className }: { href: string; children: ReactNode; className?: string }) {
  const onClick = (e: MouseEvent) => {
    e.preventDefault();
    void openExternal(href);
  };
  return (
    <a href={href} onClick={onClick} rel="noopener noreferrer" target="_blank" className={className}>
      {children}
    </a>
  );
}

export function ErrorState({ error, onRetry }: { error: ApiError; onRetry?: () => void }) {
  return (
    <div className="empty-state" role="alert">
      <p>{error.status === 404 ? "We couldn't find that." : error.message}</p>
      {onRetry && error.status !== 404 && (
        <button className="btn" onClick={onRetry}>
          Try again
        </button>
      )}
    </div>
  );
}

export function Loading({ label = "Loading…" }: { label?: string }) {
  return (
    <div className="empty-state muted" aria-busy="true">
      {label}
    </div>
  );
}

export function SectionHeader({ title, sub, children }: { title: string; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="section-h">
      <h2>{title}</h2>
      {sub && <span className="sub">{sub}</span>}
      {children}
    </div>
  );
}
