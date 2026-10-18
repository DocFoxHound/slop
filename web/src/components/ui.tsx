// Small shared pieces: app artwork, tiles, badges, links, loading and error states.

import type { MouseEvent, ReactNode } from "react";
import { Link } from "react-router";

import type { AppSummary } from "../api";
import { ApiError } from "../api";
import { ageLabel, artFor, deadlineText } from "../format";
import { openExternal } from "../platform";

export function Art({ app, withName = true, className = "" }: { app: Pick<AppSummary, "slug" | "title">; withName?: boolean; className?: string }) {
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

export function AppTile({ app }: { app: AppSummary }) {
  const deadline = deadlineText(app);
  const age = ageLabel(app.min_age);
  return (
    <Link className="tile" to={`/app/${app.slug}`}>
      <div className="tile-art">
        <Art app={app} />
        <span className="stage-corner">
          <StageBadge app={app} />
          <StatusBadge app={app} />
        </span>
      </div>
      <div className="tile-body">
        <span className="d">{app.summary}</span>
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

export function Shelf({ apps, empty }: { apps: AppSummary[] | undefined; empty: ReactNode }) {
  if (!apps) return <div className="shelf">{[0, 1, 2, 3].map((i) => <div className="tile skeleton" key={i} />)}</div>;
  if (apps.length === 0) return <div className="empty-state">{empty}</div>;
  return (
    <div className="shelf">
      {apps.map((a) => (
        <AppTile app={a} key={a.slug} />
      ))}
    </div>
  );
}

/** A link that leaves Slop. In the desktop app it opens the system browser. */
export function ExternalLink({ href, children }: { href: string; children: ReactNode }) {
  const onClick = (e: MouseEvent) => {
    e.preventDefault();
    void openExternal(href);
  };
  return (
    <a href={href} onClick={onClick} rel="noopener noreferrer" target="_blank">
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
