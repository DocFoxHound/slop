import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../platform", () => ({ isDesktop: false, openExternal: vi.fn() }));

import type { GalleryItem } from "../api";
import { Gallery } from "./Gallery";

const items: GalleryItem[] = [
  { id: "a", kind: "image", url: "https://slop.test/v1/media/a", youtube_id: null, caption: "Level one", width: 1920, height: 1080 },
  { id: "b", kind: "video", url: "https://slop.test/v1/media/b", youtube_id: "dQw4w9WgXcQ", caption: null, width: 480, height: 360 },
];

describe("Gallery", () => {
  afterEach(cleanup);

  it("steps through items and loads YouTube only when play is pressed", () => {
    const { container } = render(<Gallery items={items} title="Dungeon" />);
    expect(screen.getByAltText("Level one")).toBeTruthy();
    expect(container.querySelector("iframe")).toBeNull();

    fireEvent.click(screen.getByLabelText("Next"));
    expect(container.querySelector("iframe")).toBeNull();
    fireEvent.click(screen.getByLabelText(/Play video/));
    const frame = container.querySelector("iframe");
    expect(frame?.getAttribute("src")).toMatch(/^https:\/\/www\.youtube-nocookie\.com\/embed\/dQw4w9WgXcQ\?/);

    // Moving on stops the video.
    fireEvent.click(screen.getByLabelText("Next"));
    expect(container.querySelector("iframe")).toBeNull();
    expect(screen.getByAltText("Level one")).toBeTruthy();
  });

  it("shows nothing without items", () => {
    const { container } = render(<Gallery items={[]} title="Empty" />);
    expect(container.innerHTML).toBe("");
  });
});

describe("Gallery videos without a poster", () => {
  afterEach(cleanup);

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
});
