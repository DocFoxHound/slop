// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { CreativeCard, Work } from "../api";
import { PersonCard, RatingLine, WorkTile } from "./People";

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

describe("WorkTile", () => {
  afterEach(cleanup);
  const work: Work = {
    key: "video:abc",
    kind: "video",
    url: null,
    width: null,
    height: null,
    youtube_id: "dQw4w9WgXcQ",
    audio_url: null,
    audio_format: null,
    title: "Showreel",
    caption: "Two minutes of sprites",
    skill: { slug: "animation", name: "Animation" },
  };

  it("plays a portfolio video from YouTube (without cookies) only when asked", () => {
    const { container } = render(<WorkTile w={work} />);
    expect(container.querySelector("iframe")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Play video: Showreel" }));
    const frame = container.querySelector("iframe");
    expect(frame?.getAttribute("src")).toBe("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&rel=0");
    expect(frame?.getAttribute("title")).toBe("Showreel");
    expect(screen.getByText("Animation")).toBeTruthy();
  });

  it("gives a track a player that fetches nothing until pressed", () => {
    const { container } = render(<WorkTile w={{ ...work, kind: "audio", youtube_id: null, audio_url: "https://raw.example/theme.ogg", title: "Theme" }} />);
    const audio = container.querySelector("audio");
    expect(audio?.getAttribute("preload")).toBe("none");
    expect(audio?.getAttribute("aria-label")).toBe("Play: Theme");
  });
});
