// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { HotApp } from "../api";
import { HotReel, hotLine, ROTATE_MS } from "./HotReel";

function hot(slug: string, title: string, extra: Partial<HotApp> = {}): HotApp {
  return {
    slug,
    title,
    summary: `${title} is taking off.`,
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
    updated_at: "2026-11-19T00:00:00Z",
    adds_week: 12,
    slaps_week: 5,
    players_week: 0,
    ...extra,
  };
}

const apps = [hot("a", "Alpha"), hot("b", "Bravo"), hot("c", "Charlie")];

/** The slide showing now, by its label ("2 of 3: Bravo"). */
const slide = () => screen.getByRole("group").getAttribute("aria-label");
const live = () => screen.getByRole("group").parentElement?.getAttribute("aria-live");
const wait = (ms: number) => act(async () => vi.advanceTimersByTime(ms));

function show(list: HotApp[] = apps) {
  return render(
    <MemoryRouter>
      <HotReel apps={list} />
    </MemoryRouter>,
  );
}

/** The person's reduced-motion setting, as `matchMedia` reports it. */
function reducedMotion(on: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: on && query.includes("prefers-reduced-motion"),
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
}

describe("HotReel", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    reducedMotion(false);
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("is a labelled carousel that moves on by itself, with nothing announced while it moves", async () => {
    show();
    const reel = screen.getByRole("region", { name: "Hot this week" });
    expect(reel.getAttribute("aria-roledescription")).toBe("carousel");
    expect(screen.getByRole("group").getAttribute("aria-roledescription")).toBe("slide");
    expect(slide()).toBe("1 of 3: Alpha");
    expect(live()).toBe("off");
    await wait(ROTATE_MS - 1);
    expect(slide()).toBe("1 of 3: Alpha");
    await wait(1);
    expect(slide()).toBe("2 of 3: Bravo");
    await wait(ROTATE_MS);
    await wait(ROTATE_MS);
    expect(slide()).toBe("1 of 3: Alpha");
  });

  it("stops on Pause and starts again on Play; still, it's announced", async () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: /Pause/ }));
    expect(live()).toBe("polite");
    await wait(ROTATE_MS * 3);
    expect(slide()).toBe("1 of 3: Alpha");
    fireEvent.click(screen.getByRole("button", { name: /Next hot app/ }));
    expect(slide()).toBe("2 of 3: Bravo");
    fireEvent.click(screen.getByRole("button", { name: /Play/ }));
    // Focus on Play itself doesn't hold it still.
    act(() => screen.getByRole("button", { name: /Pause/ }).focus());
    expect(live()).toBe("off");
    await wait(ROTATE_MS);
    expect(slide()).toBe("3 of 3: Charlie");
  });

  it("waits while the pointer is over it", async () => {
    show();
    const reel = screen.getByRole("region", { name: "Hot this week" });
    fireEvent.mouseEnter(reel);
    await wait(ROTATE_MS * 2);
    expect(slide()).toBe("1 of 3: Alpha");
    fireEvent.mouseLeave(reel);
    await wait(ROTATE_MS);
    expect(slide()).toBe("2 of 3: Bravo");
  });

  it("waits while keyboard focus is in it, and Previous and Next page through it", async () => {
    show();
    const previous = screen.getByRole("button", { name: "Previous hot app" });
    act(() => previous.focus());
    fireEvent.click(previous);
    expect(slide()).toBe("3 of 3: Charlie");
    expect(live()).toBe("polite");
    await wait(ROTATE_MS * 2);
    expect(slide()).toBe("3 of 3: Charlie");
    act(() => previous.blur());
    await wait(ROTATE_MS);
    expect(slide()).toBe("1 of 3: Alpha");
  });

  it("never moves by itself with reduced motion, and has no Pause then", async () => {
    reducedMotion(true);
    show();
    expect(screen.queryByRole("button", { name: /Pause/ })).toBeNull();
    await wait(ROTATE_MS * 3);
    expect(slide()).toBe("1 of 3: Alpha");
    expect(live()).toBe("polite");
    fireEvent.click(screen.getByRole("button", { name: "Next hot app" }));
    expect(slide()).toBe("2 of 3: Bravo");
  });

  it("shows nothing when nothing is hot, and one app without controls", () => {
    const { container } = show([]);
    expect(container.innerHTML).toBe("");
    cleanup();
    show([apps[0]]);
    expect(screen.queryByRole("button")).toBeNull();
    expect(slide()).toBe("1 of 1: Alpha");
  });
});

describe("hotLine", () => {
  it("says this week's numbers, leaving out what's zero", () => {
    expect(hotLine({ adds_week: 12, slaps_week: 5, players_week: 30 })).toBe(
      "This week: 12 library adds · 5 Slaps · up to 30 playing at once",
    );
    expect(hotLine({ adds_week: 1, slaps_week: 0, players_week: 0 })).toBe("This week: 1 library add");
    expect(hotLine({ adds_week: 0, slaps_week: 0, players_week: 0 })).toBe("");
  });
});
