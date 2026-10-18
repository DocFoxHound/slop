import type { MouseEvent } from "react";
import { Link, useParams } from "react-router";

import { api, type AppDetail } from "../api";
import { Art, ErrorState, ExternalLink, Loading, StageBadge, StatusBadge } from "../components/ui";
import { ageLabel, daysSince, deadlineText, formatDate, STAGE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { openExternal } from "../platform";

export function AppPage() {
  const { slug = "" } = useParams();
  const app = useLoad(() => api.app(slug), [slug]);
  useDocumentTitle(app.data?.title);

  if (app.error) {
    return (
      <div className="page">
        <ErrorState error={app.error} onRetry={app.reload} />
      </div>
    );
  }
  if (!app.data) return <Loading />;
  return <AppView app={app.data} />;
}

function AppView({ app }: { app: AppDetail }) {
  const upcoming = app.stage !== "released";
  const deadline = deadlineText(app);
  const age = ageLabel(app.min_age);
  const firstCategory = app.kind === "game" ? { slug: "games", name: "Games" } : null;

  // Links inside the maker's Markdown leave Slop.
  const onDescriptionClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a[data-external]");
    if (link) {
      e.preventDefault();
      void openExternal((link as HTMLAnchorElement).href);
    }
  };

  return (
    <div className="page">
      <div className="crumbs">
        <Link to="/">Store</Link> ›{" "}
        {upcoming ? <Link to="/upcoming">Upcoming</Link> : firstCategory ? <Link to={`/browse/${firstCategory.slug}`}>{firstCategory.name}</Link> : <Link to="/search">Apps</Link>} ›{" "}
        {app.title}
      </div>

      {app.is_mine && app.status && app.status !== "listed" && (
        <div className="notice">
          <StatusBadge app={app} /> Only you can see this page.{" "}
          {app.status === "draft" && "It goes in the store once it's published and passes every scan."}{" "}
          <Link to={`/publish/${app.slug}`}>Edit draft</Link>
        </div>
      )}

      <div className="app-head">
        <Art app={app} withName={false} className="hero-art" />
        <div className="app-side">
          <div>
            <div className="title-row">
              <h1>{app.title}</h1>
              <StageBadge app={app} />
              {age && <span className="badge neutral" title={`Rated ${age}`}>{age}</span>}
            </div>
            <div className="muted">
              by <b className="text">{app.maker.display_name}</b> · {app.kind === "game" ? "Game" : "App"}
            </div>
          </div>
          {upcoming && (
            <div className="panel panel-pad stage-panel">
              {app.is_dead ? (
                <>
                  <b className="bad">Dead project</b>
                  <div className="muted">
                    No code changes in {app.last_commit_at ? daysSince(app.last_commit_at) : "90+"} days
                  </div>
                </>
              ) : (
                <>
                  <b>{deadline}</b>
                  <div className="muted">
                    {STAGE_LABEL[app.stage]} since {formatDate(app.stage_entered_at)}
                  </div>
                </>
              )}
            </div>
          )}
          <p className="lede">{app.summary}</p>
          <div className="chips">
            {app.tag_details.map((t) => (
              <Link className="chip" key={t.slug} to={`/tag/${t.slug}`}>
                {t.name}
              </Link>
            ))}
          </div>
          {app.is_mine && (
            <div>
              <Link className="btn" to={`/publish/${app.slug}`}>
                ✎ Edit
              </Link>
            </div>
          )}
          <dl className="kv">
            <dt>Source</dt>
            <dd>
              <ExternalLink href={app.repo_url}>{app.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
            </dd>
            <dt>Licence</dt>
            <dd>
              <code>{app.license_spdx}</code>
            </dd>
            <dt>Stage</dt>
            <dd>{STAGE_LABEL[app.stage]}</dd>
            {app.last_commit_at && (
              <>
                <dt>Last commit</dt>
                <dd>{formatDate(app.last_commit_at)}</dd>
              </>
            )}
          </dl>
        </div>
      </div>

      <div className="two-col">
        <div>
          {app.description_md.trim() ? (
            <div
              className="prose"
              onClick={onDescriptionClick}
              // Rendered with raw HTML disabled; see markdown.ts.
              dangerouslySetInnerHTML={{ __html: renderMarkdown(app.description_md) }}
            />
          ) : (
            <p className="muted">The maker hasn't written a description yet.</p>
          )}
        </div>
        <aside className="side-col">
          <div className="panel panel-pad install-box">
            <h3>Install</h3>
            <p className="muted">
              {app.stage === "coming_soon"
                ? "No builds yet. This is a Coming soon page."
                : "Installing through Slop is coming soon. For now, get it from the maker's releases."}
            </p>
            {app.stage !== "coming_soon" && (
              <ExternalLink href={`${app.repo_url}/releases`}>Maker's releases →</ExternalLink>
            )}
          </div>
          <div className="panel panel-pad trust">
            <div className="trust-row">
              <span className="ic">{"{ }"}</span>
              <span>
                <b>Open source</b> · {app.license_spdx} · <ExternalLink href={app.repo_url}>view code</ExternalLink>
              </span>
            </div>
            <div className="trust-row">
              <span className="ic">🛍</span>
              <span>
                <b>Nothing for sale</b> · donations never unlock anything
              </span>
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
