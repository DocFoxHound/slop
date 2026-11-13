// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ desktop: false, openExternal: vi.fn() }));
vi.mock("../platform", () => ({
  get isDesktop() {
    return platform.desktop;
  },
  openExternal: platform.openExternal,
}));

import { MediaViewer, type ViewerItem } from "./MediaViewer";

// jsdom has no modal dialogs: here showModal only opens one (the browser tests check the real thing).
const showModal = vi.fn(function (this: HTMLDialogElement) {
  this.open = true;
});
HTMLDialogElement.prototype.showModal = showModal;

const items: ViewerItem[] = [
  { id: "a", kind: "image", url: "https://slop.test/v1/media/a", label: "Level one", caption: "Level one", width: 1920 },
  { id: "b", kind: "image", url: "https://slop.test/v1/media/b", label: "Screenshot 2 of 3", width: 1280 },
  { id: "v", kind: "video", url: null, youtubeId: "dQw4w9WgXcQ", label: "Trailer", caption: "Trailer" },
];

/** A page with a button that opens the viewer, and what it was closed at. */
function Page({ list = items, start = 0, play = false }: { list?: ViewerItem[]; start?: number; play?: boolean }) {
  const [open, setOpen] = useState(false);
  const [last, setLast] = useState<number>();
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open
      </button>
      {last !== undefined && <p>Closed at {last}</p>}
      {open && (
        <MediaViewer
          items={list}
          start={start}
          play={play}
          label="Dungeon: screenshots and videos"
          onClose={(i) => {
            setLast(i);
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

const open = () => {
  const opener = screen.getByRole("button", { name: "Open" });
  opener.focus();
  fireEvent.click(opener);
  return screen.getByRole("dialog", { name: "Dungeon: screenshots and videos" });
};
const dialog = () => screen.queryByRole("dialog");
const count = (d: HTMLElement) => d.querySelector(".viewer-count")?.textContent;

beforeEach(() => {
  platform.desktop = false;
  platform.openExternal.mockClear();
  showModal.mockClear();
});
afterEach(cleanup);

describe("MediaViewer", () => {
  it("opens as a modal dialog at its item, focus inside, the page behind kept still", () => {
    render(<Page start={0} />);
    const d = open();
    expect(showModal).toHaveBeenCalledTimes(1);
    expect(d.tagName).toBe("DIALOG");
    expect(within(d).getByRole("img", { name: "Level one" }).getAttribute("src")).toBe("https://slop.test/v1/media/a");
    expect(count(d)).toBe("1 of 3");
    expect(d.querySelector(".viewer-caption")?.textContent).toBe("Level one");
    // Its description is the counter and the caption.
    expect(d.getAttribute("aria-describedby")?.split(" ")).toHaveLength(2);
    expect(document.activeElement).toBe(within(d).getByRole("button", { name: "Close" }));
    expect(document.documentElement.classList.contains("viewer-open")).toBe(true);
  });

  it("steps with Previous and Next, wrapping around both ways", () => {
    render(<Page />);
    const d = open();
    const next = within(d).getByRole("button", { name: "Next" });
    const prev = within(d).getByRole("button", { name: "Previous" });
    fireEvent.click(next);
    expect(count(d)).toBe("2 of 3");
    expect(within(d).getByRole("img", { name: "Screenshot 2 of 3" })).toBeTruthy();
    fireEvent.click(next);
    expect(count(d)).toBe("3 of 3");
    fireEvent.click(next);
    expect(count(d)).toBe("1 of 3");
    fireEvent.click(prev);
    expect(count(d)).toBe("3 of 3");
    expect(within(d).getByRole("button", { name: /^Play video: Trailer\. / })).toBeTruthy();
  });

  it("steps with the arrow keys, wrapping, and says each step politely", () => {
    render(<Page />);
    const d = open();
    const live = d.querySelector("[aria-live=polite]")!;
    // Opening says the dialog's name; nothing is announced yet.
    expect(live.textContent).toBe("");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("2 of 3");
    expect(live.textContent).toBe("2 of 3. Screenshot 2 of 3");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(count(d)).toBe("3 of 3");
    expect(live.textContent).toBe("3 of 3. Trailer");
    // Alt+Left is the browser's Back, not a step.
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft", altKey: true });
    expect(count(d)).toBe("3 of 3");
  });

  it("Escape closes, says which item was last, and focus goes back to what opened it", () => {
    render(<Page />);
    const d = open();
    fireEvent.click(within(d).getByRole("button", { name: "Next" }));
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(dialog()).toBeNull();
    expect(screen.getByText("Closed at 1")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Open" }));
    expect(document.documentElement.classList.contains("viewer-open")).toBe(false);
  });

  it("the browser's own close request (cancel) closes it too", () => {
    render(<Page />);
    const d = open();
    fireEvent(d, new Event("cancel", { cancelable: true }));
    expect(dialog()).toBeNull();
    expect(screen.getByText("Closed at 0")).toBeTruthy();
  });

  it("the Close button closes it", () => {
    render(<Page start={1} />);
    const d = open();
    fireEvent.click(within(d).getByRole("button", { name: "Close" }));
    expect(dialog()).toBeNull();
    expect(screen.getByText("Closed at 1")).toBeTruthy();
  });

  it("a click on the dark area around the picture closes it; on the picture or a control, it doesn't", () => {
    render(<Page />);
    const d = open();
    fireEvent.click(within(d).getByRole("img", { name: "Level one" }));
    fireEvent.click(d.querySelector(".viewer-count")!);
    expect(dialog()).not.toBeNull();
    fireEvent.click(d.querySelector(".viewer-frame")!);
    expect(dialog()).toBeNull();

    const again = open();
    fireEvent.click(again);
    expect(dialog()).toBeNull();
  });

  it("a sideways swipe steps; an up-and-down one doesn't", () => {
    render(<Page />);
    const d = open();
    const stage = d.querySelector(".viewer-stage")!;
    fireEvent.touchStart(stage, { touches: [{ clientX: 300, clientY: 200 }] });
    fireEvent.touchEnd(stage, { changedTouches: [{ clientX: 180, clientY: 210 }] });
    expect(count(d)).toBe("2 of 3");
    fireEvent.touchStart(stage, { touches: [{ clientX: 100, clientY: 200 }] });
    fireEvent.touchEnd(stage, { changedTouches: [{ clientX: 220, clientY: 200 }] });
    expect(count(d)).toBe("1 of 3");
    fireEvent.touchStart(stage, { touches: [{ clientX: 100, clientY: 100 }] });
    fireEvent.touchEnd(stage, { changedTouches: [{ clientX: 160, clientY: 400 }] });
    expect(count(d)).toBe("1 of 3");
  });

  it("a video loads nothing from YouTube until Play is pressed; stepping away removes its player", () => {
    const { container } = render(<Page start={2} />);
    const d = open();
    expect(container.querySelector("iframe")).toBeNull();
    expect(d.querySelector(".play-card")?.textContent).toBe("Trailer");
    expect(within(d).getByText("Plays from YouTube")).toBeTruthy();
    fireEvent.click(within(d).getByRole("button", { name: /^Play video: Trailer\. / }));
    const frame = d.querySelector("iframe");
    expect(frame?.getAttribute("src")).toMatch(/^https:\/\/www\.youtube-nocookie\.com\/embed\/dQw4w9WgXcQ\?/);
    expect(frame?.getAttribute("title")).toBe("Trailer");
    fireEvent.click(within(d).getByRole("button", { name: "Next" }));
    expect(container.querySelector("iframe")).toBeNull();
    // Back on it: the poster again, not the player.
    fireEvent.click(within(d).getByRole("button", { name: "Previous" }));
    expect(container.querySelector("iframe")).toBeNull();
    expect(platform.openExternal).not.toHaveBeenCalled();
  });

  it("opened by pressing play, the video plays at once; closing removes the player", () => {
    const { container } = render(<Page start={2} play />);
    open();
    expect(container.querySelector("iframe")?.getAttribute("src")).toMatch(/^https:\/\/www\.youtube-nocookie\.com\/embed\/dQw4w9WgXcQ\?autoplay=1/);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(container.querySelector("iframe")).toBeNull();
  });

  it("one item: no Previous or Next, and arrows do nothing", () => {
    render(<Page list={[items[0]]} />);
    const d = open();
    expect(within(d).queryByRole("button", { name: "Next" })).toBeNull();
    expect(within(d).queryByRole("button", { name: "Previous" })).toBeNull();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("1 of 1");
  });

  it("small pixel art keeps its sharp pixels, its box its own shape (so beside it still closes); other pictures don't", () => {
    render(<Page list={[{ ...items[0], width: 64, height: 32 }, items[1]]} />);
    const d = open();
    const sprite = within(d).getByRole("img", { name: "Level one" });
    expect(sprite.classList.contains("pixelated")).toBe(true);
    expect(sprite.style.aspectRatio).toBe("64 / 32");
    fireEvent.click(within(d).getByRole("button", { name: "Next" }));
    const shot = within(d).getByRole("img", { name: "Screenshot 2 of 3" });
    expect(shot.classList.contains("pixelated")).toBe(false);
    expect(shot.getAttribute("style")).toBeNull();
    fireEvent.click(d.querySelector(".viewer-frame")!);
    expect(dialog()).toBeNull();
  });

  it("with the Play button focused, the arrow keys keep stepping: focus stays in the viewer", () => {
    render(<Page list={[items[2], { ...items[2], id: "w", label: "Teaser", caption: "Teaser" }, items[0]]} />);
    const d = open();
    within(d).getByRole("button", { name: /^Play video: Trailer\. / }).focus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("2 of 3");
    // The next video's Play button has focus, where the last one was.
    expect(document.activeElement).toBe(within(d).getByRole("button", { name: /^Play video: Teaser\. / }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("3 of 3");
    // A picture has no Play button: Close has focus (it shows a focus ring), and the keys still step.
    expect(document.activeElement).toBe(within(d).getByRole("button", { name: "Close" }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("1 of 3");
  });

  it("pressing Play keeps focus in the viewer; the arrows then step and remove the player", () => {
    const { container } = render(<Page start={2} />);
    const d = open();
    const play = within(d).getByRole("button", { name: /^Play video: Trailer\. / });
    play.focus();
    fireEvent.click(play);
    expect(container.querySelector("iframe")).not.toBeNull();
    expect(d.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(count(d)).toBe("1 of 3");
    expect(container.querySelector("iframe")).toBeNull();
    // Keys reach it from anywhere in the page, even with focus lost to the body.
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document.body, { key: "ArrowLeft" });
    expect(count(d)).toBe("3 of 3");
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(dialog()).toBeNull();
  });

  it("a sideways drag while the page is pinch-zoomed pans; it doesn't step", () => {
    const viewport = Object.getOwnPropertyDescriptor(window, "visualViewport");
    Object.defineProperty(window, "visualViewport", { configurable: true, value: { scale: 2 } });
    try {
      render(<Page />);
      const d = open();
      const stage = d.querySelector(".viewer-stage")!;
      fireEvent.touchStart(stage, { touches: [{ clientX: 300, clientY: 200 }] });
      fireEvent.touchEnd(stage, { changedTouches: [{ clientX: 100, clientY: 200 }] });
      expect(count(d)).toBe("1 of 3");
    } finally {
      if (viewport) Object.defineProperty(window, "visualViewport", viewport);
      else delete (window as { visualViewport?: unknown }).visualViewport;
    }
  });

  it("a swipe the browser took over (touchcancel) doesn't step later", () => {
    render(<Page />);
    const d = open();
    const stage = d.querySelector(".viewer-stage")!;
    fireEvent.touchStart(stage, { touches: [{ clientX: 300, clientY: 200 }] });
    fireEvent.touchCancel(stage);
    fireEvent.touchEnd(stage, { changedTouches: [{ clientX: 100, clientY: 200 }] });
    expect(count(d)).toBe("1 of 3");
  });

  it("selecting the caption's words doesn't close it", () => {
    render(<Page />);
    const d = open();
    // A drag that began on the caption and ended on the dark area around it.
    fireEvent.pointerDown(d.querySelector(".viewer-caption")!);
    fireEvent.click(d.querySelector(".viewer-foot")!);
    expect(dialog()).not.toBeNull();
    // Text selected: not a close either.
    const selection = vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "Level" } as Selection);
    fireEvent.pointerDown(d.querySelector(".viewer-foot")!);
    fireEvent.click(d.querySelector(".viewer-foot")!);
    expect(dialog()).not.toBeNull();
    selection.mockRestore();
    // A plain click on the dark area does.
    fireEvent.pointerDown(d.querySelector(".viewer-foot")!);
    fireEvent.click(d.querySelector(".viewer-foot")!);
    expect(dialog()).toBeNull();
  });

  it("a caption that only repeats the picture's text alternative isn't read twice", () => {
    render(<Page />);
    const d = open();
    expect(d.querySelector(".viewer-caption")?.getAttribute("aria-hidden")).toBe("true");
    fireEvent.click(within(d).getByRole("button", { name: "Previous" }));
    // A video's caption is its only visible name: it's read.
    expect(d.querySelector(".viewer-caption")?.getAttribute("aria-hidden")).toBeNull();
  });

  it("with nothing to show it draws nothing and leaves the page alone", () => {
    render(<Page list={[]} />);
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(dialog()).toBeNull();
    expect(showModal).not.toHaveBeenCalled();
    expect(document.documentElement.classList.contains("viewer-open")).toBe(false);
  });
});

describe("MediaViewer in the desktop app", () => {
  it("never frames YouTube: Play opens the video in the browser and says so", () => {
    platform.desktop = true;
    const { container } = render(<Page start={2} play />);
    const d = open();
    expect(container.querySelector("iframe")).toBeNull();
    expect(within(d).getByText("Opens on YouTube")).toBeTruthy();
    fireEvent.click(within(d).getByRole("button", { name: /^Play video: Trailer\. / }));
    expect(platform.openExternal).toHaveBeenCalledWith("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    expect(container.querySelector("iframe")).toBeNull();
    // Pictures still open full size.
    fireEvent.click(within(d).getByRole("button", { name: "Next" }));
    expect(within(d).getByRole("img", { name: "Level one" })).toBeTruthy();
  });
});
