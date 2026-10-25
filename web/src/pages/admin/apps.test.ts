// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AdminApp, AdminAppFlag } from "../../api";
import { cantListReason, flagFact, pageQuery, quietLine, readShow, readSort, rowFacts, span } from "./apps";

const NOW = Date.parse("2026-10-25T12:00:00Z");
const daysAgo = (n: number) => new Date(NOW - n * 86_400_000).toISOString();

const app = (over: Partial<AdminApp> = {}): AdminApp => ({
  slug: "harbour",
  title: "Harbour Lights",
  stage: "released",
  status: "listed",
  repo_url: "https://github.com/x/y",
  maker: "Fin",
  releases: [{ id: "r1", version: "1.0.0", reviewed_at: daysAgo(10) }],
  pending_releases: 0,
  flags: [],
  visibility: "public",
  icon_url: null,
  thumbnail_url: null,
  frozen_at: null,
  frozen_reason: null,
  stage_deadline: null,
  last_commit_at: null,
  updated_at: daysAgo(1),
  newest_release: { version: "1.0.0", status: "approved", status_reason: null },
  serving_version: "1.0.0",
  rescan_flags: 0,
  downloads_lost: 0,
  open_reports: 0,
  donations_paused: false,
  players: 0,
  last_played_at: null,
  vote_count: 0,
  not_slop_percent: null,
  ...over,
});

describe("flagFact", () => {
  const text = (over: Partial<AdminApp>, flag: AdminAppFlag) => flagFact(app(over), flag, NOW);

  it("says each problem with what's behind it", () => {
    expect(text({ rescan_flags: 1 }, "rescan_flagged")).toEqual({ tone: "bad", text: "Re-scan found something in a live file" });
    expect(text({ rescan_flags: 3 }, "rescan_flagged").text).toBe("Re-scan found something in 3 live files");
    expect(text({ downloads_lost: 1 }, "download_lost").text).toBe("Its download changed or can't be reached");
    expect(text({ frozen_reason: "Slop lost access to its repository" }, "frozen").text).toBe(
      "Frozen: Slop lost access to its repository",
    );
    expect(text({}, "frozen").text).toBe("Frozen");
    expect(text({ open_reports: 2 }, "reports").text).toBe("2 open reports");
    expect(
      text({ newest_release: { version: "0.2.0", status: "failed", status_reason: "The scanner timed out" } }, "release_failed"),
    ).toEqual({ tone: "bad", text: "0.2.0 failed: The scanner timed out" });
    expect(text({ pending_releases: 1 }, "release_waiting")).toEqual({ tone: "warn", text: "A release waiting for review" });
    expect(text({ donations_paused: true }, "donations_paused").text).toBe("Donations paused");
  });

  it("counts the days to and since a deadline", () => {
    expect(text({ stage: "coming_soon", stage_deadline: daysAgo(-5) }, "deadline_soon").text).toBe(
      "Coming soon time runs out in 5 days",
    );
    expect(text({ stage: "beta", stage_deadline: daysAgo(35) }, "past_deadline")).toEqual({
      tone: "bad",
      text: "Hidden from the store: its Beta time ran out 35 days ago",
    });
  });

  it("calls a long-quiet game possibly dead", () => {
    expect(text({ last_commit_at: daysAgo(200) }, "no_commits").text).toBe("Dead? No commits for 6 months");
    expect(text({ last_played_at: daysAgo(100) }, "no_players").text).toBe("Dead? Nobody has played it for 3 months");
    expect(text({}, "no_players").text).toBe("Dead? Nobody has played it yet");
    expect(text({ not_slop_percent: 12, vote_count: 40 }, "low_rating").text).toBe("Rated 12% Not Slop (40 votes)");
  });
});

describe("span", () => {
  it("uses days, then months, then years", () => {
    expect(span(1)).toBe("1 day");
    expect(span(59)).toBe("59 days");
    expect(span(90)).toBe("3 months");
    expect(span(800)).toBe("2 years");
  });
});

describe("rowFacts", () => {
  it("shows a healthy app's quiet line", () => {
    const healthy = app({ players: 12, last_played_at: daysAgo(2), not_slop_percent: 85 });
    expect(rowFacts(healthy, NOW)).toEqual({
      shown: [{ tone: "quiet", text: "Players get 1.0.0 · 12 players · played 2 days ago · 85% Not Slop" }],
      more: [],
    });
  });

  it("shows the two most urgent flags and counts the rest", () => {
    const sick = app({ flags: ["frozen", "reports", "low_rating"], open_reports: 1, not_slop_percent: 10, vote_count: 9 });
    const { shown, more } = rowFacts(sick, NOW);
    expect(shown.map((f) => f.text)).toEqual(["Frozen", "1 open report"]);
    expect(more.map((f) => f.text)).toEqual(["Rated 10% Not Slop (9 votes)"]);
  });
});

describe("quietLine", () => {
  it("says what players get, or where the newest release is", () => {
    expect(quietLine(app({ serving_version: null, stage: "coming_soon", releases: [], newest_release: null }), NOW)).toBe(
      "No release yet",
    );
    expect(
      quietLine(app({ serving_version: null, newest_release: { version: "0.3.0", status: "awaiting_review", status_reason: null } }), NOW),
    ).toBe("0.3.0 awaiting review");
  });
});

describe("cantListReason", () => {
  it("explains why a draft can't go in yet", () => {
    expect(cantListReason(app())).toBeNull();
    expect(cantListReason(app({ status: "draft", stage: "coming_soon", releases: [] }))).toBeNull();
    expect(cantListReason(app({ status: "draft", stage: "beta", releases: [], pending_releases: 1 }))).toMatch(
      /approve its release under Review queue → New releases first/,
    );
    expect(cantListReason(app({ status: "delisted", stage: "beta", releases: [] }))).toMatch(
      /the maker hasn't submitted a release\. Set the stage to Coming soon/,
    );
  });
});

describe("filters", () => {
  it("reads the address, defaulting sensibly", () => {
    expect(readShow(null)).toBe("");
    expect(readShow("attention")).toBe("attention");
    expect(readShow("bogus")).toBe("");
    expect(readSort(null, "")).toBe("updated");
    expect(readSort(null, "attention")).toBe("urgent");
    expect(readSort("title", "attention")).toBe("title");
    expect(readSort("bogus", "listed")).toBe("updated");
  });

  it("asks the API for one page", () => {
    expect(pageQuery("attention", "urgent", "", 0)).toEqual({
      status: undefined,
      attention: true,
      sort: "urgent",
      q: undefined,
      limit: 50,
      offset: 0,
    });
    expect(pageQuery("draft", "title", "moss", 2)).toEqual({
      status: "draft",
      attention: false,
      sort: "title",
      q: "moss",
      limit: 50,
      offset: 100,
    });
  });
});
