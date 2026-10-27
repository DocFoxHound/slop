// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { browsingKey, displayAddress, fragmentId, furthest, isTyping, siteOrigin } from "./browsing";

describe("furthest", () => {
  it("drops what was ahead on a new page, keeps it going back and forth", () => {
    expect(furthest(0, 1, "PUSH")).toBe(1);
    expect(furthest(3, 1, "POP")).toBe(3);
    expect(furthest(3, 2, "PUSH")).toBe(2);
    expect(furthest(3, 3, "REPLACE")).toBe(3);
    // A reload further along than was known (another tab's session storage, say).
    expect(furthest(0, 4, "POP")).toBe(4);
  });
});

describe("siteOrigin", () => {
  it("is the page's own origin on the website", () => {
    expect(siteOrigin({ origin: "https://slaporslop.games", protocol: "https:", hostname: "slaporslop.games" }, "")).toBe("https://slaporslop.games");
    expect(siteOrigin({ origin: "http://localhost:5173", protocol: "http:", hostname: "localhost" }, "https://x.example/")).toBe("http://localhost:5173");
  });
  it("is the site the packaged desktop app was built for", () => {
    expect(siteOrigin({ origin: "tauri://localhost", protocol: "tauri:", hostname: "localhost" }, "https://test.slop.example/")).toBe("https://test.slop.example");
    expect(siteOrigin({ origin: "http://tauri.localhost", protocol: "http:", hostname: "tauri.localhost" }, "")).toBe("https://slaporslop.games");
    expect(siteOrigin({ origin: "http://tauri.localhost", protocol: "http:", hostname: "tauri.localhost" }, "not a url")).toBe("https://slaporslop.games");
  });
});

describe("displayAddress", () => {
  it("reads like an address bar", () => {
    expect(displayAddress("https://slaporslop.games", "/app/lantern-keep")).toBe("slaporslop.games/app/lantern-keep");
    expect(displayAddress("http://localhost:5173", "/search?q=caf%C3%A9")).toBe("localhost:5173/search?q=café");
    expect(displayAddress("https://slaporslop.games", "/bad%E0%A4%A")).toBe("slaporslop.games/bad%E0%A4%A");
  });
});

describe("browsingKey", () => {
  const key = (k: string, mods: Partial<Record<"altKey" | "ctrlKey" | "metaKey" | "shiftKey", boolean>> = {}) => ({
    key: k,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...mods,
  });
  it("knows the browser's shortcuts on Windows and Linux", () => {
    expect(browsingKey(key("ArrowLeft", { altKey: true }), false, false)).toBe("back");
    expect(browsingKey(key("ArrowRight", { altKey: true }), false, false)).toBe("forward");
    expect(browsingKey(key("F5"), false, false)).toBe("reload");
    expect(browsingKey(key("r", { ctrlKey: true }), false, false)).toBe("reload");
    expect(browsingKey(key("R", { ctrlKey: true, shiftKey: true }), false, false)).toBe("hard-reload");
    expect(browsingKey(key("F5", { ctrlKey: true }), false, false)).toBe("hard-reload");
    expect(browsingKey(key("ArrowLeft"), false, false)).toBeNull();
    expect(browsingKey(key("ArrowLeft", { altKey: true, shiftKey: true }), false, false)).toBeNull();
    expect(browsingKey(key("r"), false, false)).toBeNull();
    expect(browsingKey(key("[", { metaKey: true }), false, false)).toBeNull();
  });
  it("and on a Mac", () => {
    expect(browsingKey(key("[", { metaKey: true }), true, false)).toBe("back");
    expect(browsingKey(key("]", { metaKey: true }), true, false)).toBe("forward");
    expect(browsingKey(key("r", { metaKey: true }), true, false)).toBe("reload");
    expect(browsingKey(key("r", { ctrlKey: true }), true, false)).toBeNull();
    expect(browsingKey(key("ArrowLeft", { altKey: true }), true, false)).toBeNull();
  });
  it("leaves the arrows to text fields, but reloads from them", () => {
    expect(browsingKey(key("ArrowLeft", { altKey: true }), false, true)).toBeNull();
    expect(browsingKey(key("[", { metaKey: true }), true, true)).toBeNull();
    expect(browsingKey(key("F5"), false, true)).toBe("reload");
  });
});

describe("isTyping", () => {
  it("is text fields and editable areas", () => {
    const make = (html: string) => {
      const div = document.createElement("div");
      div.innerHTML = html;
      return div.firstElementChild;
    };
    expect(isTyping(make("<input>"))).toBe(true);
    expect(isTyping(make('<input type="search">'))).toBe(true);
    expect(isTyping(make('<input type="checkbox">'))).toBe(false);
    expect(isTyping(make("<textarea></textarea>"))).toBe(true);
    expect(isTyping(make("<button>Go</button>"))).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});

describe("fragmentId", () => {
  it("names the element a link points at", () => {
    expect(fragmentId("#decisions")).toBe("decisions");
    expect(fragmentId("#pictures-and-videos")).toBe("pictures-and-videos");
    expect(fragmentId("#caf%C3%A9")).toBe("café");
    expect(fragmentId("")).toBe("");
  });

  it("is empty for a handoff link's fill, or a fragment that can't be decoded", () => {
    expect(fragmentId("#fill=%7B%22title%22%3A%22Ferry%22%7D")).toBe("");
    expect(fragmentId("#%E0%A4%A")).toBe("");
  });
});
