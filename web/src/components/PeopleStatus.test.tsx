// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { CreativePage, PageStatus, PreviewSet, Work } from "../api";
import { LiveBar, PreviewBar, ownPageView } from "./PeopleStatus";

const work = (title: string): Work => ({
  key: `image:${title}.png`,
  kind: "image",
  url: `https://raw.example/${title}.png`,
  width: 64,
  height: 64,
  youtube_id: null,
  audio_url: null,
  audio_format: null,
  title,
  caption: null,
  skill: null,
});

const pending: PreviewSet = {
  status: "pending",
  status_reason: null,
  created_at: "2026-10-20T10:00:00Z",
  reviewed_at: null,
  avatar_url: "https://raw.example/new-avatar.png",
  banner_url: "https://raw.example/new-banner.png",
  design: { tree: [{ tag: "h2", children: ["New look"] }], css: "" },
  works: [work("Knight"), work("Dragon")],
};
const rejected: PreviewSet = { ...pending, status: "rejected", status_reason: "The dragon is someone else's art.", reviewed_at: "2026-10-21T10:00:00Z" };

// Only the fields the preview changes or reads; the rest of the page doesn't matter here.
const page = {
  handle: "vera",
  name: "Vera",
  is_me: true,
  avatar_url: "https://raw.example/avatar.png",
  banner_url: null,
  design: null,
  works: [work("Knight")],
  preview_set: pending,
} as unknown as CreativePage;

const status = (state: PageStatus["state"]): PageStatus => ({
  state,
  reason: null,
  update_waiting: state === "live",
  waiting_since: null,
  problem: null,
  suspended: false,
});

describe("ownPageView", () => {
  it("shows the creative their newer portfolio, as a preview", () => {
    const { page: shown, view } = ownPageView(page, false);
    expect(view).toBe("preview");
    expect(shown.works.map((w) => w.title)).toEqual(["Knight", "Dragon"]);
    expect(shown.avatar_url).toBe(pending.avatar_url);
    expect(shown.banner_url).toBe(pending.banner_url);
    expect(shown.design).toBe(pending.design);
    // Their text isn't reviewed: it's the same in both.
    expect(shown.name).toBe("Vera");
  });

  it("shows exactly the live page with ?view=live", () => {
    const { page: shown, view } = ownPageView(page, true);
    expect(view).toBe("live");
    expect(shown).toBe(page);
  });

  it("is just the page for anyone else, or with nothing newer", () => {
    // The server never sends anyone else a preview; even if it did, it isn't drawn.
    expect(ownPageView({ ...page, is_me: false }, false)).toEqual({ page: { ...page, is_me: false }, view: null });
    const noPreview = { ...page, preview_set: null };
    expect(ownPageView(noPreview, false)).toEqual({ page: noPreview, view: null });
    expect(ownPageView(noPreview, true).view).toBeNull();
  });
});

describe("PreviewBar", () => {
  afterEach(cleanup);
  const bar = (s: PageStatus, set: PreviewSet) =>
    render(
      <MemoryRouter>
        <PreviewBar handle="vera" status={s} set={set} />
      </MemoryRouter>,
    );

  it("says it's a preview only they see, and links to the live page", () => {
    bar(status("live"), pending);
    const region = screen.getByRole("region", { name: "Preview: only you can see this" });
    expect(region.textContent).toContain("Preview Only you can see this.");
    expect(region.textContent).toContain("Your changes are waiting for SLAPorSLOP's review; everyone else sees your live page.");
    expect(region.textContent).toContain("Waiting for review since");
    expect(within(region).getByRole("link", { name: "View your live page" }).getAttribute("href")).toBe("/people/vera?view=live");
  });

  it("says there's no live page yet instead of linking to one", () => {
    bar(status("waiting"), pending);
    const region = screen.getByRole("region", { name: "Preview: only you can see this" });
    expect(within(region).queryByRole("link", { name: "View your live page" })).toBeNull();
    expect(region.textContent).toContain("Your page isn't live yet: there's no live page to see.");
    expect(region.textContent).toContain("your page isn't public yet, so nobody else can see it.");
  });

  it("gives the reviewer's reason and what to do when it was turned down", () => {
    bar(status("live"), rejected);
    const region = screen.getByRole("region", { name: "Preview: only you can see this" });
    expect(region.textContent).toContain("SLAPorSLOP turned these changes down; everyone else sees your live page.");
    expect(region.textContent).toContain("The reviewer said: The dragon is someone else's art.");
    expect(region.textContent).toContain("Check my portfolio now");
    expect(within(region).getByRole("link", { name: "Fix your portfolio" }).getAttribute("href")).toBe("/people/me#portfolio");
    expect(within(region).getByRole("link", { name: "View your live page" })).toBeTruthy();
  });

  it("doesn't promise a live page while the page is hidden", () => {
    bar(status("hidden"), pending);
    const region = screen.getByRole("region", { name: "Preview: only you can see this" });
    expect(region.textContent).toContain("Your page is hidden, so nobody else can see it.");
    expect(within(region).queryByRole("link", { name: "View your live page" })).toBeNull();
  });
});

describe("LiveBar", () => {
  afterEach(cleanup);

  it("says this is what everyone sees, with the way back to the preview", () => {
    render(
      <MemoryRouter>
        <LiveBar handle="vera" status={status("live")} set={pending} />
      </MemoryRouter>,
    );
    const region = screen.getByRole("region", { name: "Your live page" });
    expect(region.textContent).toContain("This is your live page, as everyone sees it.");
    expect(region.textContent).toContain("only you can see it, in the preview");
    expect(within(region).getByRole("link", { name: "Back to your preview" }).getAttribute("href")).toBe("/people/vera");
  });

  it("says when nobody can see it yet", () => {
    render(
      <MemoryRouter>
        <LiveBar handle="vera" status={status("waiting")} set={pending} />
      </MemoryRouter>,
    );
    expect(screen.getByRole("region", { name: "Your live page" }).textContent).toContain("Your page isn't live yet: nobody else can see it.");
  });
});
