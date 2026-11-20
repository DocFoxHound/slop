// SPDX-License-Identifier: AGPL-3.0-or-later
// The Builds tab (Builds.tsx): Slop Build's file to add, or "Slop Build is set up" once the
// repository has it (the owner's report, 2026-11-20: offered again, GitHub refused the name),
// another name when one of the maker's own is in the way, Slop Check likewise, and nothing to
// build for a browser game.
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type AppDetail, type AppFixes, type ManageOverview } from "../api";
import { rememberSlopBuild } from "../components/MakerFrame";
import { Builds, type SlopBuildSetupInfo } from "./Builds";

vi.mock("../session", () => ({ useSession: () => ({ me: { id: "mia" } }) }));

const app = (over: Partial<AppDetail> = {}) =>
  ({
    slug: "harbour",
    title: "Harbour",
    stage: "coming_soon",
    status: "draft",
    private: false,
    plays_in_browser: false,
    platforms: ["windows", "linux"],
    repo_url: "https://github.com/mia/harbour",
    stage_entered_at: "2026-11-01T00:00:00Z",
    stage_deadline: null,
    interested: 0,
    release_date: null,
    release_date_precision: null,
    release_date_passed: false,
    is_mine: true,
    ...over,
  }) as unknown as AppDetail;

const setup = (over: Partial<SlopBuildSetupInfo> = {}): SlopBuildSetupInfo => ({
  host: "github",
  checks_run_on: "github_actions",
  supported: true,
  project_type: "rust",
  detected: null,
  note: null,
  file_path: ".github/workflows/slop.yml",
  workflow: "name: Slop Build\non: push",
  create_url: "https://github.com/mia/harbour/new/main?filename=.github%2Fworkflows%2Fslop.yml",
  check_file_path: ".github/workflows/slop-check.yml",
  check_workflow: "name: Slop Check",
  check_create_url: "https://github.com/mia/harbour/new/main?filename=.github%2Fworkflows%2Fslop-check.yml",
  build_now_url: "https://github.com/mia/harbour/actions/workflows/slop.yml",
  existing: null,
  check_existing: null,
  taken: null,
  check_taken: null,
  workflows_checked: true,
  ...over,
});

const noRelease: AppFixes = { release: null, fixes: [], recommendations: [], next: "" };
const slopYml = { path: ".github/workflows/slop.yml", html_url: "https://github.com/mia/harbour/blob/main/.github/workflows/slop.yml" };

beforeEach(() => {
  rememberSlopBuild("harbour", false);
  vi.spyOn(api, "manage").mockResolvedValue({ can_advance: false } as ManageOverview);
  vi.spyOn(api, "publishInvite").mockResolvedValue({ required: false, invited: false } as never);
  vi.spyOn(api, "fixes").mockResolvedValue(noRelease);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function show(answer: SlopBuildSetupInfo | ApiError, detail = app(), at = "/publish/harbour/builds") {
  vi.spyOn(api, "app").mockResolvedValue(detail);
  const load = vi.spyOn(api, "slopBuildSetup");
  if (answer instanceof ApiError) load.mockRejectedValue(answer);
  else load.mockResolvedValue(answer);
  render(
    <MemoryRouter initialEntries={[at]}>
      <Routes>
        <Route path="/publish/:slug/builds" element={<Builds />} />
      </Routes>
    </MemoryRouter>,
  );
  return load;
}
const nav = () => screen.getByRole("navigation", { name: "Harbour: maker pages" });

describe("Builds", () => {
  it("offers Slop Build's file when it isn't there, and marks the tab as the next step", async () => {
    show(setup());
    const panel = await screen.findByRole("region", { name: "Let Slop Build do it" });
    await within(panel).findByText((_, el) => el?.textContent === "Detected a Rust project.");
    expect(within(panel).getByRole("link", { name: "Add to my repository on GitHub" }).getAttribute("href")).toBe(setup().create_url);
    expect(within(panel).getByRole("link", { name: "Build now" })).toBeTruthy();
    expect(panel.textContent).toContain("name: Slop Build");
    expect(await within(nav()).findByRole("link", { name: "Builds, the next step" })).toBeTruthy();
  });

  it("says Slop Build is set up when the repository has it, with Build now and nothing to add", async () => {
    show(setup({ existing: slopYml, create_url: null }));
    const panel = await screen.findByRole("region", { name: "Slop Build" });
    expect(panel.textContent).toContain("Slop Build is set up: .github/workflows/slop.yml.");
    expect(within(panel).getByRole("link", { name: ".github/workflows/slop.yml" }).getAttribute("href")).toBe(slopYml.html_url);
    expect(within(panel).getByRole("link", { name: "Build now" }).getAttribute("href")).toBe(setup().build_now_url);
    expect(within(panel).queryByRole("link", { name: "Add to my repository on GitHub" })).toBeNull();
    // The suggestion is there to compare, folded away.
    const compare = within(panel).getByText("See the suggested workflow").closest("details")!;
    expect(compare.open).toBe(false);
    // Set up: Builds isn't the next step, even before the first release.
    await waitFor(() => expect(within(nav()).getByRole("link", { name: "Builds" }).querySelector(".tab-mark")).toBeNull());
  });

  it("suggests another name when a workflow of the maker's own is at slop.yml", async () => {
    show(
      setup({
        taken: slopYml,
        file_path: ".github/workflows/slop-build.yml",
        create_url: "https://github.com/mia/harbour/new/main?filename=.github%2Fworkflows%2Fslop-build.yml",
      }),
    );
    const panel = await screen.findByRole("region", { name: "Let Slop Build do it" });
    await within(panel).findByText(/isn't Slop Build/);
    expect(panel.querySelector(".notice")?.textContent).toBe(
      "Your repository already has .github/workflows/slop.yml, and it isn't Slop Build, so this file goes at .github/workflows/slop-build.yml instead.",
    );
    expect(within(panel).getByRole("link", { name: "Add to my repository on GitHub" }).getAttribute("href")).toContain("slop-build.yml");
  });

  it("says when it couldn't look in .github/workflows", async () => {
    show(setup({ workflows_checked: false }));
    expect(await screen.findByText(/couldn't look in your repository's/)).toBeTruthy();
  });

  it("shows Slop Check as set up, or to add, and opens it for #slop-check", async () => {
    show(setup({ check_existing: { path: ".github/workflows/checks.yml", html_url: "https://github.com/x" }, check_create_url: null }));
    const check = (await screen.findByText("Slop Check is set up", { selector: "b" })).closest("details")!;
    expect(check.id).toBe("slop-check");
    expect(within(check).queryByRole("link", { name: "Add Slop Check to my repository" })).toBeNull();
    expect(within(check).getByRole("link", { name: ".github/workflows/checks.yml" })).toBeTruthy();
    cleanup();

    show(setup(), app(), "/publish/harbour/builds#slop-check");
    const details = (await screen.findByText("Building somewhere else?")).closest("details")!;
    await waitFor(() => expect(details.open).toBe(true));
    expect(within(details).getByRole("link", { name: "Add Slop Check to my repository" }).getAttribute("href")).toBe(
      setup().check_create_url,
    );
  });

  it("has nothing to build for a browser game, and doesn't ask GitHub", async () => {
    const load = show(setup(), app({ plays_in_browser: true, platforms: ["browser"] }));
    expect(await screen.findByRole("heading", { name: "Nothing to build" })).toBeTruthy();
    expect(load).not.toHaveBeenCalled();
  });

  it("says so when Slop Build is off on this server, and still has signing", async () => {
    show(new ApiError(503, "unavailable", "Slop Build isn't set up on this server"));
    expect(await screen.findByText(/Slop Build isn't switched on/)).toBeTruthy();
    expect(screen.getByText("Or build and sign it yourself")).toBeTruthy();
  });
});
