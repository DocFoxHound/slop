// SPDX-License-Identifier: AGPL-3.0-or-later
// The maker pages' tabs (MakerFrame.tsx): Manage | Edit store page | Releases | Builds | To fix,
// To fix's count (red when something must be fixed, amber when it only should be, none when
// clean, with a name that says it), and Builds marked while it's the next step.
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, type AppDetail, type AppFixes, type Fix, type ManageOverview } from "../api";
import { buildsNext, fixesBadge, MakerFrame, rememberSlopBuild } from "./MakerFrame";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const fix = (severity: Fix["severity"], kind = severity): Fix => ({
  kind,
  severity,
  problem: `A ${severity} problem.`,
  instruction: "Do this.",
});

const fixes = (over: Partial<AppFixes> = {}): AppFixes => ({
  release: { id: "r1", version: "1.2", status: "failed" },
  fixes: [],
  recommendations: [],
  next: "Then submit a new release.",
  ...over,
});

const makerApp = (over: Partial<AppDetail> = {}) =>
  ({
    slug: "harbour",
    title: "Harbour",
    stage: "coming_soon",
    status: "draft",
    private: false,
    plays_in_browser: false,
    platforms: ["windows"],
    repo_url: "https://github.com/mia/harbour",
    stage_entered_at: "2026-11-01T00:00:00Z",
    stage_deadline: null,
    interested: 0,
    release_date: null,
    release_date_precision: null,
    release_date_passed: false,
    icon_url: null,
    thumbnail_url: null,
    is_mine: true,
    ...over,
  }) as unknown as AppDetail;

describe("the To fix count", () => {
  it("counts must and should items, red when any must be fixed, and says so", () => {
    expect(fixesBadge(fixes({ fixes: [fix("must"), fix("must"), fix("should")] }))).toEqual({
      count: 3,
      tone: "bad",
      label: "To fix, 2 must be fixed, 1 should be fixed",
    });
    expect(fixesBadge(fixes({ fixes: [fix("should")] }))).toEqual({ count: 1, tone: "warn", label: "To fix, 1 should be fixed" });
  });

  it("is none when clean, whatever the suggestions, or before it's loaded", () => {
    expect(fixesBadge(fixes())).toBeNull();
    expect(fixesBadge(fixes({ recommendations: [fix("recommended", "icon")] }))).toBeNull();
    expect(fixesBadge(undefined)).toBeNull();
  });
});

describe("Builds as the next step", () => {
  const download = { slug: "harbour", repo_url: "https://github.com/mia/harbour", browser: false };
  it("is a download on GitHub with no release yet and no Slop Build found", () => {
    expect(buildsNext(download, { release: null })).toBe(true);
    expect(buildsNext(download, { release: { id: "r1", version: "1.0", status: "ready" } })).toBe(false);
    expect(buildsNext(download, { release: null }, true)).toBe(false);
    expect(buildsNext({ ...download, browser: true }, { release: null })).toBe(false);
    expect(buildsNext({ ...download, repo_url: "https://gitlab.com/mia/harbour" }, { release: null })).toBe(false);
    // Not known yet: no mark.
    expect(buildsNext(download, undefined)).toBe(false);
  });

  it("stops once the Builds page has found Slop Build, until it no longer does", () => {
    rememberSlopBuild("harbour", true);
    expect(buildsNext(download, { release: null })).toBe(false);
    rememberSlopBuild("harbour", false);
    expect(buildsNext(download, { release: null })).toBe(true);
  });
});

describe("the tabs", () => {
  beforeEach(() => {
    vi.spyOn(api, "manage").mockResolvedValue({ can_advance: false } as ManageOverview);
    vi.spyOn(api, "publishInvite").mockResolvedValue({ required: false, invited: false } as never);
  });

  const show = (app = makerApp(), props: Partial<Parameters<typeof MakerFrame>[0]> = {}) =>
    render(
      <MemoryRouter>
        <MakerFrame app={app} tab="releases" {...props}>
          <p>page</p>
        </MakerFrame>
      </MemoryRouter>,
    );
  const nav = () => screen.getByRole("navigation", { name: "Harbour: maker pages" });

  it("are Manage, Edit store page, Releases, Builds and To fix, each with its address", async () => {
    vi.spyOn(api, "fixes").mockResolvedValue(fixes());
    show();
    await waitFor(() => expect(api.fixes).toHaveBeenCalledTimes(1));
    const links = within(nav()).getAllByRole("link");
    expect(links.map((l) => [l.textContent, l.getAttribute("href")])).toEqual([
      ["Manage", "/publish/harbour/manage"],
      ["Edit store page", "/publish/harbour"],
      ["Releases", "/publish/harbour/releases"],
      ["Builds", "/publish/harbour/builds"],
      ["To fix", "/publish/harbour/fixes"],
    ]);
    expect(within(nav()).getByRole("link", { name: "Releases" }).getAttribute("aria-current")).toBe("page");
    // Clean: no count, and the plain name.
    expect(within(nav()).getByRole("link", { name: "To fix" }).querySelector(".tab-count")).toBeNull();
  });

  it("count what must be fixed in red, named with the count", async () => {
    vi.spyOn(api, "fixes").mockResolvedValue(fixes({ fixes: [fix("must"), fix("must")] }));
    show();
    const tab = await within(nav()).findByRole("link", { name: "To fix, 2 must be fixed" });
    const badge = tab.querySelector(".tab-count")!;
    expect(badge.className).toContain("bad");
    expect(badge.textContent).toBe("!2");
    expect(badge.getAttribute("aria-hidden")).toBe("true");
  });

  it("count what only should be fixed in amber, and suggestions not at all", async () => {
    vi.spyOn(api, "fixes").mockResolvedValue(fixes({ fixes: [fix("should")], recommendations: [fix("recommended", "icon")] }));
    show();
    const tab = await within(nav()).findByRole("link", { name: "To fix, 1 should be fixed" });
    const badge = tab.querySelector(".tab-count")!;
    expect(badge.className).toContain("warn");
    expect(badge.textContent).toBe("1");
  });

  it("mark Builds before the first release, and not once there is one", async () => {
    vi.spyOn(api, "fixes").mockResolvedValue(fixes({ release: null }));
    show();
    const tab = await within(nav()).findByRole("link", { name: "Builds, the next step" });
    expect(tab.querySelector(".tab-mark")).not.toBeNull();
    cleanup();
    vi.spyOn(api, "fixes").mockResolvedValue(fixes());
    show();
    await waitFor(() => expect(api.fixes).toHaveBeenCalled());
    expect(await within(nav()).findByRole("link", { name: "Builds" })).toBeTruthy();
    expect(nav().querySelector(".tab-mark")).toBeNull();
  });

  it("take the fixes from the page that has them, without asking again", async () => {
    const load = vi.spyOn(api, "fixes").mockResolvedValue(fixes());
    show(makerApp(), { tab: "fixes", fixes: fixes({ fixes: [fix("should")] }) });
    expect(within(nav()).getByRole("link", { name: "To fix, 1 should be fixed" }).getAttribute("aria-current")).toBe("page");
    await waitFor(() => expect(api.manage).toHaveBeenCalled());
    expect(load).not.toHaveBeenCalled();
  });

  it("have no Builds for a browser game: there's nothing to build", async () => {
    vi.spyOn(api, "fixes").mockResolvedValue(fixes({ release: null }));
    show(makerApp({ plays_in_browser: true, platforms: ["browser"] }));
    await waitFor(() => expect(api.fixes).toHaveBeenCalled());
    expect(within(nav()).queryByRole("link", { name: /^Builds/ })).toBeNull();
    expect(within(nav()).getAllByRole("link").map((l) => l.textContent)).toEqual(["Manage", "Edit store page", "Releases", "To fix"]);
  });
});
