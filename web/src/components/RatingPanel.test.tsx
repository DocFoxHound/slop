import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { MemoryRouter } from "react-router";

import type { Ratings } from "../api";
import { LineChart, UpDownBars } from "./Charts";
import { RatingSummary, SinceNewWork } from "./RatingPanel";

const base: Ratings = {
  not_slop: 3,
  slop: 1,
  not_slop_weight: 2,
  slop_weight: 1,
  percent: null,
  percent_weighted: null,
  label: null,
  monthly: [],
  daily: [],
  open: true,
  min_votes: 5,
  since_new_work: null,
};

describe("RatingSummary", () => {
  afterEach(cleanup);

  it("waits for enough votes before showing a percentage", () => {
    render(<RatingSummary ratings={base} />, { wrapper: MemoryRouter });
    expect(screen.getByText("4 votes")).toBeTruthy();
    expect(screen.queryByText(/% Not Slop/)).toBeNull();
  });

  it("shows weighted first, unweighted beside it, with the label", () => {
    render(<RatingSummary ratings={{ ...base, not_slop: 8, slop: 2, percent: 80, percent_weighted: 91, label: "Very Not Slop" }} />, {
      wrapper: MemoryRouter,
    });
    expect(screen.getByText("91% Not Slop")).toBeTruthy();
    expect(screen.getByText("Very Not Slop")).toBeTruthy();
    expect(screen.getByText(/80% unweighted · 10 votes/)).toBeTruthy();
  });
});

describe("LineChart", () => {
  afterEach(cleanup);

  it("describes itself and shows its numbers on request", () => {
    const { container } = render(
      <LineChart
        title="% Not Slop by month"
        summary="Rating by month"
        labels={["2026-08", "2026-09", "2026-10"]}
        max={100}
        series={[{ name: "Weighted", values: [60, null, 80], className: "line-main" }]}
      />,
    );
    expect(container.querySelector("svg title")?.textContent).toBe("Rating by month");
    // A gap splits the line: two lone points, no polyline.
    expect(container.querySelectorAll("polyline").length).toBe(0);
    expect(container.querySelectorAll("circle").length).toBe(2);
    fireEvent.click(screen.getByText("Show numbers"));
    const row = [...container.querySelectorAll("tr")].find((r) => r.textContent?.startsWith("2026-09"));
    expect(row?.textContent).toContain("—");
  });
});

describe("UpDownBars", () => {
  afterEach(cleanup);

  it("shades marked days, and says so in its numbers too", () => {
    const { container } = render(
      <UpDownBars
        title="Votes per day"
        summary="Votes"
        labels={["Oct 1", "Oct 2", "Oct 3"]}
        up={[2, 1, 0]}
        down={[0, 14, 1]}
        upName="Up"
        downName="Down"
        marked={[false, true, false]}
        markedName="Unusual voting"
      />,
    );
    expect(container.querySelectorAll("rect.marked").length).toBe(1);
    expect(screen.getByText("Unusual voting")).toBeTruthy();
    fireEvent.click(screen.getByText("Show numbers"));
    const row = [...container.querySelectorAll("tr")].find((r) => r.textContent?.startsWith("Oct 2"));
    expect(row?.textContent).toContain("yes");
  });
});

describe("SinceNewWork", () => {
  afterEach(cleanup);

  it("waits for the release with the new work", () => {
    render(<SinceNewWork s={{ since: null, version: null, credits: ["Vera"] }} min={5} />);
    expect(screen.getByText(/New work by Vera is coming/)).toBeTruthy();
  });

  it("shows the rating from votes since that release", () => {
    render(
      <SinceNewWork
        s={{ since: "2026-10-01T00:00:00Z", version: "2.0.0", credits: ["Vera", "Loopsmith"], not_slop: 9, slop: 1, percent_weighted: 90, label: "Very Not Slop" }}
        min={5}
      />,
    );
    expect(screen.getByText("90% Not Slop")).toBeTruthy();
    expect(screen.getByText(/by Vera, Loopsmith, from version 2.0.0/)).toBeTruthy();
    expect(screen.getByText(/10 votes cast or changed since/)).toBeTruthy();
  });
});
