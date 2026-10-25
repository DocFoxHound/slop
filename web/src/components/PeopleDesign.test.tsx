// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, render, screen } from "@testing-library/react";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";

import type { DesignNode, PageDesign, Work } from "../api";
import { buildDesign, DesignArea, LINK_REL, portfolioWork, slopParts, type DesignParts } from "./PeopleDesign";

const parts: DesignParts = {
  works: () => <div data-part="works">All works</div>,
  work: (key) => (key === "image:works/a.png" ? <div data-part="work">Work A</div> : null),
  video: (id, title) => <div data-part="video">{`${id} ${title ?? ""}`}</div>,
  track: (src) => <div data-part="track">{src}</div>,
  services: () => <div data-part="services">Services</div>,
};

type El = ReactElement<Record<string, unknown> & { children?: ReactNode }>;

function el(node: ReactNode): El {
  if (!isValidElement(node)) throw new Error(`not an element: ${String(node)}`);
  return node as El;
}

describe("buildDesign", () => {
  it("draws only allowed elements and attributes, and links open elsewhere", () => {
    const tree: DesignNode[] = [
      { tag: "p", attrs: { class: "lede", lang: "pt", onclick: "alert(1)", style: "position:fixed" }, children: ["Olá"] },
      { tag: "a", attrs: { href: "https://example.com/", rel: "opener", target: "_self" }, children: ["Me"] },
      { tag: "a", attrs: { href: "javascript:alert(1)" }, children: ["Bad"] },
      { tag: "img", attrs: { src: "https://raw.githubusercontent.com/me/p/c/.slop/page/a.png", alt: "A", width: "64", height: "32" } },
      { tag: "img", attrs: { src: "page/a.png", alt: "Not linked" } },
      { tag: "script", children: ["alert(1)"] },
      { tag: "td", attrs: { colspan: "2", scope: "row" } },
    ];
    const { content, parts: slotted } = buildDesign(tree, parts);
    expect(slotted).toEqual([]);
    const [p, a, bad, img, unlinked, script, td] = content;
    expect(el(p).type).toBe("p");
    expect(el(p).props).toMatchObject({ className: "lede", lang: "pt" });
    expect(el(p).props.onclick).toBeUndefined();
    expect(el(p).props.style).toBeUndefined();
    expect(el(a).type).toBe("a");
    expect(el(a).props).toMatchObject({ href: "https://example.com/", target: "_blank", rel: LINK_REL });
    expect(el(bad).type).toBe("span");
    expect(el(bad).props.href).toBeUndefined();
    expect(el(img).props).toMatchObject({ alt: "A", width: 64, height: 32, loading: "lazy" });
    expect(unlinked).toBeNull();
    // Never drawn as itself: its text is only text.
    expect(el(script).type).toBe("span");
    expect(el(td).props).toMatchObject({ colSpan: 2 });
    expect(el(td).props.scope).toBeUndefined();
  });

  it("puts Slop's parts in slots, in order, and leaves out ones that aren't there", () => {
    const tree: DesignNode[] = [
      { tag: "section", children: [{ tag: "slop-works" }, { tag: "slop-work", attrs: { work: "image:works/a.png" } }] },
      { tag: "slop-work", attrs: { work: "image:works/missing.png" } },
      { tag: "slop-video", attrs: { youtube: "dQw4w9WgXcQ", title: "Reel" } },
      { tag: "slop-track", attrs: { src: "javascript:alert(1)" } },
      { tag: "slop-track", attrs: { src: "https://raw.githubusercontent.com/me/p/c/.slop/t.ogg" } },
      { tag: "slop-services" },
      { tag: "slop-unknown" },
    ];
    const { content, parts: slotted } = buildDesign(tree, parts);
    expect(slotted.map((p) => p.slot)).toEqual(["slop-part-0", "slop-part-1", "slop-part-2", "slop-part-3", "slop-part-4"]);
    expect(slotted.map((p) => el(p.node).props["data-part"])).toEqual(["works", "work", "video", "track", "services"]);
    const section = el(content[0]);
    const inside = section.props.children as ReactNode[];
    expect(inside.map((n) => [el(n).type, el(n).props.name])).toEqual([
      ["slot", "slop-part-0"],
      ["slot", "slop-part-1"],
    ]);
    expect(content.slice(1).map((n) => (n === null ? null : el(n).props.name))).toEqual([null, "slop-part-2", null, "slop-part-3", "slop-part-4", null]);
  });
});

describe("DesignArea", () => {
  afterEach(cleanup);

  const design: PageDesign = {
    tree: [
      { tag: "h2", children: ["Hi, I'm Vera"] },
      { tag: "div", attrs: { class: "wall" }, children: [{ tag: "slop-works" }] },
    ],
    css: ".wall{display:grid;}",
    has_services: false,
  };

  it("draws the design in a shadow root, in a labelled region, with Slop's parts slotted in", async () => {
    const works: Work[] = [
      { key: "image:a.png", kind: "image", url: "https://x/a.png", width: 64, height: 64, youtube_id: null, audio_url: null, audio_format: null, title: "Knight", caption: null, skill: null },
    ];
    await act(async () => {
      render(<DesignArea design={design} label="Vera's page" parts={slopParts({ works, services: () => null })} />);
    });
    const region = screen.getByRole("region", { name: "Vera's page" });
    const root = region.shadowRoot!;
    expect(root).toBeTruthy();
    expect(root.querySelector("style")!.textContent).toContain(".wall{display:grid;}");
    // People who ask for less motion get none, after the design's own rules.
    expect(root.querySelector("style")!.textContent!.indexOf("prefers-reduced-motion")).toBeGreaterThan(
      root.querySelector("style")!.textContent!.indexOf(".wall"),
    );
    expect(root.querySelector(".page h2")!.textContent).toBe("Hi, I'm Vera");
    const slot = root.querySelector(".wall slot")!;
    expect(slot.getAttribute("name")).toBe("slop-part-0");
    // The works are Slop's, outside the shadow root: Slop's styles and behaviour apply.
    const part = region.querySelector(":scope > [slot='slop-part-0']")!;
    expect(part.className).toBe("design-part");
    expect(part.querySelector("button[aria-label='View larger: Knight']")).toBeTruthy();
  });
});

describe("portfolioWork", () => {
  it("shows a set's works as the page does", () => {
    expect(
      portfolioWork({ kind: "audio", audio: { path: "t.ogg", url: "https://x/t.ogg", format: "ogg" }, poster: null, title: "Theme", caption: null }),
    ).toMatchObject({ key: "audio:t.ogg", kind: "audio", audio_url: "https://x/t.ogg", audio_format: "ogg", url: null, title: "Theme" });
  });
});
