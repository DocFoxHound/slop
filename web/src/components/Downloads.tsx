// SPDX-License-Identifier: AGPL-3.0-or-later
// Downloads in the Slop app (DESIGN §6), like Steam's "Downloads – 4 of 4 items complete": what
// is downloading now, the updates waiting to install, and what finished this session. A small
// status button opens the list. It stands alone (it loads what it needs when not given it), so
// it can sit in the library's sidebar or, later, in the bottom bar beside Friends & Chat.

import { useEffect, useId, useRef, useState, useSyncExternalStore } from "react";

import { useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { checkUpdates, installUpdates, LIBRARY_CHANGED, type InstalledWithUpdate } from "./AppUpdates";

export interface Download {
  slug: string;
  got: number;
  total: number;
  done: boolean;
  /** When the last progress came, so a download that stopped (failed) isn't shown forever. */
  at: number;
}

/** A download with no news for this long is no longer shown as downloading. */
const STALLED_MS = 60_000;

let current: Download[] = [];
let listening = false;
const listeners = new Set<() => void>();

function listen() {
  if (listening || !isDesktop) return;
  listening = true;
  void desktop
    .onProgress((p) => {
      const next: Download = { slug: p.slug, got: p.got, total: p.total, done: p.total > 0 && p.got >= p.total, at: Date.now() };
      current = [...current.filter((d) => d.slug !== p.slug), next];
      listeners.forEach((l) => l());
    })
    .catch(() => {
      listening = false;
    });
}

function subscribe(listener: () => void) {
  listen();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Every download this session (the Slop app's install-progress events), oldest first. */
export function useDownloads(): Download[] {
  return useSyncExternalStore(subscribe, () => current);
}

/** Downloads still going: slug → percent. */
export function downloading(list: Download[], now = Date.now()): Record<string, number> {
  const out: Record<string, number> = {};
  for (const d of list) {
    if (!d.done && now - d.at < STALLED_MS) out[d.slug] = d.total ? Math.floor((d.got / d.total) * 100) : 0;
  }
  return out;
}

/** "Downloads – 2 of 3 complete": finished this session, of those plus going and queued. */
export function downloadsLabel(done: number, active: number, queued: number): string {
  const total = done + active + queued;
  return total === 0 ? "Downloads" : `Downloads – ${done} of ${total} complete`;
}

export function DownloadsStatus({
  apps: given,
  titles = {},
  onChanged,
}: {
  /** Installed apps and their updates, when the page already has them. */
  apps?: InstalledWithUpdate[];
  /** Names for apps being installed for the first time. */
  titles?: Record<string, string>;
  onChanged?: () => void;
}) {
  const list = useDownloads();
  const own = useLoad(() => (given || !isDesktop ? Promise.resolve(undefined) : checkUpdates()), [Boolean(given)]);
  const reloadOwn = own.reload;
  useEffect(() => {
    if (given) return;
    window.addEventListener(LIBRARY_CHANGED, reloadOwn);
    return () => window.removeEventListener(LIBRARY_CHANGED, reloadOwn);
  }, [given, reloadOwn]);
  const apps = given ?? own.data ?? [];
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const panelId = useId();
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
  if (!isDesktop) return null;

  const going = downloading(list);
  const title = (slug: string) => apps.find((a) => a.installed.slug === slug)?.installed.title ?? titles[slug] ?? slug;
  const done = list.filter((d) => d.done);
  const active = Object.keys(going);
  const queued = apps.filter((a) => a.available && !(a.installed.slug in going) && !done.some((d) => d.slug === a.installed.slug));
  const checking = apps.filter((a) => !a.available && a.update?.pending);
  const label = downloadsLabel(done.length, active.length, queued.length);

  return (
    <div
      className="downloads"
      ref={box}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          setOpen(false);
          button.current?.focus();
        }
      }}
    >
      {open && (
        <div className="downloads-panel panel" id={panelId} role="group" aria-label="Downloads">
          <h2>Downloads</h2>
          {active.length + queued.length + done.length + checking.length === 0 && (
            <p className="muted small">Nothing downloading. Updates for your apps show here.</p>
          )}
          <ul className="plain">
            {active.map((slug) => (
              <li key={slug}>
                <b>{title(slug)}</b> <span className="slime small">Downloading {going[slug]}%</span>
                <progress max={100} value={going[slug]} aria-label={`${title(slug)} download progress`} />
              </li>
            ))}
            {queued.map((a) => (
              <li key={a.installed.slug}>
                <b>{a.installed.title}</b>{" "}
                <span className="muted small">
                  Update queued: {a.installed.version} → {a.update?.latest?.version}
                  {a.installed.kind === "installer" && " (runs its installer: update it from its page)"}
                </span>
              </li>
            ))}
            {checking.map((a) => (
              <li key={a.installed.slug}>
                <b>{a.installed.title}</b>{" "}
                <span className="muted small">
                  {a.update?.pending?.version}{" "}
                  {a.update?.pending?.status === "fetching" ? "is being downloaded and scanned by Slop" : "is waiting for a person to review it"}
                </span>
              </li>
            ))}
            {done.map((d) => (
              <li key={d.slug}>
                <b>{title(d.slug)}</b> <span className="muted small">Complete</span>
              </li>
            ))}
          </ul>
          {queued.length > 0 && (
            <button
              type="button"
              className="btn small slime"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                void installUpdates().finally(() => {
                  setBusy(false);
                  onChanged?.();
                  if (!given) reloadOwn();
                });
              }}
            >
              {busy ? "Updating…" : `Update ${queued.length === 1 ? "1 app" : `${queued.length} apps`} now`}
            </button>
          )}
        </div>
      )}
      <button
        type="button"
        ref={button}
        className={`downloads-bar${open ? " on" : ""}`}
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <span aria-hidden="true">⬇</span> {label}
      </button>
    </div>
  );
}
