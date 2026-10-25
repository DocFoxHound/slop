// SPDX-License-Identifier: AGPL-3.0-or-later
// What the library's sidebar lists (DESIGN §6): every app once, in groups like Steam's
// (installed, not installed, Private Slop, Interested), with a filter, a search and a short
// status for each ("Update queued", "Downloading 40%", "Running"). Pure, so it's unit tested.

import type { AppSummary, AppUpdate } from "../api";
import type { InstalledApp } from "../platform";

export type Group = "installed" | "library" | "private" | "interested";
export type Filter = "all" | "installed" | "updates" | "private" | "interested" | "community";

/** One app in the library, from whichever lists it's in. */
export interface Entry {
  slug: string;
  title: string;
  group: Group;
  /** What the store knows about it, when it does (not for an installed app Slop no longer lists). */
  app?: AppSummary;
  installed?: InstalledApp;
  update?: AppUpdate;
  /** An approved release newer than the installed one. */
  available: boolean;
  /** A community build installed beside its original: the original's title (DESIGN §9). */
  buildOf?: string;
  /** Private Slop: "you", or who shared it. */
  owner?: string;
  inLibrary: boolean;
  /** Marked Interested (an upcoming app, or one released since). */
  interested: boolean;
}

export interface Sources {
  library: AppSummary[];
  installed: Array<{ installed: InstalledApp; update?: AppUpdate; available: boolean }>;
  interested: AppSummary[];
  privateMine: AppSummary[];
  privateShared: Array<AppSummary & { owner: string }>;
}

const byTitle = (a: { title: string }, b: { title: string }) =>
  a.title.localeCompare(b.title, undefined, { sensitivity: "base" });

/** Every app once: installed first (community builds under their original), then the rest. */
export function buildEntries(s: Sources): Entry[] {
  const out: Entry[] = [];
  const seen = new Set<string>();
  const inLibrary = new Set(s.library.map((a) => a.slug));
  const interested = new Set(s.interested.map((a) => a.slug));
  const privates = [
    ...s.privateMine.map((a) => ({ ...a, owner: "you" })),
    ...s.privateShared,
  ];
  const known = (slug: string): AppSummary | undefined =>
    s.library.find((a) => a.slug === slug) ??
    privates.find((a) => a.slug === slug) ??
    s.interested.find((a) => a.slug === slug);

  const all = s.installed;
  const originalOf = (app: InstalledApp) => all.find((o) => o.installed.slug === app.build_of)?.installed;
  const originals = all.filter(({ installed }) => !originalOf(installed)).sort((a, b) => byTitle(a.installed, b.installed));
  for (const entry of originals) {
    const builds = all
      .filter(({ installed }) => installed.build_of === entry.installed.slug && installed.slug !== entry.installed.slug)
      .sort((a, b) => byTitle(a.installed, b.installed));
    for (const { installed, update, available } of [entry, ...builds]) {
      if (seen.has(installed.slug)) continue;
      seen.add(installed.slug);
      const original = originalOf(installed);
      out.push({
        slug: installed.slug,
        title: installed.title,
        group: "installed",
        app: known(installed.slug),
        installed,
        update,
        available,
        buildOf: original?.title,
        owner: privates.find((p) => p.slug === installed.slug)?.owner,
        inLibrary: inLibrary.has(installed.slug),
        interested: interested.has(installed.slug),
      });
    }
  }
  const rest = (list: AppSummary[], group: Group) => {
    for (const app of [...list].sort(byTitle)) {
      if (seen.has(app.slug)) continue;
      seen.add(app.slug);
      out.push({
        slug: app.slug,
        title: app.title,
        group,
        app,
        available: false,
        owner: (app as { owner?: string }).owner,
        inLibrary: inLibrary.has(app.slug),
        interested: interested.has(app.slug),
      });
    }
  };
  rest(privates, "private");
  rest(s.library, "library");
  rest(s.interested, "interested");
  return out;
}

export interface Live {
  /** Downloads in progress: slug → percent. */
  downloading: Record<string, number>;
  running: string[];
  autoUpdate: boolean;
}

/** The short status after an app's name, or null when there's nothing to say. */
export function statusOf(e: Entry, live: Live): string | null {
  const pct = live.downloading[e.slug];
  if (pct !== undefined) return `Downloading ${pct}%`;
  if (live.running.includes(e.slug)) return "Running";
  if (e.installed) {
    if (e.available) return live.autoUpdate && e.installed.kind !== "installer" ? "Update queued" : "Update ready";
    if (e.update?.pending?.status === "fetching") return "Scanning update";
    if (e.update?.pending?.status === "awaiting_review") return "Update in review";
    return null;
  }
  if (e.group === "interested" && e.app?.stage === "released") return "Released";
  return null;
}

export function matchesFilter(e: Entry, filter: Filter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "installed":
      return e.group === "installed";
    case "updates":
      return e.available || Boolean(e.installed && e.update?.pending);
    case "private":
      return Boolean(e.owner);
    case "interested":
      return e.interested;
    case "community":
      return Boolean(e.buildOf || e.installed?.build_of || e.app?.branch_of);
  }
}

export function matchesSearch(e: Entry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return e.title.toLowerCase().includes(q) || e.slug.includes(q) || (e.app?.summary.toLowerCase().includes(q) ?? false);
}

/** The filters that make sense here: installing happens in the Slop app only. */
export function filtersFor(desktop: boolean): Array<{ id: Filter; label: string }> {
  return [
    { id: "all", label: "All" },
    ...(desktop
      ? ([
          { id: "installed", label: "Installed" },
          { id: "updates", label: "Updates" },
        ] as const)
      : []),
    { id: "private", label: "Private Slop" },
    { id: "interested", label: "Interested" },
    { id: "community", label: "Community builds" },
  ];
}

export function groupLabel(group: Group, desktop: boolean): string {
  switch (group) {
    case "installed":
      return "Installed";
    case "library":
      return desktop ? "Not installed" : "In your library";
    case "private":
      return "Private Slop";
    case "interested":
      return "Interested";
  }
}

export interface Section {
  group: Group;
  label: string;
  entries: Entry[];
  /** How many the group has without the filter and search. */
  total: number;
}

const ORDER: Group[] = ["installed", "library", "private", "interested"];

/** The groups to show, in order, with what matches; a group with no matches is left out. */
export function sections(all: Entry[], filter: Filter, query: string, desktop: boolean): Section[] {
  return ORDER.map((group) => {
    const members = all.filter((e) => e.group === group);
    return {
      group,
      label: groupLabel(group, desktop),
      entries: members.filter((e) => matchesFilter(e, filter) && matchesSearch(e, query)),
      total: members.length,
    };
  }).filter((s) => s.entries.length > 0);
}

/** "(3)", or "(1/3)" while a filter or search hides some, like Steam's "(23/245)". */
export function countLabel(s: Pick<Section, "entries" | "total">): string {
  return s.entries.length === s.total ? `(${s.total})` : `(${s.entries.length}/${s.total})`;
}
