import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { Ratings } from "../api";
import { LineChart } from "./Charts";
import { RatingSummary } from "./RatingPanel";

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
};

describe("RatingSummary", () => {
  afterEach(cleanup);

  it("waits for enough votes before showing a percentage", () => {
    render(<RatingSummary ratings={base} />);
    expect(screen.getByText("4 votes")).toBeTruthy();
    expect(screen.queryByText(/% Not Slop/)).toBeNull();
  });

  it("shows weighted first, unweighted beside it, with the label", () => {
    render(<RatingSummary ratings={{ ...base, not_slop: 8, slop: 2, percent: 80, percent_weighted: 91, label: "Very Not Slop" }} />);
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
