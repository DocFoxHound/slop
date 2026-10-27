// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AppSummary, AppUpdate } from "../api";
import type { InstalledApp } from "../platform";
import { buildEntries, countLabel, filtersFor, groupLabel, matchesFilter, sections, statusOf, type Live } from "./libraryModel";
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
    expect(filtersFor(false).map((f) => f.id)).toEqual(["all", "private", "interested", "community"]);
    expect(filtersFor(true).map((f) => f.id)).toContain("updates");
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
