// SPDX-License-Identifier: AGPL-3.0-or-later
// Install, launch and library buttons for one app.
//
// Desktop: the Rust side downloads the maker's file, checks it against SLAPorSLOP's signed manifest,
// and installs it into the library folder (docs/ARCHITECTURE.md §6.3). The website can only add
// apps to the library; installing happens in the desktop app.

import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Link, useLocation } from "react-router";

import { api } from "../api";
import { useLoad } from "../hooks";
import { LIBRARY_CHANGED } from "./AppUpdates";
import { desktop, isDesktop, type InstalledApp, type Launched } from "../platform";
import {
  decided,
  isOn,
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

/** What the library knows about an installed app, for its action bar (DESIGN §6). */
export interface BarInfo {
  /** An approved release newer than the installed one, and the version. */
  available?: boolean;
  latest?: string | null;
  /** A newer release SLAPorSLOP is still checking. */
  pending?: { version: string; status: "fetching" | "awaiting_review" } | null;
  /** "Update apps automatically" is on, so an available update is queued. */
  autoUpdate?: boolean;
  /** SLAPorSLOP started it and it's still running. */
  running?: boolean;
  /** Icon buttons for the bar's right end (the library's info button). */
  extra?: ReactNode;
  /** Opens the app's saves (the library's Saves tab). */
  onShowSaves?: () => void;
}

export function InstallControls({
  slug,
  title,
  inLibrary,
  sandbox,
  bar,
  donate,
}: {
  slug: string;
  title: string;
  inLibrary?: boolean;
  /** For suggesting sandbox mode (DESIGN §6). */
  sandbox?: SandboxHint;
  /** The library's Steam-like action bar: one big button, a status, and icon buttons. */
  bar?: BarInfo;
  /** The Donate button (DonateButton), in the row with Play, Install or Download. */
  donate?: ReactNode;
}) {
  return isDesktop ? (
    <DesktopControls slug={slug} title={title} sandbox={sandbox} bar={bar} donate={donate} />
  ) : (
    <WebControls slug={slug} inLibrary={inLibrary} bar={bar} donate={donate} />
  );
}

/** A gear, for settings buttons (drawn, so it looks the same on every system). */
export const GEAR_ICON = (
  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);

/** An "i" in a circle, for information buttons. */
export const INFO_ICON = (
  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="10" />
    <path d="M12 16v-5M12 8h.01" />
  </svg>
);

/** An icon button that opens a small panel under it; Escape or a click outside closes it. */
export function Popover({
  label,
  icon,
  children,
}: {
  label: string;
  icon: ReactNode;
  children: ReactNode | ((close: () => void) => ReactNode);
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [open]);
  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  return (
    <div
      className="lib-pop"
      ref={box}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          close();
        }
      }}
    >
      <button
        type="button"
        ref={button}
        className={`btn lib-icon-btn${open ? " on" : ""}`}
        aria-label={label}
        title={label}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <span aria-hidden="true">{icon}</span>
      </button>
      {open && (
        <div className="lib-pop-panel panel" id={id} role="group" aria-label={label}>
          {typeof children === "function" ? children(close) : children}
        </div>
      )}
    </div>
  );
}

function WebControls({ slug, inLibrary, bar, donate }: { slug: string; inLibrary?: boolean; bar?: BarInfo; donate?: ReactNode }) {
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

  if (bar) {
    return (
      <>
        <div className="lib-bar">
          <Link className="btn big slime lib-primary" to="/download">
            ⬇ Get the SLAPorSLOP app to play
          </Link>
          {donate}
          <div className="lib-bar-status">
            <span className="lib-bar-label">{added ? "In your library" : "Not in your library"}</span>
            <span className="muted small">Install it with the SLAPorSLOP desktop app. It checks every file against SLAPorSLOP's signature first.</span>
          </div>
          <div className="lib-bar-icons">
            {me ? (
              <button className="btn" onClick={() => void toggle()} aria-pressed={added}>
                {added ? "✓ In your library" : "+ Add to library"}
              </button>
            ) : (
              <Link className="btn" to={`/signin?return_to=${encodeURIComponent(location.pathname)}`}>
                Sign in to add to your library
              </Link>
            )}
            {bar.extra}
          </div>
        </div>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </>
    );
  }

  return (
    <div className="install-controls">
      <div className="get-row">
        {me ? (
          <button className={`btn ${added ? "" : "slime"}`} onClick={() => void toggle()} aria-pressed={added}>
            {added ? "✓ In your library" : "+ Add to library"}
          </button>
        ) : (
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(location.pathname)}`}>
            Sign in to add to your library
          </Link>
        )}
        {donate}
      </div>
      <p className="muted small">Install it with the SLAPorSLOP desktop app. It checks every file against SLAPorSLOP's signature first.</p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function DesktopControls({
  slug,
  title,
  sandbox: given,
  bar,
  donate,
}: {
  slug: string;
  title: string;
  sandbox?: SandboxHint;
  bar?: BarInfo;
  donate?: ReactNode;
}) {
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

  const needsNotice = needs && (
    <div className="notice small" role="dialog" aria-label="Runtimes needed">
      <p>
        <b>{targetTitle}</b> needs {needs.map((n) => n.name).join(" and ")}, which this computer doesn't have yet. SLAPorSLOP
        downloads {needs.length === 1 ? "it" : "them"} from {[...new Set(needs.map((n) => n.vendor))].join(" and ")} and
        only runs {needs.length === 1 ? "it" : "them"} if Windows confirms the signature. Windows asks you to allow the
        install.
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
  );
  const buildNote = targetSlug !== slug && (
    <p className="muted small">
      Plays <b>{targetTitle}</b>, a community build by {target?.maker}. It has its own saves.
    </p>
  );
  const offerSavesNotice = offerSaves && (
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
  );
  const sandboxPrompt = askSandbox && (
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
  );
  const sandboxSwitch = installed?.kind !== "installer" && sandboxStatus.data && (
    <SandboxSwitch slug={targetSlug} status={sandboxStatus.data} hint={sandbox} onChanged={sandboxStatus.reload} />
  );
  const launchNotices = launched && <LaunchNotices launched={launched} slug={target?.build_of ?? slug} title={targetTitle} />;
  const failedNotice = failed && (
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
  );
  const uninstallConfirm = confirmUninstall && (
    <div className="notice">
      <p>
        Uninstall <b>{title}</b>? Its mods go too. Your saves stay, so reinstalling picks up where you left off.
      </p>
      <label className="check">
        <input type="checkbox" checked={deleteSaves} onChange={(e) => setDeleteSaves(e.target.checked)} /> Delete my saves
        too
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
  );
  const errorNote = error && (
    <p className="form-error" role="alert">
      {error}
    </p>
  );
  const playPick = builds.length > 0 && (
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
  );
  const installerConfirm = (
    <div className="notice">
      <p>
        <b>{title}</b> runs its own installer, which may ask for admin rights. SLAPorSLOP checked that it's the exact file the maker
        submitted and SLAPorSLOP approved.
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
  );

  if (bar) {
    // Steam's action bar: the one thing to do now, what's happening, and the rest behind icons.
    const install = () => void run("install", () => desktop.install(slug));
    const downloading = installing || Boolean(busy === null && progress && progress.got < progress.total);
    const bytes = progress && progress.total ? `${formatBytes(progress.got)} of ${formatBytes(progress.total)}` : null;
    let primary: ReactNode;
    let secondary: ReactNode = null;
    let label: string;
    let detail: ReactNode;
    if (!installed) {
      primary = (
        <button className="btn big slime lib-primary" disabled={busy !== null || !build.data} onClick={install}>
          {downloading ? (progress ? `Downloading ${pct}%` : "Starting download…") : "Install"}
        </button>
      );
      label = downloading ? "Downloading" : build.data ? "Not installed" : "Not available yet";
      detail = downloading
        ? `${pct}% complete${bytes ? ` · ${bytes}` : ""}, from the maker's release page`
        : build.data
          ? "Downloaded from the maker's release page and checked against SLAPorSLOP's signature."
          : "No approved build yet.";
    } else if (downloading) {
      primary = (
        <button className="btn big slime lib-primary" disabled>
          {progress ? `Downloading ${pct}%` : "Starting download…"}
        </button>
      );
      label = bar.available ? "Updating" : "Downloading";
      detail = `${pct}% complete${bytes ? ` · ${bytes}` : ""}`;
    } else if (installed.kind === "installer") {
      primary = (
        <button className="btn big slime lib-primary" onClick={() => setConfirmInstaller(true)}>
          Run installer…
        </button>
      );
      label = bar.available ? "Update ready" : "Installed";
      detail = bar.available ? `Version ${installed.version} → ${bar.latest ?? "newer"}` : `Version ${installed.version}`;
      if (bar.available)
        secondary = (
          <button className="btn" disabled={busy !== null} onClick={install}>
            Update
          </button>
        );
    } else if (bar.available) {
      primary = (
        <button className="btn big slime lib-primary" disabled={busy !== null} onClick={install}>
          Update
        </button>
      );
      secondary = (
        <button className="btn" disabled={busy !== null} onClick={() => void play()}>
          {busy === "launch" ? "Starting…" : `▶ Play ${installed.version}`}
        </button>
      );
      label = bar.autoUpdate ? "Update queued" : "Update ready";
      detail = `0% complete · version ${installed.version} → ${bar.latest ?? "newer"}`;
    } else if (bar.running) {
      primary = (
        <button className="btn big lib-primary" disabled>
          Running
        </button>
      );
      label = "Running";
      detail = `${targetTitle} is open. Updates wait until it closes.`;
    } else {
      primary = (
        <button className="btn big slime lib-primary" disabled={busy !== null} onClick={() => void play()}>
          {busy === "launch" ? "Starting…" : "▶ Play"}
        </button>
      );
      label = bar.pending ? (bar.pending.status === "fetching" ? "Scanning update" : "Update in review") : "Installed";
      detail = bar.pending
        ? `Version ${installed.version}; ${bar.pending.version} is ${
            bar.pending.status === "fetching" ? "being downloaded and scanned by SLAPorSLOP" : "waiting for a person to review it"
          }`
        : `Version ${installed.version}`;
    }
    const sandboxOn = Boolean(sandboxStatus.data?.available && isOn(sandboxStatus.data, targetSlug));
    return (
      <div className="lib-controls">
        <div className="lib-bar">
          {primary}
          {secondary}
          {donate}
          <div className="lib-bar-status">
            <span className="lib-bar-label">{label}</span>
            <span className="muted small">{detail}</span>
            {downloading && progress && (
              <progress max={progress.total} value={progress.got} aria-label="Download progress" />
            )}
          </div>
          {installed && installed.kind !== "installer" && playPick && <div className="lib-bar-pick">{playPick}</div>}
          <div className="lib-bar-icons">
            {installed && (
              <Popover label="Settings for this app" icon={GEAR_ICON}>
                {(close) => (
                  <div className="lib-menu">
                    {sandboxSwitch}
                    {sandboxOn && (
                      <button
                        type="button"
                        className="btn small"
                        disabled={busy !== null}
                        onClick={() => {
                          close();
                          launch(true);
                        }}
                      >
                        Run normally this time
                      </button>
                    )}
                    {bar.onShowSaves && installed.kind !== "installer" && (
                      <button
                        type="button"
                        className="btn small"
                        onClick={() => {
                          close();
                          bar.onShowSaves?.();
                        }}
                      >
                        Saves…
                      </button>
                    )}
                    <button
                      type="button"
                      className="btn small"
                      disabled={busy !== null || !build.data}
                      onClick={() => {
                        close();
                        install();
                      }}
                    >
                      Reinstall / update
                    </button>
                    <button
                      type="button"
                      className="btn small ghost danger"
                      disabled={busy !== null}
                      onClick={() => {
                        close();
                        setConfirmUninstall(true);
                      }}
                    >
                      Uninstall…
                    </button>
                    <p className="muted small">Version {installed.version} is installed.</p>
                  </div>
                )}
              </Popover>
            )}
            {bar.extra}
          </div>
        </div>
        {installed?.kind === "installer" && confirmInstaller && installerConfirm}
        {needsNotice}
        {installed && buildNote}
        {offerSavesNotice}
        {sandboxPrompt}
        {launchNotices}
        {failedNotice}
        {uninstallConfirm}
        {errorNote}
      </div>
    );
  }

  return (
    <div className="install-controls">
      {installed ? (
        <>
          {installed.kind === "installer" && confirmInstaller ? (
            installerConfirm
          ) : (
            <div className="get-row">
              {installed.kind === "installer" ? (
                <button className="btn big slime" onClick={() => setConfirmInstaller(true)}>
                  Run installer…
                </button>
              ) : (
                <div className="play-split">
                  <button className="btn big slime" disabled={busy !== null} onClick={() => void play()}>
                    {busy === "launch" ? "Starting…" : "▶ Play"}
                  </button>
                  {playPick}
                </div>
              )}
              {donate}
            </div>
          )}
          {needsNotice}
          {buildNote}
          {offerSavesNotice}
          {sandboxPrompt}
          {sandboxSwitch}
          {launchNotices}
          {failedNotice}
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
          {uninstallConfirm}
        </>
      ) : build.data ? (
        <>
          <div className="get-row">
            <button className="btn big slime" disabled={busy !== null} onClick={() => void run("install", () => desktop.install(slug))}>
              {installing ? (progress ? `Installing… ${pct}%` : "Starting download…") : "Install"}
            </button>
            {donate}
          </div>
          {installing && progress && (
            <>
              <progress max={progress.total} value={progress.got} aria-label="Download progress" />
              <p className="muted small">
                {formatBytes(progress.got)} of {formatBytes(progress.total)}, from the maker's release page
              </p>
            </>
          )}
          {!installing && (
            <p className="muted small">Downloaded from the maker's release page and checked against SLAPorSLOP's signature.</p>
          )}
        </>
      ) : (
        <>
          <p className="muted">No approved build yet.</p>
          {donate && <div className="get-row">{donate}</div>}
        </>
      )}
      {errorNote}
    </div>
  );
}
