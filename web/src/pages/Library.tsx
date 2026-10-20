// The library (DESIGN §6): what you've added, and on the desktop app, what's installed.

import { useEffect, useState } from "react";
import { Link } from "react-router";

import { api, type AppSummary, type AppUpdate } from "../api";
import { checkUpdates, installUpdates, LIBRARY_CHANGED } from "../components/AppUpdates";
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
  update?: AppUpdate;
  available?: boolean;
}

export function Library() {
  useDocumentTitle("Library");
  const { me } = useSession();
  const library = useLoad(() => (me ? api.library() : Promise.resolve([])), [me?.id]);
  const installed = useLoad(() => (isDesktop ? checkUpdates() : Promise.resolve([])), []);
  useEffect(() => {
    const reload = () => installed.reload();
    window.addEventListener(LIBRARY_CHANGED, reload);
    return () => window.removeEventListener(LIBRARY_CHANGED, reload);
  }, [installed]);
  const interested = useLoad(() => (me ? api.interested.mine() : Promise.resolve([])), [me?.id]);
  const privateSlop = useLoad(() => (me ? api.private.mine() : Promise.resolve({ mine: [], shared: [] })), [me?.id]);

  if (me === undefined || library.loading || installed.loading) return <Loading />;

  // Installed apps first (they work signed out and offline), then the rest of the library.
  const rows: Row[] = [];
  for (const { installed: app, update, available } of installed.data ?? []) {
    const entry = library.data?.find((e) => e.slug === app.slug);
    rows.push({
      slug: app.slug,
      title: app.title,
      subtitle: `by ${app.maker} · version ${app.version}`,
      app: entry,
      installed: app,
      update,
      available,
    });
  }
  const waiting = rows.filter((r) => r.available).length;
  for (const entry of library.data ?? []) {
    if (!rows.some((r) => r.slug === entry.slug)) {
      rows.push({ slug: entry.slug, title: entry.title, subtitle: entry.summary, app: entry });
    }
  }

  return (
    <div className="page">
      <h1 className="page-title">Library</h1>
      {isDesktop && (installed.data?.length ?? 0) > 0 && <UpdateBar waiting={waiting} onUpdated={installed.reload} />}
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
                {row.installed && <UpdateLine row={row} />}
                {row.installed && row.update && row.update.builds.length > 1 && (
                  <BuildSwitcher current={row.installed} builds={row.update.builds} onSwitched={installed.reload} />
                )}
              </div>
              <div className="library-actions">
                <InstallControls slug={row.slug} title={row.title} inLibrary={Boolean(row.app)} />
              </div>
            </li>
          ))}
        </ul>
      )}

      {((privateSlop.data?.mine.length ?? 0) > 0 || (privateSlop.data?.shared.length ?? 0) > 0) && (
        <>
          <h2 className="section-title">Private Slop</h2>
          <p className="muted small">Yours, and what friends shared with you. Only the people it's shared with can see these.</p>
          <ul className="library-list">
            {[...(privateSlop.data?.mine ?? []).map((a) => ({ ...a, owner: "you" })), ...(privateSlop.data?.shared ?? [])].map((a) => (
              <li key={a.slug} className="panel library-row">
                <Link to={`/app/${a.slug}`} className="library-art" aria-label={a.title}>
                  <Art app={a} withName={false} />
                </Link>
                <div className="library-info">
                  <div className="title-row">
                    <Link to={`/app/${a.slug}`} className="text">
                      <b>{a.title}</b>
                    </Link>
                    <span className="badge neutral">PRIVATE</span>
                  </div>
                  <div className="muted small">by {a.owner}</div>
                </div>
                <div className="library-actions">
                  <InstallControls slug={a.slug} title={a.title} inLibrary={Boolean(library.data?.some((e) => e.slug === a.slug))} />
                </div>
              </li>
            ))}
          </ul>
        </>
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

/** "Update apps automatically", and updating everything now. */
function UpdateBar({ waiting, onUpdated }: { waiting: number; onUpdated: () => void }) {
  const auto = useLoad(() => desktop.autoUpdateSetting(), []);
  const [busy, setBusy] = useState(false);
  return (
    <div className="update-bar panel panel-pad">
      <label className="check">
        <input
          type="checkbox"
          checked={auto.data ?? true}
          disabled={auto.loading}
          onChange={(e) => void desktop.autoUpdateSetting(e.target.checked).then(auto.reload)}
        />{" "}
        Update apps automatically <span className="muted small">(never while an app is running)</span>
      </label>
      {waiting > 0 && (
        <button
          className="btn small slime"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void installUpdates().finally(() => {
              setBusy(false);
              onUpdated();
            });
          }}
        >
          {busy ? "Updating…" : `Update ${waiting === 1 ? "1 app" : `${waiting} apps`} now`}
        </button>
      )}
    </div>
  );
}

/** What's new for an installed app: an approved update, or one still being checked. */
function UpdateLine({ row }: { row: Row }) {
  const u = row.update;
  if (!u) return null;
  return (
    <div className="small">
      {row.available && u.latest && (
        <span className="badge soon">UPDATE TO {u.latest.version} READY</span>
      )}{" "}
      {u.pending && (
        <span className="muted">
          {u.pending.version} is being checked:{" "}
          {u.pending.status === "fetching" ? "downloading and scanning" : "scanned, waiting for a person to review it"}.
        </span>
      )}
    </div>
  );
}

/** Switch between the official app and its community builds (DESIGN §6, §9). */
function BuildSwitcher({ current, builds, onSwitched }: { current: InstalledApp; builds: AppUpdate["builds"]; onSwitched: () => void }) {
  const platform = useLoad(() => desktop.platformInfo(), []);
  const [target, setTarget] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const os = platform.data?.platform;
  const options = builds.filter((b) => b.slug === current.slug || (os && b.platforms.includes(os)));
  if (options.length < 2) return null;
  const chosen = options.find((b) => b.slug === target);
  const switchTo = async () => {
    if (!chosen) return;
    setBusy(true);
    setError(undefined);
    try {
      await desktop.install(chosen.slug);
      await desktop.uninstall(current.slug);
      setTarget(undefined);
      onSwitched();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="small build-switch">
      <label>
        Build{" "}
        <select value={target ?? current.slug} onChange={(e) => setTarget(e.target.value === current.slug ? undefined : e.target.value)}>
          {options.map((b) => (
            <option key={b.slug} value={b.slug}>
              {b.official ? `${b.title} (official)` : `${b.title} (community)`}
            </option>
          ))}
        </select>
      </label>
      {chosen && (
        <div className="notice">
          <p>
            Install <b>{chosen.title}</b> and uninstall <b>{current.title}</b>?{" "}
            {!chosen.official && "Community builds are made by other people from the original's code and reviewed like any app."} Most
            games keep saves in your user folder, so they usually carry over, but check the game's own notes.
          </p>
          <div className="actions">
            <button className="btn small ghost" onClick={() => setTarget(undefined)} disabled={busy}>
              Cancel
            </button>
            <button className="btn small slime" onClick={() => void switchTo()} disabled={busy}>
              {busy ? "Switching…" : "Switch"}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
