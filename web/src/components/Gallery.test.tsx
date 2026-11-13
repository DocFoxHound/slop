// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ desktop: false, openExternal: vi.fn() }));
vi.mock("../platform", () => ({
  get isDesktop() {
    return platform.desktop;
  },
  openExternal: platform.openExternal,
}));

import type { GalleryItem } from "../api";
import { Gallery } from "./Gallery";

// jsdom has no modal dialogs: here showModal only opens one.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};

const items: GalleryItem[] = [
  { id: "a", kind: "image", url: "https://slop.test/v1/media/a", youtube_id: null, caption: "Level one", width: 1920, height: 1080 },
  { id: "b", kind: "video", url: "https://slop.test/v1/media/b", youtube_id: "dQw4w9WgXcQ", caption: null, width: 480, height: 360 },
  { id: "c", kind: "image", url: "https://slop.test/v1/media/c", youtube_id: null, caption: null, width: 1280, height: 720 },
];
const gallery = () => screen.getByRole("region", { name: "Dungeon: screenshots and videos" });
const viewer = () => screen.getByRole("dialog", { name: "Dungeon: screenshots and videos" });

beforeEach(() => {
  platform.desktop = false;
  platform.openExternal.mockClear();
});
afterEach(cleanup);

describe("Gallery", () => {
  it("steps through items, and the thumbnails only choose what the stage shows", () => {
    const { container } = render(<Gallery items={items} title="Dungeon" />);
    expect(within(gallery()).getByAltText("Level one")).toBeTruthy();
    fireEvent.click(within(gallery()).getByLabelText("Next"));
    expect(within(gallery()).getByRole("button", { name: "Play video: Video 2 of 3. Plays from YouTube" })).toBeTruthy();
    fireEvent.click(within(gallery()).getByRole("tab", { name: "Screenshot 3 of 3" }));
    expect(within(gallery()).getByAltText("Screenshot 3 of 3")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
  });

  it("the big picture opens full size; the stage then shows the item last viewed, and has focus", () => {
    render(<Gallery items={items} title="Dungeon" />);
    const zoom = within(gallery()).getByRole("button", { name: "View full size: Level one" });
    expect(zoom.getAttribute("aria-haspopup")).toBe("dialog");
    zoom.focus();
    fireEvent.click(zoom);
    const d = viewer();
    expect(within(d).getByRole("img", { name: "Level one" })).toBeTruthy();
    expect(d.querySelector(".viewer-count")?.textContent).toBe("1 of 3");
    // Stepping onto the video shows its poster; nothing loads from YouTube.
    fireEvent.click(within(d).getByRole("button", { name: "Previous" }));
    expect(within(d).getByRole("img", { name: "Screenshot 3 of 3" })).toBeTruthy();
    fireEvent.click(within(d).getByRole("button", { name: "Previous" }));
    expect(within(d).getByRole("button", { name: "Play video: Video 2 of 3. Plays from YouTube" })).toBeTruthy();
    expect(d.querySelector("iframe")).toBeNull();

    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    const stage = within(gallery()).getByRole("button", { name: "Play video: Video 2 of 3. Plays from YouTube" });
    expect(document.activeElement).toBe(stage);
    expect(within(gallery()).getByRole("tab", { name: "Video 2 of 3" }).getAttribute("aria-selected")).toBe("true");
  });

  it("pressing play on a video opens it full size and plays it there; YouTube loads only then", () => {
    const { container } = render(<Gallery items={items} title="Dungeon" />);
    fireEvent.click(within(gallery()).getByLabelText("Next"));
    expect(container.querySelector("iframe")).toBeNull();
    const play = within(gallery()).getByRole("button", { name: "Play video: Video 2 of 3. Plays from YouTube" });
    play.focus();
    fireEvent.click(play);
    const frame = viewer().querySelector("iframe");
    expect(frame?.getAttribute("src")).toMatch(/^https:\/\/www\.youtube-nocookie\.com\/embed\/dQw4w9WgXcQ\?/);

    // Moving on stops the video; closing goes back to the stage at the item last viewed.
    fireEvent.click(within(viewer()).getByRole("button", { name: "Next" }));
    expect(container.querySelector("iframe")).toBeNull();
    fireEvent.click(within(viewer()).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    const stage = within(gallery()).getByRole("button", { name: "View full size: Screenshot 3 of 3" });
    expect(document.activeElement).toBe(stage);
    expect(platform.openExternal).not.toHaveBeenCalled();
  });

  it("shows nothing without items", () => {
    const { container } = render(<Gallery items={[]} title="Empty" />);
    expect(container.innerHTML).toBe("");
  });
});

describe("Gallery in the desktop app", () => {
  it("a video opens on YouTube in the browser, never in a frame; pictures still open full size", () => {
    platform.desktop = true;
    const { container } = render(<Gallery items={items} title="Dungeon" />);
    fireEvent.click(within(gallery()).getByLabelText("Next"));
    expect(within(gallery()).getByText("Opens on YouTube")).toBeTruthy();
    fireEvent.click(within(gallery()).getByRole("button", { name: "Play video: Video 2 of 3. Opens on YouTube" }));
    expect(platform.openExternal).toHaveBeenCalledWith("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();

    fireEvent.click(within(gallery()).getByLabelText("Next"));
    fireEvent.click(within(gallery()).getByRole("button", { name: "View full size: Screenshot 3 of 3" }));
    expect(within(viewer()).getByRole("img", { name: "Screenshot 3 of 3" })).toBeTruthy();
  });
});

describe("Gallery videos without a poster", () => {
  it("shows a plain card until play", () => {
    const { container } = render(
      <Gallery
        items={[{ id: "v", kind: "video", url: null, youtube_id: "dQw4w9WgXcQ", caption: "Trailer", width: null, height: null }]}
        title="Dungeon"
      />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector(".play-card")?.textContent).toBe("Trailer");
  });

  it("the Play button's name holds the words it shows (WCAG 2.5.3)", () => {
    render(
      <Gallery
        items={[
          { id: "a", kind: "image", url: "https://slop.test/v1/media/a", youtube_id: null, caption: null, width: 1280, height: 720 },
          { id: "v", kind: "video", url: null, youtube_id: "dQw4w9WgXcQ", caption: null, width: null, height: null },
        ]}
        title="Dungeon"
      />,
    );
    fireEvent.click(within(gallery()).getByLabelText("Next"));
    const play = within(gallery()).getByRole("button", { name: "Play video: Video 2 of 2. Plays from YouTube" });
    expect(play.querySelector(".play-card")?.textContent).toBe("Video 2 of 2");
    expect(play.querySelector(".play-note")?.textContent).toBe("Plays from YouTube");
  });
});
