// SPDX-License-Identifier: AGPL-3.0-or-later
// Updates for installed apps (DESIGN §6). Slop checks when the desktop app starts and every 30
// minutes; with "Update apps automatically" on, it installs approved updates for apps that
// aren't running, one at a time. The library page shows what's waiting and what's being checked.

import { useEffect } from "react";

import { api, type AppUpdate } from "../api";
import { desktop, isDesktop, type InstalledApp } from "../platform";

const EVERY_MS = 30 * 60 * 1000;
/** Fired on window when installed apps changed, so the library can reload. */
export const LIBRARY_CHANGED = "slop-library-changed";

export interface InstalledWithUpdate {
  installed: InstalledApp;
  update?: AppUpdate;
  /** An approved release newer than the installed one. */
  available: boolean;
}

export async function checkUpdates(): Promise<InstalledWithUpdate[]> {
  const installed = await desktop.installed();
  if (installed.length === 0) return [];
  const updates = await api.libraryUpdates(installed.map((a) => a.slug)).catch(() => [] as AppUpdate[]);
  return installed.map((app) => {
    const update = updates.find((u) => u.slug === app.slug);
    return { installed: app, update, available: Boolean(update?.latest && update.latest.release_id !== app.release_id) };
  });
}

let running = false;

/** Installs every available update for apps that aren't running. Returns how many it installed. */
export async function installUpdates(): Promise<number> {
  if (running) return 0;
  running = true;
  let done = 0;
  try {
    const [apps, busy] = await Promise.all([checkUpdates(), desktop.running()]);
    for (const a of apps) {
      // Installers run the maker's own setup, so they're only updated when the player asks.
      if (!a.available || busy.includes(a.installed.slug) || a.installed.kind === "installer") continue;
      try {
        await desktop.install(a.installed.slug);
        done += 1;
      } catch {
        // Tried again next time (e.g. the maker's file was briefly unavailable).
      }
    }
  } finally {
    running = false;
    if (done > 0) window.dispatchEvent(new Event(LIBRARY_CHANGED));
  }
  return done;
}

/** In the desktop app's layout: checks now and every 30 minutes, if the player wants it. */
export function useAutoUpdates() {
  useEffect(() => {
    if (!isDesktop) return;
    const tick = () =>
      void desktop
        .autoUpdateSetting()
        .then((on) => (on ? installUpdates() : 0))
        .catch(() => 0);
    const first = window.setTimeout(tick, 20_000);
    const every = window.setInterval(tick, EVERY_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(every);
    };
  }, []);
}
