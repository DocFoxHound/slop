// Install, launch and library buttons for one app.
//
// Desktop: the Rust side downloads the maker's file, checks it against Slop's signed manifest,
// and installs it into the library folder (docs/ARCHITECTURE.md §6.3). The website can only add
// apps to the library; installing happens in the desktop app.

import { useCallback, useEffect, useState } from "react";
import { Link, useLocation } from "react-router";

import { api } from "../api";
import { useLoad } from "../hooks";
import { desktop, isDesktop, type InstalledApp, type Launched } from "../platform";
import {
  decided,
  LaunchNotices,
  SandboxFailedNotice,
  SandboxPrompt,
  SandboxSwitch,
  useSandboxFailed,
  useSandboxStatus,
  type SandboxHint,
} from "./Sandbox";
import { useSession } from "../session";

function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1e3))} KB`;
}

export function InstallControls({
  slug,
  title,
  inLibrary,
  sandbox,
}: {
  slug: string;
  title: string;
  inLibrary?: boolean;
  /** For suggesting sandbox mode (DESIGN §6). */
  sandbox?: SandboxHint;
}) {
  return isDesktop ? (
    <DesktopControls slug={slug} title={title} sandbox={sandbox} />
  ) : (
    <WebControls slug={slug} inLibrary={inLibrary} />
  );
}

function WebControls({ slug, inLibrary }: { slug: string; inLibrary?: boolean }) {
  const { me } = useSession();
  const location = useLocation();
  const [added, setAdded] = useState(inLibrary ?? false);
  const [error, setError] = useState<string>();
  useEffect(() => setAdded(inLibrary ?? false), [inLibrary]);

  const toggle = async () => {
    setError(undefined);
    try {
      if (added) await api.removeFromLibrary(slug);
      else await api.addToLibrary(slug);
      setAdded(!added);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="install-controls">
      {me ? (
        <button className={`btn ${added ? "" : "slime"}`} onClick={() => void toggle()} aria-pressed={added}>
          {added ? "✓ In your library" : "+ Add to library"}
        </button>
      ) : (
        <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(location.pathname)}`}>
          Sign in to add to your library
        </Link>
      )}
      <p className="muted small">Install it with the Slop desktop app. It checks every file against Slop's signature first.</p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function DesktopControls({ slug, title, sandbox }: { slug: string; title: string; sandbox?: SandboxHint }) {
  const build = useLoad(() => api.hasBuild(slug), [slug]);
  const sandboxStatus = useSandboxStatus();
  const [askSandbox, setAskSandbox] = useState(false);
  const [launched, setLaunched] = useState<Launched>();
  const [failed, clearFailed] = useSandboxFailed(slug);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const [deleteSaves, setDeleteSaves] = useState(false);
  const [installed, setInstalled] = useState<InstalledApp | null>();
  const [busy, setBusy] = useState<"install" | "launch" | "uninstall" | null>(null);
  const [progress, setProgress] = useState<{ got: number; total: number }>();
  const [error, setError] = useState<string>();
  const [confirmInstaller, setConfirmInstaller] = useState(false);

  const reload = useCallback(async () => {
    const apps = await desktop.installed();
    setInstalled(apps.find((a) => a.slug === slug) ?? null);
  }, [slug]);

  useEffect(() => {
    void reload();
    let unlisten: (() => void) | undefined;
    void desktop
      .onProgress((p) => {
        if (p.slug === slug) setProgress({ got: p.got, total: p.total });
      })
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [reload, slug]);

  const run = async (kind: "install" | "launch" | "uninstall", action: () => Promise<unknown>) => {
    setBusy(kind);
    setError(undefined);
    try {
      await action();
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
      setProgress(undefined);
    }
  };

  const launch = (normalOnce = false) => {
    clearFailed();
    void run("launch", async () => setLaunched(await desktop.launch(slug, normalOnce)));
  };
  const play = () => {
    const status = sandboxStatus.data;
    if (status?.available && sandbox?.suggested && !decided(status, slug)) setAskSandbox(true);
    else launch();
  };

  if (installed === undefined || build.loading) return <div className="install-controls muted">Checking…</div>;

  const installing = busy === "install";
  const pct = progress && progress.total ? Math.floor((progress.got / progress.total) * 100) : 0;

  return (
    <div className="install-controls">
      {installed ? (
        <>
          {installed.kind === "installer" ? (
            confirmInstaller ? (
              <div className="notice">
                <p>
                  <b>{title}</b> runs its own installer, which may ask for admin rights. Slop checked that it's the exact file
                  the maker submitted and Slop approved.
                </p>
                <div className="actions">
                  <button className="btn ghost" onClick={() => setConfirmInstaller(false)}>
                    Cancel
                  </button>
                  <button
                    className="btn slime"
                    onClick={() => {
                      setConfirmInstaller(false);
                      void run("launch", () => desktop.runInstaller(slug));
                    }}
                  >
                    Run installer
                  </button>
                </div>
              </div>
            ) : (
              <button className="btn big slime" onClick={() => setConfirmInstaller(true)}>
                Run installer…
              </button>
            )
          ) : (
            <button className="btn big slime" disabled={busy !== null} onClick={play}>
              {busy === "launch" ? "Starting…" : "▶ Play"}
            </button>
          )}
          {askSandbox && (
            <SandboxPrompt
              title={title}
              onChoose={(on) => {
                setAskSandbox(false);
                void desktop.setSandbox(slug, on).then(() => {
                  sandboxStatus.reload();
                  launch();
                });
              }}
            />
          )}
          {installed.kind !== "installer" && sandboxStatus.data && (
            <SandboxSwitch slug={slug} status={sandboxStatus.data} hint={sandbox} onChanged={sandboxStatus.reload} />
          )}
          {launched && <LaunchNotices launched={launched} title={title} />}
          {failed && (
            <SandboxFailedNotice
              slug={slug}
              title={title}
              onRunNormally={() => launch(true)}
              onTurnOff={() => {
                clearFailed();
                void desktop.setSandbox(slug, false).then(sandboxStatus.reload);
              }}
              onClose={clearFailed}
            />
          )}
          <p className="muted small">Version {installed.version} is installed.</p>
          <div className="row-actions">
            <button
              className="btn small"
              disabled={busy !== null || !build.data}
              onClick={() => void run("install", () => desktop.install(slug))}
            >
              {installing ? `Updating… ${pct}%` : "Reinstall / update"}
            </button>
            <button className="btn small ghost danger" disabled={busy !== null} onClick={() => setConfirmUninstall(true)}>
              Uninstall
            </button>
          </div>
          {confirmUninstall && (
            <div className="notice">
              <p>
                Uninstall <b>{title}</b>? Its mods go too. Your saves stay, so reinstalling picks up where you left off.
              </p>
              <label className="check">
                <input type="checkbox" checked={deleteSaves} onChange={(e) => setDeleteSaves(e.target.checked)} /> Delete my
                saves too
              </label>
              <div className="actions">
                <button className="btn small ghost" onClick={() => setConfirmUninstall(false)}>
                  Cancel
                </button>
                <button
                  className="btn small danger"
                  onClick={() => {
                    setConfirmUninstall(false);
                    void run("uninstall", () => desktop.uninstall(slug, deleteSaves));
                  }}
                >
                  Uninstall
                </button>
              </div>
            </div>
          )}
        </>
      ) : build.data ? (
        <>
          <button className="btn big slime" disabled={busy !== null} onClick={() => void run("install", () => desktop.install(slug))}>
            {installing ? (progress ? `Installing… ${pct}%` : "Starting download…") : "Install"}
          </button>
          {installing && progress && (
            <>
              <progress max={progress.total} value={progress.got} aria-label="Download progress" />
              <p className="muted small">
                {formatBytes(progress.got)} of {formatBytes(progress.total)}, from the maker's release page
              </p>
            </>
          )}
          {!installing && (
            <p className="muted small">Downloaded from the maker's release page and checked against Slop's signature.</p>
          )}
        </>
      ) : (
        <p className="muted">No approved build yet.</p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
