// The library (DESIGN §6): what you've added, and on the desktop app, what's installed.

import { Link } from "react-router";

import { api, type AppSummary } from "../api";
import { InstallControls } from "../components/InstallControls";
import { Art, ErrorState, Loading, StageBadge } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop, type InstalledApp } from "../platform";
import { useSession } from "../session";

interface Row {
  slug: string;
  title: string;
  subtitle: string;
  app?: AppSummary;
  installed?: InstalledApp;
}

export function Library() {
  useDocumentTitle("Library");
  const { me } = useSession();
  const library = useLoad(() => (me ? api.library() : Promise.resolve([])), [me?.id]);
  const installed = useLoad(() => (isDesktop ? desktop.installed() : Promise.resolve([])), []);
  const interested = useLoad(() => (me ? api.interested.mine() : Promise.resolve([])), [me?.id]);

  if (me === undefined || library.loading || installed.loading) return <Loading />;

  // Installed apps first (they work signed out and offline), then the rest of the library.
  const rows: Row[] = [];
  for (const app of installed.data ?? []) {
    const entry = library.data?.find((e) => e.slug === app.slug);
    rows.push({ slug: app.slug, title: app.title, subtitle: `by ${app.maker} · version ${app.version}`, app: entry, installed: app });
  }
  for (const entry of library.data ?? []) {
    if (!rows.some((r) => r.slug === entry.slug)) {
      rows.push({ slug: entry.slug, title: entry.title, subtitle: entry.summary, app: entry });
    }
  }

  return (
    <div className="page">
      <h1 className="page-title">Library</h1>
      {!me && (
        <p className="notice">
          <Link to="/signin?return_to=/library">Sign in</Link> to keep a library that follows you between devices.
          {isDesktop && " Apps installed on this computer are shown below either way."}
        </p>
      )}
      {library.error && <ErrorState error={library.error} onRetry={library.reload} />}
      {rows.length === 0 && !interested.data?.length ? (
        <div className="empty-state">
          Nothing here yet. <Link to="/">Find something in the store</Link>.
        </div>
      ) : (
        <ul className="library-list">
          {rows.map((row) => (
            <li key={row.slug} className="panel library-row">
              <Link to={`/app/${row.slug}`} className="library-art" aria-label={row.title}>
                <Art app={{ slug: row.slug, title: row.title }} withName={false} />
              </Link>
              <div className="library-info">
                <div className="title-row">
                  <Link to={`/app/${row.slug}`} className="text">
                    <b>{row.title}</b>
                  </Link>
                  {row.app && <StageBadge app={row.app} />}
                  {row.installed && <span className="badge official">INSTALLED</span>}
                </div>
                <div className="muted small">{row.subtitle}</div>
              </div>
              <div className="library-actions">
                <InstallControls slug={row.slug} title={row.title} inLibrary={Boolean(row.app)} />
              </div>
            </li>
          ))}
        </ul>
      )}

      {(interested.data?.length ?? 0) > 0 && (
        <>
          <h2 className="section-title">Interested</h2>
          <p className="muted small">Upcoming apps you're waiting for. We tell you when each reaches beta or is released.</p>
          <ul className="library-list">
            {interested.data!.map((a) => (
              <li key={a.slug} className="panel library-row">
                <Link to={`/app/${a.slug}`} className="library-art" aria-label={a.title}>
                  <Art app={a} withName={false} />
                </Link>
                <div className="library-info">
                  <div className="title-row">
                    <Link to={`/app/${a.slug}`} className="text">
                      <b>{a.title}</b>
                    </Link>
                    <StageBadge app={a} />
                  </div>
                  <div className="muted small">
                    {a.stage === "released" ? "Released: add it to your library to install it." : a.summary}
                  </div>
                </div>
                <div className="library-actions">
                  {a.stage === "released" ? (
                    <InstallControls slug={a.slug} title={a.title} inLibrary={false} />
                  ) : (
                    <button
                      type="button"
                      className="btn small ghost"
                      onClick={() => void api.interested.remove(a.slug).then(interested.reload)}
                    >
                      Not interested
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
