// SPDX-License-Identifier: AGPL-3.0-or-later
// The To fix tab (Fixes.tsx): must and should items, each ready to copy for an AI agent, the
// suggestions after them, and what it says when there's nothing to fix.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, type AppDetail, type AppFixes, type ManageOverview } from "../api";
import { Fixes, fixPrompt, suggestionPrompt } from "./Fixes";

vi.mock("../session", () => ({ useSession: () => ({ me: { id: "mia" } }) }));

const app = (over: Partial<AppDetail> = {}) =>
  ({
    slug: "harbour",
    title: "Harbour",
    stage: "beta",
    status: "listed",
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
    is_mine: true,
    ...over,
  }) as unknown as AppDetail;

const found: AppFixes = {
  release: { id: "r1", version: "1.2", status: "awaiting_review" },
  fixes: [
    { kind: "secret", severity: "must", problem: "A key is in src/net.rs.", instruction: "Revoke it and remove it." },
    {
      kind: "saves",
      severity: "should",
      problem: "Saves go to Documents.",
      instruction: "Write saves to SLOP_SAVE_DIR.",
      guide_section: "Saves and sandbox mode",
    },
  ],
  recommendations: [{ kind: "media:icon", severity: "recommended", problem: "No icon yet.", instruction: "Add slop/icon.png." }],
  next: "Then submit a new release.",
};

let copied: string[] = [];
beforeEach(() => {
  copied = [];
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: (t: string) => (copied.push(t), Promise.resolve()) },
  });
  vi.spyOn(api, "manage").mockResolvedValue({ can_advance: true } as ManageOverview);
  vi.spyOn(api, "publishInvite").mockResolvedValue({ required: false, invited: false } as never);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function show(fixes: AppFixes, detail = app()) {
  vi.spyOn(api, "app").mockResolvedValue(detail);
  const load = vi.spyOn(api, "fixes").mockResolvedValue(fixes);
  render(
    <MemoryRouter initialEntries={["/publish/harbour/fixes"]}>
      <Routes>
        <Route path="/publish/:slug/fixes" element={<Fixes />} />
      </Routes>
    </MemoryRouter>,
  );
  return load;
}

describe("To fix", () => {
  it("lists what must and should be fixed, each to copy for an AI, then the suggestions", async () => {
    const load = show(found);
    const panel = await screen.findByRole("region", { name: "To fix in 1.2" });
    const items = within(panel).getAllByRole("listitem");
    expect(items.map((li) => li.textContent?.slice(0, 20))).toEqual(["MUST FIX A key is in", "SHOULD FIX Saves go "]);
    fireEvent.click(within(items[1]).getByRole("button", { name: "Copy for your AI" }));
    expect(copied).toEqual([
      "SLAPorSLOP's checks of my app found this. Saves go to Documents.\nFix it in the repository: Write saves to SLOP_SAVE_DIR.\n" +
        '(SLAPorSLOP\'s publishing guide, section "Saves and sandbox mode", has the details.)',
    ]);
    fireEvent.click(within(panel).getByRole("button", { name: "Copy all for your AI" }));
    expect(copied[1]).toContain("A key is in src/net.rs.");
    expect(copied[1]).toMatch(/Then submit a new release\.$/);

    // Suggestions come after, quieter, and hold nothing up.
    const suggestions = screen.getByRole("region", { name: "Worth doing" });
    expect(within(suggestions).getByText("SUGGESTED")).toBeTruthy();
    fireEvent.click(within(suggestions).getByRole("button", { name: "Copy for your AI" }));
    expect(copied[2]).toBe("SLAPorSLOP suggests this for my app. No icon yet.\nDo it in the repository: Add slop/icon.png.");

    // The tab's count comes from the same answer: one call.
    expect(screen.getByRole("link", { name: "To fix, 1 must be fixed, 1 should be fixed" }).getAttribute("aria-current")).toBe("page");
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("says when there's nothing to fix, while a release is being checked, and before the first", async () => {
    show({ ...found, fixes: [], recommendations: [] });
    expect(await screen.findByText("Nothing to fix in 1.2.")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Worth doing" })).toBeNull();
    expect(screen.getByRole("link", { name: "To fix" })).toBeTruthy();
    cleanup();

    show({ ...found, release: { id: "r2", version: "1.3", status: "fetching" }, fixes: [] });
    expect((await screen.findByRole("status")).textContent).toBe("SLAPorSLOP is still checking 1.3: what it finds shows here.");
    cleanup();

    show({ ...found, release: null, fixes: [] });
    const panel = await screen.findByRole("region", { name: "To fix" });
    expect(panel.textContent).toContain("SLAPorSLOP's checks run on each release you send");
    expect(within(panel).getByRole("link", { name: "Submit a release" }).getAttribute("href")).toBe("/publish/harbour/releases");
    // Suggestions show before there's a release too.
    expect(screen.getByRole("region", { name: "Worth doing" })).toBeTruthy();
  });

  it("speaks of play addresses for a browser game", async () => {
    show({ ...found, release: null, fixes: [], recommendations: [] }, app({ plays_in_browser: true, platforms: ["browser"] }));
    const panel = await screen.findByRole("region", { name: "To fix" });
    expect(panel.textContent).toContain("on each play address you send");
    expect(within(panel).getByRole("link", { name: "Add the play address" })).toBeTruthy();
  });

  it("words fixes and suggestions for an AI agent", () => {
    expect(fixPrompt(found.fixes[0])).toBe(
      "SLAPorSLOP's checks of my app found this. A key is in src/net.rs.\nFix it in the repository: Revoke it and remove it.",
    );
    expect(suggestionPrompt({ ...found.recommendations[0], guide_section: "Pictures" })).toContain(
      '(SLAPorSLOP\'s publishing guide, section "Pictures", has the details.)',
    );
  });
});
