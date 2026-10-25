// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { AppIcon, appIconProps, iconSources } from "./AppIcon";
import { cropSquare, iconState } from "./IconSlot";

describe("AppIcon", () => {
  afterEach(cleanup);

  it("shows the approved icon, square at the size asked, hidden from screen readers", () => {
    const { container } = render(
      <AppIcon title="Lantern Keep" iconUrl="https://raw.example/icon.png" thumbnailUrl="https://raw.example/thumb.png" size={48} />,
    );
    const box = container.firstElementChild as HTMLElement;
    expect(box.getAttribute("data-kind")).toBe("icon");
    expect(box.getAttribute("aria-hidden")).toBe("true");
    expect(box.style.width).toBe("48px");
    expect(box.style.height).toBe("48px");
    expect(box.querySelector("img")!.getAttribute("src")).toBe("https://raw.example/icon.png");
    expect(box.querySelector("img")!.getAttribute("alt")).toBe("");
  });

  it("falls back to a crop of the thumbnail, then the letter, also when a picture doesn't load", () => {
    const { container, rerender } = render(<AppIcon title="neon drift" thumbnailUrl="https://raw.example/thumb.png" />);
    const box = () => container.firstElementChild as HTMLElement;
    expect(box().getAttribute("data-kind")).toBe("crop");
    expect(box().style.width).toBe("32px");
    fireEvent.error(box().querySelector("img")!);
    expect(box().getAttribute("data-kind")).toBe("letter");
    expect(box().textContent).toBe("N");

    rerender(<AppIcon title="neon drift" iconUrl="https://raw.example/gone.png" thumbnailUrl="https://raw.example/thumb2.png" />);
    expect(box().getAttribute("data-kind")).toBe("icon");
    fireEvent.error(box().querySelector("img")!);
    expect(box().getAttribute("data-kind")).toBe("crop");
    expect(box().querySelector("img")!.getAttribute("src")).toBe("https://raw.example/thumb2.png");

    rerender(<AppIcon title="  " iconUrl={null} thumbnailUrl={null} />);
    expect(box().getAttribute("data-kind")).toBe("letter");
    expect(box().textContent).toBe("?");
    expect(box().style.background).toContain("linear-gradient");
  });

  it("takes an app summary's fields", () => {
    expect(appIconProps({ slug: "x", title: "X", icon_url: "i", thumbnail_url: null })).toEqual({
      title: "X",
      slug: "x",
      iconUrl: "i",
      thumbnailUrl: null,
    });
    expect(iconSources(null, "t")).toEqual([{ kind: "crop", url: "t" }]);
    expect(iconSources(undefined, undefined)).toEqual([]);
  });
});

describe("the Icon slot", () => {
  const pic = (sha256: string) => ({ path: "icon.png", url: `https://raw.example/${sha256}.png`, sha256, width: 512, height: 512, format: "png" });
  const set = (icon: ReturnType<typeof pic> | null) =>
    ({ icon }) as unknown as NonNullable<Parameters<typeof iconState>[0]["live"]>;

  it("says where the icon is", () => {
    expect(iconState({ live: null, pending: null, icon_problem: null })).toBe("missing");
    expect(iconState({ live: set(null), pending: null, icon_problem: null })).toBe("missing");
    expect(iconState({ live: set(pic("a")), pending: null, icon_problem: null })).toBe("approved");
    expect(iconState({ live: set(pic("a")), pending: set(pic("a")), icon_problem: null })).toBe("approved");
    expect(iconState({ live: set(pic("a")), pending: set(pic("b")), icon_problem: null })).toBe("pending");
    expect(iconState({ live: null, pending: set(pic("b")), icon_problem: null })).toBe("pending");
    expect(iconState({ live: set(pic("a")), pending: null, icon_problem: "icon.png is 600×300" })).toBe("problem");
  });

  it("crops the biggest square, where the maker slides it", () => {
    expect(cropSquare(920, 430, 0.5)).toEqual({ x: 245, y: 0, side: 430 });
    expect(cropSquare(920, 430, 0)).toEqual({ x: 0, y: 0, side: 430 });
    expect(cropSquare(920, 430, 1)).toEqual({ x: 490, y: 0, side: 430 });
    expect(cropSquare(400, 1000, 2)).toEqual({ x: 0, y: 600, side: 400 });
    expect(cropSquare(512, 512, 0.3)).toEqual({ x: 0, y: 0, side: 512 });
  });
});
