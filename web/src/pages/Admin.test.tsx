// SPDX-License-Identifier: AGPL-3.0-or-later
// What the browser tests can't reach on the shared test server: a queue with nothing in it, a
// report past its due date, and a report about a bounty.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, type AdminQueue, type QueuedReport } from "../api";
import { Admin } from "./Admin";

vi.mock("../session", () => ({
  useSession: () => ({ me: { id: "a", display_name: "Ada Admin", is_admin: true }, refresh: async () => {} }),
}));

const empty: AdminQueue = {
  abuse: [],
  rescans: [],
  releases: [],
  changes: [],
  media: [],
  portfolios: [],
  problems: [],
  review_disputes: [],
  ready_to_list: [],
  reports: [],
  appeals: [],
  rules: [
    ["scam", "Scams and fraud"],
    ["spam", "Spam"],
  ],
  checks: { signing_key: true, scanning: true, maker_signatures_required: true, github_app: true, github_token: true },
} as unknown as AdminQueue;

const report = (over: Partial<QueuedReport>): QueuedReport => ({
  id: "r1",
  reference: "R-AAAA-BBBB",
  subject_kind: "app",
  subject_label: "App: Miner",
  link: null,
  quoted: null,
  account: null,
  reason: "scam",
  reason_label: "A scam or fraud",
  details: "It asks for card numbers.",
  reporter: "Pat Player",
  copyright: null,
  priority: 1,
  due_at: "2026-10-01T00:00:00Z",
  overdue: false,
  created_at: "2026-09-30T00:00:00Z",
  same_subject: 0,
  actions: ["delist_app", "warn", "suspend_account"],
  suggested_rule: "scam",
  preserve: false,
  ...over,
});

const show = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <Admin />
    </MemoryRouter>,
  );

describe("the admin panel", () => {
  beforeEach(() => {
    vi.spyOn(api.admin, "donationHolds").mockResolvedValue([]);
    vi.spyOn(api.admin, "actions").mockResolvedValue({ approvals_required: 1, actions: [] } as never);
    vi.spyOn(api.admin, "ops").mockResolvedValue([]);
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("says when nothing is waiting", async () => {
    vi.spyOn(api.admin, "queue").mockResolvedValue(empty);
    show("/admin");
    expect(await screen.findByText(/Nothing waiting\./)).toBeTruthy();
    expect(screen.queryByRole("link", { name: /Reports$/ })).toBeNull();
  });

  it("marks reports past their due date", async () => {
    vi.spyOn(api.admin, "queue").mockResolvedValue({
      ...empty,
      reports: [report({ overdue: true }), report({ id: "r2", reference: "R-CCCC-DDDD", due_at: "2099-01-01T00:00:00Z" })],
    });
    show("/admin?tab=queue");
    expect(await screen.findByText("1 OVERDUE")).toBeTruthy();
    const late = screen.getByText(/R-AAAA-BBBB/);
    expect(late.textContent).toContain("(overdue)");
    expect(late.className).toContain("bad");
    expect(late.closest("li")?.className).toContain("overdue");
    expect(screen.getByText(/R-CCCC-DDDD/).closest("li")?.className).not.toContain("overdue");
    // Nothing missing on this server, so no notes about it.
    expect(screen.queryByText(/Scanning is off/)).toBeNull();
  });

  it("offers to hide a reported bounty", async () => {
    vi.spyOn(api.admin, "queue").mockResolvedValue({
      ...empty,
      reports: [report({ subject_kind: "bounty", subject_label: "Bounty: Free coins", actions: ["hide_bounty", "warn", "suspend_account"] })],
    });
    show("/admin?tab=queue");
    fireEvent.click(await screen.findByRole("button", { name: "Decide" }));
    const what = screen.getByRole("combobox", { name: "What to do" }) as HTMLSelectElement;
    expect([...what.options].map((o) => o.text)).toEqual(["Hide the bounty", "Warn them (no restriction)", "Suspend their account"]);
    expect(screen.getByRole("button", { name: "Hide the bounty" })).toBeTruthy();
  });
});
