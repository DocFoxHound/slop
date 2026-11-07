// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AppSummary, AppUpdate, Category } from "../api";
import type { InstalledApp } from "../platform";
import {
  buildEntries,
  categoryFilter,
  categoryFilters,
  countLabel,
  dimmed,
  filtersFor,
  groupLabel,
  inCategory,
  matchesFilter,
  sections,
  statusOf,
  type Live,
} from "./libraryModel";
import { downloading, downloadsLabel } from "../components/Downloads";

function app(slug: string, title: string, extra: Partial<AppSummary> = {}): AppSummary {
  return {
    slug,
    title,
    summary: `${title} does things.`,
    kind: "game",
    stage: "released",
    min_age: 0,
    content_labels: [],
    players_now: null,
    tags: [],
    thumbnail_url: null,
    icon_url: null,
    vote_count: 0,
    not_slop_percent: null,
    made_with_people: false,
    interested: 0,
    branch_of: null,
    plays_in_browser: false,
    is_dead: false,
    stage_deadline: null,
    updated_at: "",
    ...extra,
  };
}

function inst(slug: string, title: string, extra: Partial<InstalledApp> = {}): InstalledApp {
  return { slug, title, maker: "M", version: "1.0", release_id: `r-${slug}`, kind: "archive", installed_at: "", ...extra };
}

const update = (slug: string, extra: Partial<AppUpdate> = {}): AppUpdate => ({ slug, latest: null, pending: null, builds: [], ...extra });

const sources = {
  library: [app("zeta", "Zeta"), app("alpha", "Alpha"), app("orig", "Original"), app("mine", "Mine")],
  installed: [
    { installed: inst("orig", "Original"), update: update("orig", { latest: { release_id: "new", version: "2.0" } }), available: true },
    { installed: inst("fork", "A Fork", { build_of: "orig" }), available: false },
    { installed: inst("solo", "Solo"), update: update("solo", { pending: { version: "1.1", status: "fetching" } }), available: false },
  ],
  interested: [app("soon", "Soon", { stage: "coming_soon" }), app("alpha", "Alpha"), app("out", "Out Now")],
  privateMine: [app("mine", "Mine")],
  privateShared: [{ ...app("theirs", "Theirs"), owner: "Bo" }],
};

const live: Live = { downloading: {}, running: [], autoUpdate: true };

describe("the library's list", () => {
  const entries = buildEntries(sources);

  it("lists every app once: installed (builds under their original), private, library, interested", () => {
    expect(entries.map((e) => [e.slug, e.group])).toEqual([
      ["orig", "installed"],
      ["fork", "installed"],
      ["solo", "installed"],
      ["mine", "private"],
      ["theirs", "private"],
      ["alpha", "library"],
      ["zeta", "library"],
      ["out", "interested"],
      ["soon", "interested"],
    ]);
    const fork = entries.find((e) => e.slug === "fork")!;
    expect(fork.buildOf).toBe("Original");
    expect(entries.find((e) => e.slug === "mine")).toMatchObject({ owner: "you", inLibrary: true });
    expect(entries.find((e) => e.slug === "alpha")).toMatchObject({ inLibrary: true, interested: true });
  });

  it("says what each app is doing, most urgent first", () => {
    const of = (slug: string, l: Partial<Live> = {}) => statusOf(entries.find((e) => e.slug === slug)!, { ...live, ...l });
    expect(of("orig")).toBe("Update queued");
    expect(of("orig", { autoUpdate: false })).toBe("Update ready");
    expect(of("orig", { downloading: { orig: 40 } })).toBe("Downloading 40%");
    expect(of("orig", { running: ["orig"] })).toBe("Running");
    expect(of("solo")).toBe("Scanning update");
    expect(of("fork")).toBeNull();
    expect(of("zeta")).toBeNull();
    expect(of("out")).toBe("Released");
    expect(of("soon")).toBeNull();
    const installer = buildEntries({
      ...sources,
      installed: [{ installed: inst("setup", "Setup", { kind: "installer" }), update: update("setup", { latest: { release_id: "x", version: "2" } }), available: true }],
    })[0];
    // Apps with their own installer aren't updated automatically.
    expect(statusOf(installer, live)).toBe("Update ready");
    const review = buildEntries({
      ...sources,
      installed: [{ installed: inst("r", "R"), update: update("r", { pending: { version: "2", status: "awaiting_review" } }), available: false }],
    })[0];
    expect(statusOf(review, live)).toBe("Update in review");
  });

  it("filters, searches and counts like Steam's groups", () => {
    const slugs = (f: Parameters<typeof matchesFilter>[1]) => entries.filter((e) => matchesFilter(e, f)).map((e) => e.slug);
    expect(slugs("installed")).toEqual(["orig", "fork", "solo"]);
    expect(slugs("updates")).toEqual(["orig", "solo"]);
    expect(slugs("private")).toEqual(["mine", "theirs"]);
    expect(slugs("interested")).toEqual(["alpha", "out", "soon"]);
    expect(slugs("community")).toEqual(["fork"]);

    const all = sections(entries, "all", "", true);
    expect(all.map((s) => [s.label, countLabel(s)])).toEqual([
      ["Installed", "(3)"],
      ["Not installed", "(2)"],
      ["Private", "(2)"],
      ["Interested", "(2)"],
    ]);
    const searched = sections(entries, "all", "  ZE ", true);
    expect(searched.map((s) => [s.group, countLabel(s), s.entries.map((e) => e.slug)])).toEqual([["library", "(1/2)", ["zeta"]]]);
    // The store's summary is searched too (installed apps the store doesn't list have none).
    expect(sections(entries, "all", "does things", true).flatMap((s) => s.entries)).toHaveLength(7);
    expect(sections(entries, "all", "nothing like this", true)).toEqual([]);
  });

  it("names things for where it runs", () => {
    expect(groupLabel("library", true)).toBe("Not installed");
    expect(groupLabel("library", false)).toBe("In your library");
    expect(groupLabel("browser", true)).toBe("Plays in your browser");
    expect(groupLabel("browser", false)).toBe("Plays in your browser");
    expect(filtersFor(false).map((f) => f.id)).toEqual(["all", "browser", "private", "interested", "community"]);
    expect(filtersFor(true).map((f) => f.id)).toEqual(["all", "installed", "updates", "browser", "private", "interested", "community"]);
    expect(filtersFor(true).find((f) => f.id === "browser")?.label).toBe("Plays in your browser");
  });
});

describe("games that play in the browser", () => {
  const browser = (slug: string, title: string, extra: Partial<AppSummary> = {}) => app(slug, title, { plays_in_browser: true, ...extra });
  const s = {
    ...sources,
    library: [...sources.library, browser("orbit", "Orbit"), browser("comet", "Comet")],
    // One Pat is only waiting for (Interested, coming soon), and one both in the library and Interested.
    interested: [...sources.interested, browser("later", "Later", { stage: "coming_soon" }), browser("comet", "Comet")],
  };
  const entries = buildEntries(s);
  const group = (slug: string) => entries.find((e) => e.slug === slug)?.group;

  it("sit in their own group after Installed, on the website and in the SLAPorSLOP app, never under Not installed", () => {
    expect(group("orbit")).toBe("browser");
    expect(group("comet")).toBe("browser");
    // Waiting for one is still Interested; it isn't in the library yet.
    expect(group("later")).toBe("interested");
    for (const desktop of [true, false]) {
      const shown = sections(entries, "all", "", desktop);
      expect(shown.map((x) => x.group)).toEqual(["installed", "browser", "library", "private", "interested"]);
      const own = shown.find((x) => x.group === "browser")!;
      expect(own.label).toBe("Plays in your browser");
      expect(own.entries.map((e) => e.slug)).toEqual(["comet", "orbit"]);
      expect(countLabel(own)).toBe("(2)");
      expect(shown.find((x) => x.group === "library")!.entries.map((e) => e.slug)).not.toContain("orbit");
    }
  });

  it("aren't dimmed: they play from here, like installed apps", () => {
    const of = (slug: string) => entries.find((e) => e.slug === slug)!;
    expect(dimmed(of("orbit"))).toBe(false);
    expect(dimmed(of("orig"))).toBe(false);
    expect(dimmed(of("zeta"))).toBe(true);
    expect(dimmed(of("later"))).toBe(true);
  });

  it("have their own filter, and none of the installed ones'", () => {
    const slugs = (f: Parameters<typeof matchesFilter>[1]) => entries.filter((e) => matchesFilter(e, f)).map((e) => e.slug);
    expect(slugs("browser")).toEqual(["comet", "orbit", "later"]);
    expect(slugs("installed")).not.toContain("orbit");
    expect(slugs("updates")).not.toContain("orbit");
    // Filtered, the browser group shows whole and Interested keeps the one waiting (of Soon, Out Now, Later).
    const filtered = sections(entries, "browser", "", true);
    expect(filtered.map((x) => [x.group, countLabel(x)])).toEqual([
      ["browser", "(2)"],
      ["interested", "(1/3)"],
    ]);
  });

  it("have no status: nothing downloads, updates or runs in SLAPorSLOP", () => {
    const orbit = entries.find((e) => e.slug === "orbit")!;
    expect(statusOf(orbit, { downloading: {}, running: [], autoUpdate: true })).toBeNull();
    expect(orbit).toMatchObject({ inLibrary: true, available: false });
    expect(orbit.installed).toBeUndefined();
    expect(entries.find((e) => e.slug === "comet")).toMatchObject({ inLibrary: true, interested: true });
  });
});

describe("the store's categories", () => {
  const cat = (slug: string, name: string, tags: string[]): Category => ({
    slug,
    name,
    description: "",
    tags: tags.map((t) => ({ slug: t, name: t })),
  });
  // In the store's order (api.categories()): Servers right after Utilities.
  const categories = [
    cat("games", "Games", ["puzzle", "rpg"]),
    cat("tools", "Tools", ["office"]),
    cat("utilities", "Utilities", ["backup"]),
    cat("servers", "Servers", ["dedicated-server"]),
    cat("vibe", "Vibe", ["cozy"]),
  ];
  const entries = buildEntries({
    library: [
      app("relay", "Relay Server", { kind: "app", tags: ["dedicated-server", "backup"] }),
      app("castle", "Castle", { tags: ["puzzle", "cozy"] }),
      app("orbit", "Orbit", { plays_in_browser: true, tags: ["puzzle"] }),
    ],
    // Installed and no longer in the store: no tags to go by.
    installed: [{ installed: inst("gone", "Gone"), available: false }],
    interested: [app("soon", "Soon", { stage: "coming_soon", tags: ["rpg"] })],
    privateMine: [],
    privateShared: [],
  });
  const of = (slug: string) => entries.find((e) => e.slug === slug)!;

  it("an entry is in a category when one of its app's tags is; one without store data is in none", () => {
    expect(inCategory(of("relay"), categories[3])).toBe(true);
    expect(inCategory(of("relay"), categories[2])).toBe(true);
    expect(inCategory(of("castle"), categories[3])).toBe(false);
    expect(of("gone").app).toBeUndefined();
    for (const c of categories) expect(inCategory(of("gone"), c), c.slug).toBe(false);
  });

  it("lists each category the library has an entry in, in the store's order, by its name", () => {
    expect(categoryFilters(entries, categories)).toEqual([
      { id: "category:games", label: "Games" },
      { id: "category:utilities", label: "Utilities" },
      { id: "category:servers", label: "Servers" },
      { id: "category:vibe", label: "Vibe" },
    ]);
    // A chosen category stays in the list, though nothing's in it any more.
    expect(categoryFilters(entries, categories, categoryFilter("tools")).map((f) => f.label)).toEqual([
      "Games",
      "Tools",
      "Utilities",
      "Servers",
      "Vibe",
    ]);
    expect(categoryFilters(entries, [])).toEqual([]);
    expect(categoryFilters([], categories)).toEqual([]);
  });

  it("filters each group by category, beside search and the counts", () => {
    const slugs = (f: Parameters<typeof matchesFilter>[1]) => entries.filter((e) => matchesFilter(e, f, categories)).map((e) => e.slug);
    expect(slugs(categoryFilter("servers"))).toEqual(["relay"]);
    // Browser games and Interested ones are in their categories too.
    expect(slugs(categoryFilter("games"))).toEqual(["castle", "orbit", "soon"]);
    expect(slugs(categoryFilter("tools"))).toEqual([]);
    // Without the store's categories (not loaded, or a category it doesn't have), nothing matches.
    expect(entries.filter((e) => matchesFilter(e, categoryFilter("servers")))).toEqual([]);
    expect(slugs(categoryFilter("no-such-category"))).toEqual([]);
    const shown = sections(entries, categoryFilter("games"), "", false, categories);
    expect(shown.map((x) => [x.group, countLabel(x)])).toEqual([
      ["browser", "(1)"],
      ["library", "(1/2)"],
      ["interested", "(1)"],
    ]);
    expect(sections(entries, categoryFilter("games"), "cast", false, categories).flatMap((x) => x.entries.map((e) => e.slug))).toEqual([
      "castle",
    ]);
    // The kinds' filters are as before.
    expect(slugs("all")).toHaveLength(entries.length);
  });
});

describe("downloads", () => {
  it("counts what finished of what's going and queued", () => {
    expect(downloadsLabel(0, 0, 0)).toBe("Downloads");
    expect(downloadsLabel(2, 0, 1)).toBe("Downloads – 2 of 3 complete");
  });

  it("shows downloads still going, not finished or stalled ones", () => {
    const now = 1_000_000;
    expect(
      downloading(
        [
          { slug: "a", got: 40, total: 100, done: false, at: now - 1000 },
          { slug: "b", got: 100, total: 100, done: true, at: now },
          { slug: "c", got: 10, total: 100, done: false, at: now - 120_000 },
        ],
        now,
      ),
    ).toEqual({ a: 40 });
  });
});
