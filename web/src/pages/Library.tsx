// The library (DESIGN §6): what you've added, and on the desktop app, what's installed.

import { useEffect, useState } from "react";
import { Link } from "react-router";

import { api, type AppSummary, type AppUpdate } from "../api";
import { checkUpdates, installUpdates, LIBRARY_CHANGED } from "../components/AppUpdates";
import { InstallControls } from "../components/InstallControls";
import { GameMods } from "../components/ModControls";
import { isOn, useSandboxStatus } from "../components/Sandbox";
import { Art, ErrorState, Loading, StageBadge } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop, type InstalledApp, type SaveCopy } from "../platform";
import { useSession } from "../session";

interface Row {
  slug: string;
  title: string;
  subtitle: string;
  app?: AppSummary;
  installed?: InstalledApp;
  update?: AppUpdate;
  available?: boolean;
  /** A community build shown under its installed original. */
  nested?: boolean;
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
  const sandbox = useLoad(() => (isDesktop ? desktop.sandboxStatus() : Promise.resolve(undefined)), []);
  const privateSlop = useLoad(() => (me ? api.private.mine() : Promise.resolve({ mine: [], shared: [] })), [me?.id]);

  if (me === undefined || library.loading || installed.loading) return <Loading />;

  // Installed apps first (they work signed out and offline), then the rest of the library.
  // Community builds sit under their original when it's installed too; its Play button can
  // start them (DESIGN §9).
  const rows: Row[] = [];
  const all = installed.data ?? [];
  const originalOf = (app: InstalledApp) => all.find((o) => o.installed.slug === app.build_of)?.installed;
  const ordered = all
    .filter(({ installed: app }) => !originalOf(app))
    .flatMap((entry) => [entry, ...all.filter(({ installed: app }) => app.build_of === entry.installed.slug)]);
  for (const { installed: app, update, available } of ordered) {
    const entry = library.data?.find((e) => e.slug === app.slug);
    const original = originalOf(app);
    rows.push({
      slug: app.slug,
      title: app.title,
      subtitle: original
        ? `Community build of ${original.title} · by ${app.maker} · version ${app.version}`
        : `by ${app.maker} · version ${app.version}`,
      app: entry,
      installed: app,
      update,
      available,
      nested: Boolean(original),
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
            <li key={row.slug} className={`panel library-row${row.nested ? " nested" : ""}`}>
              <Link to={`/app/${row.slug}`} className="library-art" aria-label={row.title}>
                <Art app={{ slug: row.slug, title: row.title }} withName={false} />
              </Link>
              <div className="library-info">
                <div className="title-row">
                  <Link to={`/app/${row.slug}`} className="text">
                    <b>{row.title}</b>
                  </Link>
                  {row.app && <StageBadge app={row.app} />}
                  {row.nested && <span className="badge neutral">COMMUNITY</span>}
                  {row.installed && <span className="badge official">INSTALLED</span>}
                  {row.installed && sandbox.data?.available && isOn(sandbox.data, row.slug) && (
                    <span className="badge neutral">SANDBOX</span>
                  )}
                </div>
                <div className="muted small">{row.subtitle}</div>
                {row.installed && <UpdateLine row={row} />}
                {row.installed && (
                  <Community row={row} installed={all.map((e) => e.installed)} onChanged={installed.reload} />
                )}
                {row.installed && row.installed.kind !== "installer" && <Saves slug={row.slug} />}
              </div>
              <div className="library-actions">
                <InstallControls
                  slug={row.slug}
                  title={row.title}
                  inLibrary={Boolean(row.app)}
                  sandbox={
                    row.update
                      ? { suggested: Boolean(row.update.sandbox_suggested), offline: !row.update.sandbox_suggested }
                      : undefined
                  }
                />
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
  const sandbox = useSandboxStatus();
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
      {sandbox.data?.available && (
        <label className="check">
          <input
            type="checkbox"
            checked={sandbox.data.settings.all}
            onChange={(e) => void desktop.setSandbox(null, e.target.checked).then(sandbox.reload)}
          />{" "}
          Run every app in sandbox mode <span className="muted small">(unless you turned it off for one)</span>
        </label>
      )}
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

/** An installed app's community: other builds to install beside it, and mods to add (DESIGN §9). */
function Community({ row, installed, onChanged }: { row: Row; installed: InstalledApp[]; onChanged: () => void }) {
  const builds = row.update?.builds ?? [];
  const mods = row.update?.mods ?? [];
  const added = useLoad(() => desktop.installedMods(row.slug), [row.slug]);
  const addedCount = added.data?.length ?? 0;
  const others = builds.filter((b) => b.slug !== row.slug && !installed.some((a) => a.slug === b.slug));
  if (others.length === 0 && mods.length === 0 && addedCount === 0) return null;
  const parts = [
    addedCount > 0 ? `${addedCount} ${addedCount === 1 ? "mod" : "mods"} added` : mods.length > 0 ? `${mods.length} ${mods.length === 1 ? "mod" : "mods"}` : null,
    others.length > 0 ? `${others.length} other ${others.length === 1 ? "build" : "builds"}` : null,
  ].filter(Boolean);
  return (
    <details className="library-community small">
      <summary>Community: {parts.join(" · ")}</summary>
      {row.installed && others.length > 0 && (
        <OtherBuilds current={row.installed} builds={builds} others={others} onInstalled={onChanged} />
      )}
      <GameMods game={row.slug} available={mods} onChanged={added.reload} />
    </details>
  );
}

/** Install the official app or a community build beside this one (DESIGN §6, §9). Community
 * builds of an installed original show in its Play button's list. */
function OtherBuilds({
  current,
  builds,
  others,
  onInstalled,
}: {
  current: InstalledApp;
  builds: AppUpdate["builds"];
  others: AppUpdate["builds"];
  onInstalled: () => void;
}) {
  const platform = useLoad(() => desktop.platformInfo(), []);
  const [target, setTarget] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [bringSaves, setBringSaves] = useState(true);
  const [playIt, setPlayIt] = useState(true);
  const os = platform.data?.platform;
  const options = others.filter((b) => os && b.platforms.includes(os));
  if (options.length === 0) return null;
  const chosen = options.find((b) => b.slug === target);
  const original = builds.find((b) => b.official);
  const install = async () => {
    if (!chosen) return;
    setBusy(true);
    setError(undefined);
    try {
      await desktop.install(chosen.slug);
      if (bringSaves) await desktop.copySaves(current.slug, chosen.slug);
      if (playIt && original) await desktop.setPlayChoice(original.slug, chosen.official ? null : chosen.slug);
      setTarget(undefined);
      window.dispatchEvent(new Event(LIBRARY_CHANGED));
      onInstalled();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="small build-switch">
      <label>
        Other builds{" "}
        <select value={target ?? ""} onChange={(e) => setTarget(e.target.value || undefined)}>
          <option value="">Choose one to install…</option>
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
            Install <b>{chosen.title}</b> beside <b>{current.title}</b>?{" "}
            {!chosen.official &&
              "Community builds are made by other people from the original's code, and checked and reviewed like any app."}{" "}
            Each build has its own saves.
          </p>
          <label className="check">
            <input type="checkbox" checked={bringSaves} onChange={(e) => setBringSaves(e.target.checked)} /> Copy my saves to{" "}
            {chosen.title} <span className="muted">(a fork may store saves differently)</span>
          </label>
          {original && (
            <label className="check">
              <input type="checkbox" checked={playIt} onChange={(e) => setPlayIt(e.target.checked)} /> Make it what{" "}
              {original.title}'s Play button starts <span className="muted">(change it any time from Play's list)</span>
            </label>
          )}
          <div className="actions">
            <button className="btn small ghost" onClick={() => setTarget(undefined)} disabled={busy}>
              Cancel
            </button>
            <button className="btn small slime" onClick={() => void install()} disabled={busy}>
              {busy ? "Installing…" : "Install"}
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

/** An installed app's save folder: copies taken before sandboxed sessions, to go back to. */
function Saves({ slug }: { slug: string }) {
  const [open, setOpen] = useState(false);
  const copies = useLoad(() => (open ? desktop.saveCopies(slug) : Promise.resolve([] as SaveCopy[])), [open, slug]);
  const [done, setDone] = useState<string>();
  const [error, setError] = useState<string>();
  return (
    <details className="library-community small" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>Saves</summary>
      <p className="muted">
        Kept in Slop's save folder for this app, the same in normal and sandbox mode, and kept when you uninstall. Slop copies
        them before each sandboxed session.
      </p>
      {copies.data && copies.data.length === 0 && <p className="muted">No copies yet.</p>}
      <ul className="plain">
        {copies.data?.map((c) => (
          <li key={c.id}>
            {new Date(c.taken_at).toLocaleString()} · {Math.max(1, Math.round(c.bytes / 1024))} KB{" "}
            <button
              className="btn small ghost"
              onClick={() => {
                setError(undefined);
                void desktop.restoreSaves(slug, c.id).then(
                  () => {
                    setDone(c.id);
                    copies.reload();
                  },
                  (err: unknown) => setError(err instanceof Error ? err.message : String(err)),
                );
              }}
            >
              Go back to this
            </button>
            {done === c.id && <span className="muted"> Restored. Your saves before that were copied too.</span>}
          </li>
        ))}
      </ul>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </details>
  );
}
