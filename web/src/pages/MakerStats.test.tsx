// SPDX-License-Identifier: AGPL-3.0-or-later
// Publish → Stats (MakerStats.tsx): the account's cards, a row per app linking to its Manage
// page with a 30-day sparkline and its text alternative, and the empty state.
import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type AppStats, type MakerStats as Stats } from "../api";
import { MakerStats, moneyLine, sparkSummary } from "./MakerStats";

vi.mock("../session", () => ({ useSession: () => ({ me: { id: "mia" } }) }));

const DAY = 86_400_000;
const days = Array.from({ length: 30 }, (_, i) => new Date(Date.UTC(2026, 10, 19) - (29 - i) * DAY).toISOString().slice(0, 10));

function appStats(o: Partial<AppStats>): AppStats {
  return {
    slug: "arena",
    title: "Arena",
    kind: "game",
    stage: "released",
    status: "listed",
    private: false,
    branch_of: null,
    server_for: null,
    counted: true,
    library_adds: 14,
    library_adds_7d: 5,
    library_adds_30d: 9,
    daily_library_adds: days.map((_, i) => (i === 29 ? 2 : i === 26 ? 3 : i === 19 ? 4 : 0)),
    first_plays_30d: 3,
    peak_players_30d: 7,
    in_libraries: 2,
    not_slop: 4,
    slop: 1,
    not_slop_percent: 80,
    interested: 0,
    releases_published: 2,
    open_bounties: 1,
    reviews: 1,
    donations: [{ currency: "usd", donations: 3, donated_minor: 1800, refunded_minor: 300 }],
    awards: [{ month: "2026-10", position: 1 }],
    ...o,
  };
}

const stats: Stats = {
  simulated: false,
  days,
  account: {
    apps: 2,
    counts: { draft: 0, listed: 2, delisted: 0, coming_soon: 0, beta: 0, released: 1, private: 1 },
    library_adds: 14,
    library_adds_30d: 9,
    first_plays_30d: 3,
    in_libraries: 2,
    not_slop: 4,
    slop: 1,
    not_slop_percent: 80,
    interested: 0,
    releases_published: 2,
    awards: 1,
    donations: [{ currency: "usd", donations: 3, donated_minor: 1800, refunded_minor: 300 }],
    donations_this_year: [{ currency: "usd", donations: 2, donated_minor: 1300, refunded_minor: 300 }],
    busiest: { slug: "arena", title: "Arena", library_adds_30d: 9, peak_players_30d: 7 },
  },
  apps: [
    appStats({}),
    appStats({
      slug: "secret",
      title: "Secret",
      private: true,
      counted: false,
      library_adds: null,
      library_adds_7d: null,
      library_adds_30d: null,
      first_plays_30d: null,
      daily_library_adds: [],
      peak_players_30d: null,
      not_slop_percent: null,
      donations: [],
      awards: [],
    }),
  ],
};

function show() {
  return render(
    <MemoryRouter initialEntries={["/publish/stats"]}>
      <MakerStats />
    </MemoryRouter>,
  );
}

describe("the Stats tab", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("shows the account's cards and a row per app", async () => {
    vi.spyOn(api, "makerStats").mockResolvedValue(stats);
    show();
    const table = await screen.findByRole("table", { name: /Your apps, newest changed first/ });
    // The tabs, with Stats the current page.
    const tabs = screen.getByRole("navigation", { name: "Publish" });
    expect(within(tabs).getByRole("link", { name: "Stats" }).getAttribute("aria-current")).toBe("page");
    expect(within(tabs).getByRole("link", { name: "Your apps" }).getAttribute("href")).toBe("/publish");

    const account = screen.getByRole("region", { name: "Your account" });
    expect(within(account).getByText("Library adds, 30 days").nextElementSibling?.textContent).toBe("9");
    expect(within(account).getByText("Slaps").nextElementSibling?.textContent).toBe("80%");
    expect(within(account).getByText(/This year: \$13 from 2 donations/)).toBeTruthy();
    expect(within(account).getByRole("link", { name: "Arena" }).getAttribute("href")).toBe("/publish/arena/manage");

    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(2);
    const arena = rows[0];
    expect(within(arena).getByRole("rowheader").textContent).toContain("Released · In the store");
    expect(within(arena).getByRole("link", { name: "Arena" }).getAttribute("href")).toBe("/publish/arena/manage");
    expect(arena.textContent).toContain("5 in 7 days · 14 in all");
    expect(arena.textContent).toContain("$18 from 3 donations");
    const spark = within(arena).getByRole("img");
    expect(spark.getAttribute("aria-label")).toBe(sparkSummary(stats.apps[0].daily_library_adds, days));
    expect(spark.getAttribute("aria-label")).toContain("9 in all; the most on one day was 4");

    const secret = rows[1];
    expect(within(secret).getByRole("rowheader").textContent).toContain("Private");
    expect(within(secret).getAllByText("Not counted")).toHaveLength(2);
    expect(within(secret).queryByRole("img")).toBeNull();
  });

  it("says when there are no apps yet", async () => {
    vi.spyOn(api, "makerStats").mockResolvedValue({ ...stats, apps: [], account: { ...stats.account, apps: 0 } });
    show();
    expect(await screen.findByText(/No stats yet: you haven't made anything\./)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Create your first draft" }).getAttribute("href")).toBe("/publish/new");
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("says simulated donations are test payments", async () => {
    vi.spyOn(api, "makerStats").mockResolvedValue({ ...stats, simulated: true });
    show();
    expect(await screen.findByText(/This server simulates payments/)).toBeTruthy();
  });
});

describe("the sparkline's text", () => {
  it("says when nothing happened, and from when it was counted", () => {
    expect(sparkSummary(days.map(() => 0), days)).toBe("Library adds per day, last 30 days: none.");
    const late = days.map((_, i) => (i < 28 ? null : i === 29 ? 1 : 0));
    expect(sparkSummary(late, days)).toMatch(/^Library adds per day, last 30 days \(counted from .+\): 1 in all;/);
    expect(sparkSummary([], [])).toBe("Library adds per day: nothing counted in the last 30 days.");
  });

  it("puts each currency's total in one line", () => {
    expect(
      moneyLine([
        { currency: "eur", donations: 1, donated_minor: 700, refunded_minor: 0 },
        { currency: "usd", donations: 2, donated_minor: 1250, refunded_minor: 0 },
      ]),
    ).toBe("€7 from 1 donation · $12.50 from 2 donations");
  });
});
