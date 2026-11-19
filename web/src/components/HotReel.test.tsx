// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { GalleryItem, HotApp } from "../api";
import { HotReel, hotLine, PICTURE_MS, ROTATE_MS } from "./HotReel";

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

const image = (url: string): GalleryItem => ({ id: url, kind: "image", url, youtube_id: null, caption: null, width: 920, height: 430 });
const withPictures = [
  hot("a", "Alpha", { gallery: [image("/m/a1.png"), image("/m/a2.png"), image("/m/a3.png")] }),
  hot("b", "Bravo", {
    gallery: [{ id: "v", kind: "video", url: "/m/b-poster.png", youtube_id: "abc", caption: null, width: 1280, height: 720 }],
  }),
];
/** The picture the shown slide has now. */
const picture = () => document.querySelector<HTMLImageElement>(".hot-art img")?.getAttribute("src");

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

  it("goes through the shown app's pictures every PICTURE_MS, each once before any comes again", async () => {
    show(withPictures);
    const seen = new Set<string>();
    for (let i = 0; i < 3; i++) {
      seen.add(picture()!);
      await wait(PICTURE_MS);
    }
    expect([...seen].sort()).toEqual(["/m/a1.png", "/m/a2.png", "/m/a3.png"]);
    expect(slide()).toBe("2 of 2: Bravo");
    // A video by its poster, marked as one.
    expect(picture()).toBe("/m/b-poster.png");
    expect(screen.getByText("▶ Video")).toBeTruthy();
  });

  it("fills a timer bar over each turn, held with the reel", async () => {
    show(withPictures);
    const bar = () => document.querySelector<HTMLElement>(".hot-timer")!;
    expect(bar().style.animationDuration).toBe(`${ROTATE_MS}ms`);
    expect(bar().style.animationPlayState).toBe("running");
    const reel = screen.getByRole("region", { name: "Hot this week" });
    await wait(ROTATE_MS - 2000);
    fireEvent.mouseEnter(reel);
    const held = picture();
    expect(bar().style.animationPlayState).toBe("paused");
    await wait(PICTURE_MS * 5);
    expect(picture()).toBe(held);
    expect(slide()).toBe("1 of 2: Alpha");
    // Carries on where it was: what's left of the turn, not a new one.
    fireEvent.mouseLeave(reel);
    await wait(2000);
    expect(slide()).toBe("2 of 2: Bravo");
  });

  it("holds still while it's off screen", async () => {
    const observers: Array<(entries: Array<{ isIntersecting: boolean }>) => void> = [];
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(cb: (entries: Array<{ isIntersecting: boolean }>) => void) {
          observers.push(cb);
        }
        observe() {}
        disconnect() {}
      },
    );
    try {
      show(withPictures);
      act(() => observers.forEach((cb) => cb([{ isIntersecting: false }])));
      const first = picture();
      await wait(ROTATE_MS * 2);
      expect(slide()).toBe("1 of 2: Alpha");
      expect(picture()).toBe(first);
      act(() => observers.forEach((cb) => cb([{ isIntersecting: true }])));
      await wait(ROTATE_MS);
      expect(slide()).toBe("2 of 2: Bravo");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("with reduced motion, shows one picture and no timer bar", async () => {
    reducedMotion(true);
    show(withPictures);
    const first = picture();
    await wait(PICTURE_MS * 4);
    expect(picture()).toBe(first);
    expect(document.querySelector(".hot-timer")).toBeNull();
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
