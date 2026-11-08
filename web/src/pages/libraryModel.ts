// SPDX-License-Identifier: AGPL-3.0-or-later
// What the library's sidebar lists (DESIGN §6): every app once, in groups like Steam's
// (installed, plays in your browser, not installed, Private, Interested), with a filter (by kind,
// or by store category), a search and a short status for each ("Update queued", "Downloading 40%",
// "Running"). Pure, so it's unit tested.

import type { AppSummary, AppUpdate, Category } from "../api";
import type { InstalledApp } from "../platform";

export type Group = "installed" | "browser" | "library" | "private" | "interested";
/** One store category's apps, by its slug. */
export type CategoryFilter = `category:${string}`;
export type Filter = "all" | "installed" | "updates" | "browser" | "private" | "interested" | "community" | CategoryFilter;

/** One app in the library, from whichever lists it's in. */
export interface Entry {
  slug: string;
  title: string;
  group: Group;
  /** What the store knows about it, when it does: not for an installed app SLAPorSLOP no longer
   *  lists, nor while an installed app the account library doesn't have is still being looked up. */
  app?: AppSummary;
  installed?: InstalledApp;
  update?: AppUpdate;
  /** An approved release newer than the installed one. */
  available: boolean;
  /** A community build installed beside its original: the original's title (DESIGN §9). */
  buildOf?: string;
  /** A dedicated server, listed under its game when the game is listed too: the game's title. */
  serverFor?: string;
  /** Private: "you", or who shared it. */
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
  /** The store's data for installed apps none of the lists above has (installed signed out, or
   *  taken out of the library on the website): `unknownInstalled` says which to look up. */
  store?: AppSummary[];
}

/** Installed apps none of the account's lists knows: their store data is looked up on its own,
 *  so they're in their categories too. */
export function unknownInstalled(s: Omit<Sources, "store">): string[] {
  const known = new Set([...s.library, ...s.interested, ...s.privateMine, ...s.privateShared].map((a) => a.slug));
  return s.installed.map((i) => i.installed.slug).filter((slug) => !known.has(slug));
}

const byTitle = (a: { title: string }, b: { title: string }) =>
  a.title.localeCompare(b.title, undefined, { sensitivity: "base" });

/** Every app once: installed first (community builds and dedicated servers under their game),
 *  then the rest (servers under their game there too). */
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
    s.interested.find((a) => a.slug === slug) ??
    s.store?.find((a) => a.slug === slug);

  const all = s.installed;
  const originalOf = (app: InstalledApp) => all.find((o) => o.installed.slug === app.build_of)?.installed;
  // A dedicated server's game, when that's installed too (servers aren't linked in the manifest,
  // so the store's data says which game it's for).
  const gameOf = (app: InstalledApp) => {
    const game = known(app.slug)?.server_for;
    return game ? all.find((o) => o.installed.slug === game)?.installed : undefined;
  };
  const originals = all
    .filter(({ installed }) => !originalOf(installed) && !gameOf(installed))
    .sort((a, b) => byTitle(a.installed, b.installed));
  for (const entry of originals) {
    const builds = all
      .filter(({ installed }) => installed.build_of === entry.installed.slug && installed.slug !== entry.installed.slug)
      .sort((a, b) => byTitle(a.installed, b.installed));
    const servers = all
      .filter(({ installed }) => gameOf(installed)?.slug === entry.installed.slug)
      .sort((a, b) => byTitle(a.installed, b.installed));
    for (const { installed, update, available } of [entry, ...builds, ...servers]) {
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
        serverFor: gameOf(installed)?.title,
        owner: privates.find((p) => p.slug === installed.slug)?.owner,
        inLibrary: inLibrary.has(installed.slug),
        interested: interested.has(installed.slug),
      });
    }
  }
  const rest = (list: AppSummary[], group: Group) => {
    const sorted = [...list].sort(byTitle);
    // A server whose game is in the same list goes right under it.
    const gameIn = (app: AppSummary) => (app.server_for ? sorted.find((g) => g.slug === app.server_for) : undefined);
    const push = (app: AppSummary, game?: AppSummary) => {
      if (seen.has(app.slug)) return;
      seen.add(app.slug);
      out.push({
        slug: app.slug,
        title: app.title,
        // Nothing to install: a browser game in the library is ready to play, here or anywhere.
        group: group === "library" && app.plays_in_browser ? "browser" : group,
        app,
        available: false,
        serverFor: game?.title,
        owner: (app as { owner?: string }).owner,
        inLibrary: inLibrary.has(app.slug),
        interested: interested.has(app.slug),
      });
    };
    for (const app of sorted) {
      if (gameIn(app)) continue;
      push(app);
      for (const server of sorted.filter((a) => a.server_for === app.slug)) push(server, app);
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

/** The filter for one store category. */
export const categoryFilter = (slug: string): CategoryFilter => `category:${slug}`;

function isCategoryFilter(filter: Filter): filter is CategoryFilter {
  return filter.startsWith("category:");
}

/** In a store category: one of the entry's app's tags is one of the category's, and the category
 *  takes it as the store does (catalog.rs `push_category_limits`: Kids & Family lists only apps
 *  rated at most its age, without its excluded labels). An entry without store data (an installed
 *  app SLAPorSLOP no longer lists) is in none. */
export function inCategory(e: Entry, category: Pick<Category, "tags" | "max_age" | "excluded_labels">): boolean {
  const app = e.app;
  if (!app) return false;
  if ((app.min_age ?? 0) > (category.max_age ?? 18)) return false;
  if ((app.content_labels ?? []).some((l) => (category.excluded_labels ?? []).includes(l))) return false;
  return category.tags.some((t) => app.tags.includes(t.slug));
}

/** Whether the entry is shown under `filter`; a category's needs the store's categories
 *  (api.categories()), and matches nothing without them. */
export function matchesFilter(e: Entry, filter: Filter, categories: readonly Category[] = []): boolean {
  if (isCategoryFilter(filter)) {
    const category = categories.find((c) => categoryFilter(c.slug) === filter);
    return Boolean(category && inCategory(e, category));
  }
  switch (filter) {
    case "all":
      return true;
    case "installed":
      return e.group === "installed";
    case "updates":
      return e.available || Boolean(e.installed && e.update?.pending);
    case "browser":
      return Boolean(e.app?.plays_in_browser);
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

/** The filters that make sense here: installing happens in the SLAPorSLOP app only. Games that play
 * in the browser play from both. */
export function filtersFor(desktop: boolean): Array<{ id: Filter; label: string }> {
  return [
    { id: "all", label: "All" },
    ...(desktop
      ? ([
          { id: "installed", label: "Installed" },
          { id: "updates", label: "Updates" },
        ] as const)
      : []),
    { id: "browser", label: "Plays in your browser" },
    { id: "private", label: "Private" },
    { id: "interested", label: "Interested" },
    { id: "community", label: "Community builds" },
  ];
}

/** The Show filter's Categories: every store category in the store's order (`categories` as
 *  api.categories() gives them) that an entry is in, plus `chosen` if it's one, so the choice
 *  stays in the list. */
export function categoryFilters(
  all: Entry[],
  categories: readonly Category[],
  chosen?: Filter,
): Array<{ id: CategoryFilter; label: string }> {
  return categories
    .filter((c) => categoryFilter(c.slug) === chosen || all.some((e) => inCategory(e, c)))
    .map((c) => ({ id: categoryFilter(c.slug), label: c.name }));
}

export function groupLabel(group: Group, desktop: boolean): string {
  switch (group) {
    case "installed":
      return "Installed";
    case "browser":
      return "Plays in your browser";
    case "library":
      return desktop ? "Not installed" : "In your library";
    case "private":
      return "Private";
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

const ORDER: Group[] = ["installed", "browser", "library", "private", "interested"];

/** Rows shown dimmed: what isn't ready to play from here (installed apps and browser games are). */
export function dimmed(e: Pick<Entry, "group">): boolean {
  return e.group !== "installed" && e.group !== "browser";
}

/** The groups to show, in order, with what matches; a group with no matches is left out. A
 *  category filter needs the store's categories. */
export function sections(
  all: Entry[],
  filter: Filter,
  query: string,
  desktop: boolean,
  categories: readonly Category[] = [],
): Section[] {
  return ORDER.map((group) => {
    const members = all.filter((e) => e.group === group);
    return {
      group,
      label: groupLabel(group, desktop),
      entries: members.filter((e) => matchesFilter(e, filter, categories) && matchesSearch(e, query)),
      total: members.length,
    };
  }).filter((s) => s.entries.length > 0);
}

/** "(3)", or "(1/3)" while a filter or search hides some, like Steam's "(23/245)". */
export function countLabel(s: Pick<Section, "entries" | "total">): string {
  return s.entries.length === s.total ? `(${s.total})` : `(${s.entries.length}/${s.total})`;
}
