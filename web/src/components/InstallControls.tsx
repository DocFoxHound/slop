// SPDX-License-Identifier: AGPL-3.0-or-later
// Install, launch and library buttons for one app.
//
// Desktop: the Rust side downloads the maker's file, checks it against Slop's signed manifest,
// and installs it into the library folder (docs/ARCHITECTURE.md §6.3). The website can only add
// apps to the library; installing happens in the desktop app.

import { useCallback, useEffect, useState } from "react";
import { Link, useLocation } from "react-router";

import { api } from "../api";
import { useLoad } from "../hooks";
import { LIBRARY_CHANGED } from "./AppUpdates";
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

function DesktopControls({ slug, title, sandbox: given }: { slug: string; title: string; sandbox?: SandboxHint }) {
  const build = useLoad(() => api.hasBuild(slug), [slug]);
  const sandboxStatus = useSandboxStatus();
  const [askSandbox, setAskSandbox] = useState(false);
  const [launched, setLaunched] = useState<Launched>();
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const [deleteSaves, setDeleteSaves] = useState(false);
  const [installed, setInstalled] = useState<InstalledApp | null>();
  // Installed community builds of this app, and the one its Play button starts (DESIGN §9).
  const [builds, setBuilds] = useState<InstalledApp[]>([]);
  const [choice, setChoice] = useState<string>();
  const [offerSaves, setOfferSaves] = useState<{ from: InstalledApp; to: InstalledApp }>();
  const [busy, setBusy] = useState<"install" | "launch" | "uninstall" | null>(null);
  const [progress, setProgress] = useState<{ got: number; total: number }>();
  const [error, setError] = useState<string>();
  const [confirmInstaller, setConfirmInstaller] = useState(false);

  const reload = useCallback(async () => {
    const [apps, choices] = await Promise.all([desktop.installed(), desktop.playChoices()]);
    setInstalled(apps.find((a) => a.slug === slug) ?? null);
    setBuilds(apps.filter((a) => a.build_of === slug));
    setChoice(choices[slug]);
  }, [slug]);

  // What Play starts: the picked community build while it's installed, else this app.
  const target = (choice && builds.find((b) => b.slug === choice)) || installed || undefined;
  const targetSlug = target?.slug ?? slug;
  const targetTitle = target?.title ?? title;
  const [failed, clearFailed] = useSandboxFailed(targetSlug);
  // A mod that goes online makes sandbox mode suggested for the game it's added to.
  const added = useLoad(() => desktop.installedMods(targetSlug), [targetSlug]);
  const reloadMods = added.reload;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    void desktop.onModsChanged(() => reloadMods()).then((u) => (gone ? u() : (unlisten = u)));
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [reloadMods]);
  const onlineMod = (added.data ?? []).find((m) => given?.onlineMods?.includes(m.slug));
  const sandbox: SandboxHint | undefined =
    given && onlineMod ? { ...given, suggested: true, offline: false } : given;

  useEffect(() => {
    void reload();
    // Another part of the page installed or removed something (a community build, say).
    const changed = () => void reload();
    window.addEventListener(LIBRARY_CHANGED, changed);
    let unlisten: (() => void) | undefined;
    void desktop
      .onProgress((p) => {
        if (p.slug === slug) setProgress({ got: p.got, total: p.total });
      })
      .then((u) => (unlisten = u));
    return () => {
      window.removeEventListener(LIBRARY_CHANGED, changed);
      unlisten?.();
    };
  }, [reload, slug]);

  const run = async (kind: "install" | "launch" | "uninstall", action: () => Promise<unknown>) => {
    setBusy(kind);
    setError(undefined);
    try {
      await action();
      await reload();
      if (kind !== "launch") window.dispatchEvent(new Event(LIBRARY_CHANGED));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
      setProgress(undefined);
    }
  };

  const launch = (normalOnce = false) => {
    clearFailed();
    void run("launch", async () => setLaunched(await desktop.launch(targetSlug, normalOnce)));
  };
  const [needs, setNeeds] = useState<Array<{ id: string; name: string; vendor: string }>>();
  const play = async () => {
    // Shared runtimes first: the player agrees before anything is installed.
    const missing = await desktop.missingRuntimes(targetSlug).catch(() => []);
    if (missing.length > 0) {
      setNeeds(missing);
      return;
    }
    const status = sandboxStatus.data;
    if (status?.available && sandbox?.suggested && !decided(status, targetSlug)) setAskSandbox(true);
    else launch();
  };
  const installNeeds = () =>
    void run("install", async () => {
      await desktop.installRuntimes(targetSlug);
      setNeeds(undefined);
      await play();
    });
  const pick = async (next: string) => {
    setError(undefined);
    setOfferSaves(undefined);
    const from = target;
    const to = next === slug ? installed : builds.find((b) => b.slug === next);
    try {
      await desktop.setPlayChoice(slug, next === slug ? null : next);
      setChoice(next === slug ? undefined : next);
      setLaunched(undefined);
      // Each build has its own save folder: offer to bring saves along into an empty one.
      if (from && to && from.slug !== to.slug) {
        const [had, has] = await Promise.all([desktop.saveSize(from.slug), desktop.saveSize(to.slug)]);
        if (had > 0 && has === 0) setOfferSaves({ from, to });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
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
            <div className="play-split">
              <button className="btn big slime" disabled={busy !== null} onClick={() => void play()}>
                {busy === "launch" ? "Starting…" : "▶ Play"}
              </button>
              {builds.length > 0 && (
                <select
                  className="play-pick"
                  aria-label={`Which version of ${title} Play starts`}
                  value={targetSlug}
                  disabled={busy !== null}
                  onChange={(e) => void pick(e.target.value)}
                >
                  <option value={slug}>{title} (original)</option>
                  {builds.map((b) => (
                    <option key={b.slug} value={b.slug}>
                      {b.title} (community, by {b.maker})
                    </option>
                  ))}
                </select>
              )}
            </div>
          )}
          {needs && (
            <div className="notice small" role="dialog" aria-label="Runtimes needed">
              <p>
                <b>{targetTitle}</b> needs {needs.map((n) => n.name).join(" and ")}, which this computer doesn't have yet.
                Slop downloads {needs.length === 1 ? "it" : "them"} from {[...new Set(needs.map((n) => n.vendor))].join(" and ")}{" "}
                and only runs {needs.length === 1 ? "it" : "them"} if Windows confirms the signature. Windows asks you to
                allow the install.
              </p>
              <div className="actions">
                <button className="btn small ghost" disabled={busy !== null} onClick={() => setNeeds(undefined)}>
                  Not now
                </button>
                <button className="btn small slime" disabled={busy !== null} onClick={installNeeds}>
                  {busy === "install" ? "Installing…" : "Install and play"}
                </button>
              </div>
            </div>
          )}
          {targetSlug !== slug && (
            <p className="muted small">
              Plays <b>{targetTitle}</b>, a community build by {target?.maker}. It has its own saves.
            </p>
          )}
          {offerSaves && (
            <div className="notice small">
              <p>
                {offerSaves.to.title} has no saves yet. Copy your saves from {offerSaves.from.title}? A fork may store saves
                differently; yours for {offerSaves.from.title} stay as they are.
              </p>
              <div className="actions">
                <button className="btn small ghost" onClick={() => setOfferSaves(undefined)}>
                  No thanks
                </button>
                <button
                  className="btn small slime"
                  onClick={() =>
                    void desktop.copySaves(offerSaves.from.slug, offerSaves.to.slug).then(
                      () => setOfferSaves(undefined),
                      (err: unknown) => setError(err instanceof Error ? err.message : String(err)),
                    )
                  }
                >
                  Copy my saves
                </button>
              </div>
            </div>
          )}
          {askSandbox && (
            <SandboxPrompt
              title={targetTitle}
              onChoose={(on) => {
                setAskSandbox(false);
                void desktop.setSandbox(targetSlug, on).then(() => {
                  sandboxStatus.reload();
                  launch();
                });
              }}
            />
          )}
          {installed.kind !== "installer" && sandboxStatus.data && (
            <SandboxSwitch slug={targetSlug} status={sandboxStatus.data} hint={sandbox} onChanged={sandboxStatus.reload} />
          )}
          {launched && <LaunchNotices launched={launched} slug={target?.build_of ?? slug} title={targetTitle} />}
          {failed && (
            <SandboxFailedNotice
              slug={targetSlug}
              title={targetTitle}
              onRunNormally={() => launch(true)}
              onTurnOff={() => {
                clearFailed();
                void desktop.setSandbox(targetSlug, false).then(sandboxStatus.reload);
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
