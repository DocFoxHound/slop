// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { CreativeCard } from "../api";
import { PersonCard, RatingLine } from "./People";

const card: CreativeCard = {
  handle: "vera",
  name: "Vera Okafor",
  kind: "individual",
  team_size: null,
  headline: "Pixel art",
  location: "Lagos, NG",
  skills: [{ slug: "pixel-art", name: "Pixel Art" }],
  ai_use: "none",
  taking_work: true,
  can_hire: true,
  accent: "grape",
  avatar_url: null,
  preview: [],
  from_price: { minor: 2500, currency: "usd" },
  rating: { not_slop: 9, slop: 1, percent: 90, label: "Very Not Slop", min_reviews: 3 },
  clients: 10,
};

describe("PersonCard", () => {
  afterEach(cleanup);

  it("says what they do, from what price, and how they use AI", () => {
    render(
      <MemoryRouter>
        <PersonCard c={card} />
      </MemoryRouter>,
    );
    expect(screen.getByRole("link").getAttribute("href")).toBe("/people/vera");
    expect(screen.getByText("from $25")).toBeTruthy();
    expect(screen.getByText(/NO AI/)).toBeTruthy();
    expect(screen.getByText("Taking work")).toBeTruthy();
    expect(screen.getByText("90% Not Slop")).toBeTruthy();
  });

  it("explains why someone can't be hired", () => {
    render(
      <MemoryRouter>
        <PersonCard c={{ ...card, can_hire: false }} />
      </MemoryRouter>,
    );
    expect(screen.getByText("Not taking paid work yet")).toBeTruthy();
  });
});

describe("RatingLine", () => {
  afterEach(cleanup);

  it("waits for enough reviews", () => {
    render(<RatingLine rating={{ not_slop: 1, slop: 0, percent: null, label: null, min_reviews: 3 }} clients={1} />);
    expect(screen.getByText(/1 review · 1 verified client/)).toBeTruthy();
  });
});
