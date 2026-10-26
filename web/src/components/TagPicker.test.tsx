// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";

import type { AppKind, Category, ContentLabel } from "../api";
import { categoryMismatches, MAX_TAGS, TagPicker, tagMatches } from "./TagPicker";

const cat = (slug: string, name: string, tags: string[], extra: Partial<Category> = {}): Category => ({
  slug,
  name,
  description: "",
  tags: tags.map((t) => ({ slug: t.toLowerCase().replace(/ /g, "-"), name: t })),
  ...extra,
});
const family = cat("family", "Kids & Family", ["Kids", "Family", "All Ages"], {
  max_age: 7,
  excluded_labels: ["suggestive", "horror", "strong_language", "gambling_themes"],
});
const categories = [
  cat("games", "Games", ["Action", "Open World", "Puzzle", "Racing", "Roguelike", "Strategy", "Tycoon", "Idle", "Arcade", "Rhythm", "Sports"]),
  family,
  cat("tools", "Tools", ["Note-taking", "Office"]),
  cat("vibe", "Vibe", ["Cozy", "Retro"]),
];

function Harness({ initial = [], kind = "game", minAge = 0, labels = [] }: { initial?: string[]; kind?: AppKind; minAge?: number; labels?: ContentLabel[] }) {
  const [tags, setTags] = useState(initial);
  return (
    <>
      <TagPicker categories={categories} value={tags} onChange={setTags} kind={kind} minAge={minAge} labels={labels} />
      <output data-testid="tags">{tags.join(",")}</output>
    </>
  );
}

const picked = () => screen.getByTestId("tags").textContent;
const search = () => screen.getByRole("searchbox", { name: "Find a tag" });
const group = (name: string) => screen.getByText(name, { selector: ".tag-group-name" }).closest("details")!;

describe("tagMatches", () => {
  it("finds a tag by its name or address, ignoring case", () => {
    const tag = { slug: "open-world", name: "Open World" };
    for (const q of ["open", "OPEN W", "world", "open-world", "  ", ""]) expect(tagMatches(tag, q)).toBe(true);
    expect(tagMatches(tag, "puzzle")).toBe(false);
  });
});

describe("categoryMismatches", () => {
  it("names Kids & Family tags on an app it won't list, and why", () => {
    expect(categoryMismatches(categories, ["kids", "puzzle"], 7, [])).toEqual([]);
    expect(categoryMismatches(categories, ["kids", "puzzle"], 0, ["violence", "romance"])).toEqual([]);
    const [old] = categoryMismatches(categories, ["kids", "family", "puzzle"], 12, []);
    expect(old.category.slug).toBe("family");
    expect(old.tags.map((t) => t.slug)).toEqual(["kids", "family"]);
    expect(old.tooOld).toBe(true);
    const [labelled] = categoryMismatches(categories, ["all-ages"], 3, ["horror", "violence"]);
    expect([labelled.tooOld, labelled.labels]).toEqual([false, ["horror"]]);
    // Categories without a limit never complain.
    expect(categoryMismatches(categories, ["puzzle", "cozy"], 18, ["horror"])).toEqual([]);
  });
});

describe("TagPicker", () => {
  afterEach(cleanup);

  it("groups tags by category; groups that rarely fit the type start closed", () => {
    render(<Harness />);
    expect(group("Games").open).toBe(true);
    expect(group("Kids & Family").open).toBe(true);
    expect(group("Tools").open).toBe(false);
    cleanup();
    // An app: games start closed, unless one of its tags is a game tag.
    render(<Harness kind="app" />);
    expect(group("Games").open).toBe(false);
    expect(group("Tools").open).toBe(true);
    cleanup();
    render(<Harness kind="app" initial={["arcade"]} />);
    expect(group("Games").open).toBe(true);
    expect(within(group("Games")).getByText("· Arcade")).toBeTruthy();
  });

  it("picks and unpicks, counts, and names what's picked in each group's summary", () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole("button", { name: "Puzzle" }));
    fireEvent.click(screen.getByRole("button", { name: "Cozy" }));
    expect(picked()).toBe("puzzle,cozy");
    expect(screen.getByRole("button", { name: "Puzzle" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("(2/10)")).toBeTruthy();
    expect(within(group("Vibe")).getByText("· Cozy")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Puzzle" }));
    expect(picked()).toBe("cozy");
  });

  it(`stops at ${MAX_TAGS}: the rest can't be picked until one is taken off`, () => {
    const ten = ["action", "open-world", "puzzle", "racing", "roguelike", "strategy", "tycoon", "idle", "arcade", "rhythm"];
    render(<Harness initial={ten} />);
    expect((screen.getByRole("button", { name: "Sports" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Rhythm" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Rhythm" }));
    expect((screen.getByRole("button", { name: "Sports" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("searching and clearing leaves closed groups closed", () => {
    render(<Harness />);
    fireEvent.change(search(), { target: { value: "office" } });
    expect(group("Tools").open).toBe(true);
    fireEvent.keyDown(search(), { key: "Escape" });
    expect(group("Tools").open).toBe(false);
  });

  it("searches across every group, opening the ones with matches; Enter picks the only match, Escape clears", () => {
    render(<Harness />);
    fireEvent.change(search(), { target: { value: "note" } });
    // Tools was closed; it opens for its match, and the groups without one go.
    expect(group("Tools").open).toBe(true);
    expect(screen.queryByText("Games", { selector: ".tag-group-name" })).toBeNull();
    expect(screen.getByText("1 tag matches.")).toBeTruthy();
    const enter = fireEvent.keyDown(search(), { key: "Enter" });
    expect(enter).toBe(false); // the form isn't sent
    expect(picked()).toBe("note-taking");

    fireEvent.change(search(), { target: { value: "zzz" } });
    expect(screen.getByText("No tag matches “zzz”.")).toBeTruthy();
    fireEvent.keyDown(search(), { key: "Enter" });
    expect(picked()).toBe("note-taking");

    fireEvent.keyDown(search(), { key: "Escape" });
    expect((search() as HTMLInputElement).value).toBe("");
    // Every group is back, and Tools stays open: the tag picked from the search is in it.
    expect(group("Games").open).toBe(true);
    expect(group("Tools").open).toBe(true);
    expect(within(group("Tools")).getByText("· Note-taking")).toBeTruthy();
  });

  it("warns, without refusing, when Kids & Family won't list the app", () => {
    const view = render(<Harness initial={["kids", "puzzle"]} minAge={12} />);
    expect(screen.getByText(/Kids & Family lists only apps rated 7\+ or under, without suggestive themes, horror, strong language or gambling themes\./)).toBeTruthy();
    expect(screen.getByText(/This one is rated 12\+, so Kids won't show it there\. You can still save/)).toBeTruthy();
    view.rerender(<TagPickerWith tags={["kids", "family"]} minAge={7} labels={["strong_language"]} />);
    expect(screen.getByText(/This one is labelled strong language, so Kids and Family won't show it there/)).toBeTruthy();
    view.rerender(<TagPickerWith tags={["kids"]} minAge={7} labels={["violence"]} />);
    expect(screen.queryByText(/won't show it there/)).toBeNull();
  });
});

function TagPickerWith({ tags, minAge, labels }: { tags: string[]; minAge: number; labels: ContentLabel[] }) {
  return <TagPicker categories={categories} value={tags} onChange={() => {}} kind="game" minAge={minAge} labels={labels} />;
}
