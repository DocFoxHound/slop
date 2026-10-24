// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { Avatar, initial } from "./Avatar";

describe("Avatar", () => {
  afterEach(cleanup);

  it("shows the picture when there is one, hidden from screen readers (the name is beside it)", () => {
    const { container } = render(<Avatar name="Ann" url="https://slop.example/v1/users/1/avatar?v=ab" />);
    const box = container.firstElementChild!;
    expect(box.className).toBe("avatar");
    expect(box.getAttribute("aria-hidden")).toBe("true");
    const img = box.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("https://slop.example/v1/users/1/avatar?v=ab");
    expect(img.getAttribute("alt")).toBe("");
    expect(box.textContent).toBe("");
  });

  it("shows the letter without one, and when the picture doesn't load", () => {
    const { container, rerender } = render(<Avatar name="  zed" url={null} className="avatar big" />);
    expect(container.firstElementChild!.className).toBe("avatar big");
    expect(container.textContent).toBe("Z");
    expect(container.querySelector("img")).toBeNull();

    rerender(<Avatar name="zed" url="https://slop.example/gone.png" className="avatar big" />);
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe("Z");

    // A new picture is tried again.
    rerender(<Avatar name="zed" url="https://slop.example/new.png" className="avatar big" />);
    expect(container.querySelector("img")!.getAttribute("src")).toBe("https://slop.example/new.png");
  });

  it("never draws an empty square", () => {
    expect(initial("   ")).toBe("?");
    expect(initial("éva")).toBe("É");
  });
});
