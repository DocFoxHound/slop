// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { Category } from "../api";
import { storeSection } from "./StoreNav";

const cat = (slug: string, tags: string[]): Category =>
  ({ slug, name: slug, description: "", tags: tags.map((t) => ({ slug: t, name: t })) }) as unknown as Category;
const categories = [cat("games", ["puzzle", "racing"]), cat("family", ["kids"]), cat("utilities", ["files"]), cat("tools", ["editor"])];

describe("storeSection", () => {
  it("marks the bar's own sections", () => {
    expect(storeSection("/browse", "", categories)).toBe("browse");
    expect(storeSection("/discover", "", categories)).toBe("discover");
    // The tab's old address, on its way to the new one.
    expect(storeSection("/recommended", "", categories)).toBe("discover");
    expect(storeSection("/browse/games", "", categories)).toBe("games");
    expect(storeSection("/browse/family", "", categories)).toBe("family");
    expect(storeSection("/browse/utilities", "", categories)).toBe("utilities");
    // Everything in a category is still in its section.
    expect(storeSection("/browse/games/all", "", categories)).toBe("games");
    expect(storeSection("/browse/family/all", "", categories)).toBe("family");
  });

  it("puts the other categories and the rest of the store under More", () => {
    expect(storeSection("/browse/tools", "", categories)).toBe("more");
    expect(storeSection("/browse/learning", "", categories)).toBe("more");
    expect(storeSection("/browse/tools/all", "", categories)).toBe("more");
    for (const p of ["/upcoming", "/made-with-people", "/community", "/teams"]) expect(storeSection(p, "", categories)).toBe("more");
    // "Everything" is search without words.
    expect(storeSection("/search", "", categories)).toBe("more");
    expect(storeSection("/search", "?sort=popular", categories)).toBe("more");
  });

  it("gives a tag its category's section, once the categories are known", () => {
    expect(storeSection("/tag/puzzle", "", categories)).toBe("games");
    expect(storeSection("/tag/files", "", categories)).toBe("utilities");
    expect(storeSection("/tag/kids", "", categories)).toBe("family");
    expect(storeSection("/tag/editor", "", categories)).toBe("more");
    expect(storeSection("/tag/puzzle", "", undefined)).toBeNull();
    expect(storeSection("/tag/unknown", "", categories)).toBeNull();
  });

  it("marks nothing on the front page, a search, or an app", () => {
    expect(storeSection("/", "", categories)).toBeNull();
    expect(storeSection("/search", "?q=castle", categories)).toBeNull();
    expect(storeSection("/app/castle-rock", "", categories)).toBeNull();
  });
});
