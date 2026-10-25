// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";

import { api, type Ratings } from "../api";
import { LineChart, UpDownBars } from "./Charts";
import { RatingHeader, RatingSummary, rateStep, SinceNewWork } from "./RatingPanel";

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

describe("Rated in Beta", () => {
  afterEach(cleanup);

  it("says how many of the votes were given in Beta", () => {
    render(<RatingSummary ratings={{ ...base, in_beta: 2 }} />, { wrapper: MemoryRouter });
    expect(screen.getByText(/2 of these votes were given while it was in Beta/)).toBeTruthy();
    cleanup();
    render(<RatingSummary ratings={{ ...base, in_beta: 4 }} />, { wrapper: MemoryRouter });
    expect(screen.getByText(/All 4 votes were given while it was in Beta/)).toBeTruthy();
    cleanup();
    render(<RatingSummary ratings={{ ...base, in_beta: 0 }} />, { wrapper: MemoryRouter });
    expect(screen.queryByText(/in Beta/)).toBeNull();
  });
});

describe("rateStep", () => {
  const mine = { vote: null, can_vote: false, reason: null, played: false, weight: 0.1 } as const;

  it("is Rate it whenever the viewer can vote, in Beta too", () => {
    expect(rateStep({ ...base, stage: "beta", mine: { ...mine, can_vote: true, played: true } }, false, true)).toEqual({
      kind: "rate",
      vote: null,
      beta: true,
    });
    expect(rateStep({ ...base, stage: "released", mine: { ...mine, can_vote: true, played: true, vote: "slop" } }, false, true)).toEqual({
      kind: "rate",
      vote: "slop",
      beta: false,
    });
  });

  it("otherwise names the step to take", () => {
    expect(rateStep({ ...base, open: false, stage: "coming_soon" }, false, false).kind).toBe("coming_soon");
    expect(rateStep(base, true, true).kind).toBe("maker");
    expect(rateStep(base, false, false).kind).toBe("sign_in");
    expect(rateStep({ ...base, mine: { ...mine, reason: "Launch it from the Slop app first: only players can vote." } }, false, true).kind).toBe(
      "play",
    );
    expect(rateStep({ ...base, mine: { ...mine, played: true, reason: "Something else." } }, false, true)).toEqual({
      kind: "other",
      reason: "Something else.",
    });
  });
});

describe("RatingHeader", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  const show = async (r: Ratings, o: { isMine?: boolean; signedIn?: boolean } = {}) => {
    vi.spyOn(api, "ratings").mockResolvedValue(r);
    render(
      <MemoryRouter>
        <RatingHeader
          slug="herds"
          title="Angry Herds"
          isMine={o.isMine ?? false}
          signedIn={o.signedIn ?? true}
          interested={<button type="button">☆ I'm interested</button>}
        />
      </MemoryRouter>,
    );
    await screen.findByText("No ratings yet");
  };

  it("offers Rate it as the main button to a player who played the Beta", async () => {
    await show({ ...base, stage: "beta", mine: { vote: null, can_vote: true, reason: null, played: true, weight: 0.3 } });
    expect(screen.getByRole("button", { name: "Rate it" }).className).toContain("slime");
    expect(screen.getByText(/your rating is marked Rated in Beta/)).toBeTruthy();
  });

  it("gives signed-out visitors a Sign in button, and the website's players the Slop app", async () => {
    await show({ ...base, stage: "beta" }, { signedIn: false });
    expect(screen.getByRole("link", { name: "Sign in to rate" }).getAttribute("href")).toBe(`/signin?return_to=${encodeURIComponent("/app/herds")}`);
    cleanup();
    await show({ ...base, stage: "beta", mine: { vote: null, can_vote: false, reason: "Launch it from the Slop app first: only players can vote.", played: false, weight: 0.1 } });
    expect(screen.getByText("Play it from the Slop app to rate it.")).toBeTruthy();
    expect(screen.getByRole("link", { name: "⬇ Get the Slop app" }).getAttribute("href")).toBe("/download");
  });

  it("tells the maker, and offers Interested while it's Coming soon", async () => {
    await show({ ...base, stage: "beta" }, { isMine: true });
    expect(screen.getByText("Makers can't rate their own app.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Rate it" })).toBeNull();
    cleanup();
    await show({ ...base, open: false, stage: "coming_soon" }, { signedIn: false });
    expect(screen.getByText("Ratings open when it's playable")).toBeTruthy();
    expect(screen.getByRole("button", { name: "☆ I'm interested" })).toBeTruthy();
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
