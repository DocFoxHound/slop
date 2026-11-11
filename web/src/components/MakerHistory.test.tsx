// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type AppHistory, type HistoryDay } from "../api";
import { HistoryPanel } from "./MakerHistory";

const END = Date.UTC(2026, 10, 10); // 2026-11-10, the last day of every answer below.
const DAY = 86_400_000;

/** `n` UTC days ending on 2026-11-10, oldest first, each filled by `fill(i)` (i = 0 is the oldest). */
function answer(n: number, o: { history_from?: string; players_from?: string; fill?: (i: number, day: string) => Partial<HistoryDay> } = {}): AppHistory {
  const days = Array.from({ length: n }, (_, i) => {
    const day = new Date(END - (n - 1 - i) * DAY).toISOString().slice(0, 10);
    return { day, players: 0, first_plays: 0, library_adds: 0, peak: 0, ...o.fill?.(i, day) };
  });
  return { history_from: o.history_from ?? "2026-01-01", players_from: o.players_from ?? "2026-01-01", days };
}

/** A month with players per day counted only from 2026-11-07 (three days before the end). */
const month = answer(30, {
  history_from: "2026-10-01",
  players_from: "2026-11-07",
  fill: (i, day) => ({
    players: day >= "2026-11-07" ? [4, 2, 0, 3][i - 26] : null,
    first_plays: 1,
    library_adds: i === 5 ? 2 : i === 20 ? 3 : 0,
  }),
});

const panel = () => screen.getByRole("region", { name: "History" });

describe("HistoryPanel", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("shows the charts, the totals and the numbers", async () => {
    vi.spyOn(api, "history").mockResolvedValue(month);
    const { container } = render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(within(panel()).getByRole("heading", { name: "History", level: 2 })).toBeTruthy();
    expect(screen.getByText("Library adds per day")).toBeTruthy();
    expect(screen.getByText(/Totals only, never who\./)).toBeTruthy();
    // Who isn't counted (amendment 9.1).
    expect(screen.getByText(/Your own plays and adds aren't counted, and neither are plays by people who switched off "Share my playtime"\./)).toBeTruthy();
    const titles = container.querySelectorAll("svg title");
    const summary = titles[0]?.textContent ?? "";
    expect(summary).toContain("from 2026-10-12 to 2026-11-10 (UTC days)");
    expect(summary).toContain("The most on one day was 4.");
    expect(summary).toContain("First plays in that time: 30.");
    // Library adds are times added, not accounts (amendment 9.3).
    expect(titles[1]?.textContent).toBe("Times it was added to a library each day from 2026-10-12 to 2026-11-10 (UTC days): 5 in all.");
    expect(screen.getByText(/In the days shown: 30 first plays and 5 library adds\./)).toBeTruthy();
    expect(screen.getByText(/Players per day aren't added up/)).toBeTruthy();

    fireEvent.click(screen.getAllByRole("button", { name: "Show numbers" })[0]);
    const table = within(panel()).getByRole("table");
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(30);
    // Not counted that day is a dash, not 0; a counted day with nobody is 0.
    const cells = (when: string) => within(rows.find((r) => r.firstElementChild?.textContent === when)!).getAllByRole("cell").map((c) => c.textContent);
    expect(cells("10-12")).toEqual(["10-12", "—", "1"]);
    expect(cells("11-09")).toEqual(["11-09", "0", "1"]);
    expect(cells("11-10")).toEqual(["11-10", "3", "1"]);

    expect(screen.getByText(/Players per day are counted from 2026-11-07, and that first day is a part day\./)).toBeTruthy();
    expect(screen.getByText(/A dash in the numbers means not counted, which isn't zero\./)).toBeTruthy();
    expect(screen.getByText(/SLAPorSLOP doesn't count installs\. First plays are the closest number/)).toBeTruthy();
  });

  it("shows the busiest moment only when Pulse has numbers", async () => {
    vi.spyOn(api, "history").mockResolvedValue(month);
    render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(screen.queryByText("Players, busiest moment each day")).toBeNull();
    expect(screen.queryByText(/From Slop Pulse/)).toBeNull();
    cleanup();

    vi.spyOn(api, "history").mockResolvedValue({ ...month, days: month.days.map((d, i) => ({ ...d, peak: i === 28 ? 4 : d.peak })) });
    const { container } = render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players, busiest moment each day");
    expect(container.querySelectorAll("svg")).toHaveLength(3);
    expect(container.querySelectorAll("svg title")[2].textContent).toBe("The most players at once each day from 2026-10-12 to 2026-11-10. The highest was 4.");
    // Pulse counts the maker's own play too, unlike the totals above it.
    expect(screen.getByText("From Slop Pulse: it counts everyone who shares their playtime, you too if you do.")).toBeTruthy();
  });

  it("asks again when the range changes", async () => {
    const history = vi.spyOn(api, "history").mockImplementation(async (_slug, n) => answer(n, { fill: () => ({ players: 1 }) }));
    render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(history).toHaveBeenCalledWith("pong", 30);
    const days30 = screen.getByRole("button", { name: "30 days" });
    const days90 = screen.getByRole("button", { name: "90 days" });
    expect(screen.getByRole("group", { name: "Days shown" })).toBeTruthy();
    expect(days30.getAttribute("aria-pressed")).toBe("true");
    expect(days90.getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByRole("button", { name: "A year" }).getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(screen.getAllByRole("button", { name: "Show numbers" })[0]);
    days90.focus();
    fireEvent.click(days90);
    expect(history).toHaveBeenLastCalledWith("pong", 90);
    expect(days90.getAttribute("aria-pressed")).toBe("true");
    expect(days30.getAttribute("aria-pressed")).toBe("false");
    // The open table stays open and fills with the new range; focus stays on the button.
    // Rows are counted with a plain selector: a role query over hundreds of rows is slow in jsdom.
    const tableRows = () => within(panel()).getByRole("table").querySelectorAll("tr");
    await waitFor(() => expect(tableRows()).toHaveLength(91));
    expect(document.activeElement).toBe(days90);
    expect(panel().getAttribute("aria-busy")).toBe("false");
    // 90 days are MM-DD; a year names the year too, so the same MM-DD can't be read as the wrong year.
    expect(tableRows()[1].firstElementChild?.textContent).toBe("08-13");

    fireEvent.click(screen.getByRole("button", { name: "A year" }));
    expect(history).toHaveBeenLastCalledWith("pong", 365);
    await waitFor(() => expect(tableRows()).toHaveLength(366));
    const rows = tableRows();
    expect(rows[1].firstElementChild?.textContent).toBe("2025-11-11");
    expect(rows[365].firstElementChild?.textContent).toBe("2026-11-10");
  });

  it("says so when there is nothing yet", async () => {
    // Nothing counted before the 20th; zeros after.
    vi.spyOn(api, "history").mockResolvedValue(
      answer(30, { history_from: "2026-10-20", players_from: "2026-10-20", fill: (_i, day) => (day < "2026-10-20" ? { players: null, first_plays: null, library_adds: null, peak: null } : {}) }),
    );
    const { container } = render(<HistoryPanel slug="pong" />);
    // It says only what was counted: plays by people who don't share their playtime never are.
    await screen.findByText("Nothing to show yet: no plays or library adds were counted in these days.");
    expect(container.querySelector("svg")).toBeNull();
    expect(screen.queryByText(/In the days shown/)).toBeNull();
    expect(screen.getByText(/SLAPorSLOP doesn't count installs\./)).toBeTruthy();
    // No numbers are shown, so there's no dash to explain.
    expect(screen.getByText("Players per day are counted from 2026-10-20, and that first day is a part day.")).toBeTruthy();
    expect(screen.queryByText(/A dash in the numbers/)).toBeNull();
    // The range choice stays, so another range can still be read.
    expect(screen.getByRole("button", { name: "A year" })).toBeTruthy();
  });

  it("shows the error and tries again", async () => {
    const history = vi
      .spyOn(api, "history")
      .mockRejectedValueOnce(new ApiError(500, "internal", "Something went wrong"))
      .mockResolvedValueOnce(month);
    render(<HistoryPanel slug="pong" />);
    expect((await screen.findByRole("alert")).textContent).toContain("Something went wrong");
    expect(screen.queryByText("Players and first plays per day")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(history).toHaveBeenCalledTimes(2);
    await screen.findByText("Players and first plays per day");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says from which day each number was counted", async () => {
    // Made inside the range, before players per day were counted: the back-filled days are named.
    vi.spyOn(api, "history").mockResolvedValue(
      answer(30, {
        history_from: "2026-10-25",
        players_from: "2026-11-03",
        fill: (_i, day) => ({
          players: day >= "2026-11-03" ? 1 : null,
          first_plays: day >= "2026-10-25" ? 1 : null,
          library_adds: day >= "2026-10-25" ? 0 : null,
          peak: day >= "2026-10-25" ? 0 : null,
        }),
      }),
    );
    render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(screen.getByText(/In the days shown: 17 first plays and 0 library adds\./)).toBeTruthy();
    expect(screen.getByText(/Players per day are counted from 2026-11-03/)).toBeTruthy();
    expect(screen.getByText(/First plays and library adds before 2026-11-03 were worked out from what was still on SLAPorSLOP that day/)).toBeTruthy();
    expect(screen.getByText("Nothing is counted before 2026-10-25: the day the app was made, or the day it became public.")).toBeTruthy();
    cleanup();

    // Counted from long before the range: none of the three, and one play is "1 first play".
    vi.spyOn(api, "history").mockResolvedValue(answer(30, { fill: (i) => ({ first_plays: i === 0 ? 1 : 0, library_adds: i === 1 ? 1 : 0 }) }));
    render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(screen.getByText(/In the days shown: 1 first play and 1 library add\./)).toBeTruthy();
    expect(screen.queryByText(/Players per day are counted from/)).toBeNull();
    expect(screen.queryByText(/were worked out from/)).toBeNull();
    expect(screen.queryByText(/Nothing is counted before/)).toBeNull();
    cleanup();

    // Back-filled long ago: no shown day was worked out from what was left, so no note about it.
    vi.spyOn(api, "history").mockResolvedValue(answer(30, { history_from: "2026-01-01", players_from: "2026-02-01", fill: () => ({ players: 1 }) }));
    render(<HistoryPanel slug="pong" />);
    await screen.findByText("Players and first plays per day");
    expect(screen.queryByText(/were worked out from/)).toBeNull();
    expect(screen.queryByText(/Players per day are counted from/)).toBeNull();
  });
});
