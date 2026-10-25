// SPDX-License-Identifier: AGPL-3.0-or-later
// What the browser tests can't reach on the shared test server: a queue with nothing in it, a
// report past its due date, a report about a bounty, and the inbox's quick decisions with every
// request caught.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, type AdminApp, type AdminQueue, type QueuedReport } from "../api";
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
    vi.spyOn(api.admin, "aiMessages").mockResolvedValue([]);
    vi.spyOn(api.admin, "legalRequests").mockResolvedValue([]);
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
    show("/admin?tab=queue&box=reports");
    const rows = within(await screen.findByRole("list", { name: "Reports" })).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText("OVERDUE")).toBeTruthy();
    expect(within(rows[1]).getByText("SERIOUS")).toBeTruthy();
    // Nothing missing on this server, so no notes about it.
    expect(screen.queryByText(/Scanning is off/)).toBeNull();
    cleanup();

    // Opened, the report says so too.
    show("/admin?tab=queue&box=reports&item=r1");
    const late = await screen.findByText(/R-AAAA-BBBB · due/);
    expect(late.textContent).toContain("(overdue)");
    expect(late.className).toContain("bad");
    expect(late.closest("li")?.className).toContain("overdue");
  });

  it("offers to hide a reported bounty", async () => {
    vi.spyOn(api.admin, "queue").mockResolvedValue({
      ...empty,
      reports: [report({ subject_kind: "bounty", subject_label: "Bounty: Free coins", actions: ["hide_bounty", "warn", "suspend_account"] })],
    });
    show("/admin?tab=queue&box=reports&item=r1");
    fireEvent.click(await screen.findByRole("button", { name: "Decide" }));
    const what = screen.getByRole("combobox", { name: "What to do" }) as HTMLSelectElement;
    expect([...what.options].map((o) => o.text)).toEqual(["Hide the bounty", "Warn them (no restriction)", "Suspend their account"]);
    expect(screen.getByRole("button", { name: "Hide the bounty" })).toBeTruthy();
  });

  it("denies only after Are you sure?, with the reason the maker reads; cancelling keeps it", async () => {
    const release = {
      id: "rel1",
      version: "2.0.0",
      commit_sha: "a".repeat(40),
      status: "awaiting_review",
      commit_verified: true,
      created_at: "2026-10-20T00:00:00Z",
      source: "maker",
      auto_approved: false,
      files: [],
      source_checks: null,
      repo_scan: null,
      advisories: [],
      app: { slug: "miner", title: "Miner", stage: "released", status: "draft", repo_url: "https://github.com/o/miner", maker: "Mia" },
      notes: [],
    };
    const queue = vi.spyOn(api.admin, "queue").mockResolvedValue({ ...empty, releases: [release] } as unknown as AdminQueue);
    const reject = vi.spyOn(api.admin, "reject").mockResolvedValue(undefined);
    show("/admin?tab=queue&box=releases");
    const deny = await screen.findByRole("button", { name: "Deny Miner 2.0.0" });
    fireEvent.click(deny);
    expect(screen.getByText("Are you sure?")).toBeTruthy();
    const yes = screen.getByRole("button", { name: "Yes, deny" }) as HTMLButtonElement;
    // The reason comes first.
    expect(yes.disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Are you sure?")).toBeNull();
    expect(reject).not.toHaveBeenCalled();

    fireEvent.click(deny);
    fireEvent.change(screen.getByRole("textbox", { name: /Why \(the maker reads this\)/ }), { target: { value: "No program in the zip." } });
    queue.mockResolvedValue(empty);
    fireEvent.click(screen.getByRole("button", { name: "Yes, deny" }));
    await waitFor(() => expect(reject).toHaveBeenCalledWith("rel1", "No program in the zip."));
    expect(await screen.findByText("Rejected “Miner 2.0.0”.")).toBeTruthy();
    expect(await screen.findByText("Nothing waiting here.")).toBeTruthy();
  });

  it("lists apps a page at a time, and asks for only those needing attention, most urgent first", async () => {
    vi.spyOn(api.admin, "queue").mockResolvedValue(empty);
    const row = (n: number): AdminApp =>
      ({
        slug: `app-${n}`,
        title: `App ${n}`,
        stage: "released",
        status: "listed",
        repo_url: "https://github.com/x/y",
        maker: "Mia",
        releases: [],
        pending_releases: 0,
        flags: n === 0 ? ["reports"] : [],
        visibility: "public",
        icon_url: null,
        thumbnail_url: null,
        frozen_at: null,
        frozen_reason: null,
        stage_deadline: null,
        last_commit_at: null,
        updated_at: "2026-10-20T00:00:00Z",
        newest_release: null,
        serving_version: "1.0.0",
        rescan_flags: 0,
        downloads_lost: 0,
        open_reports: n === 0 ? 3 : 0,
        donations_paused: false,
        players: 0,
        last_played_at: null,
        vote_count: 0,
        not_slop_percent: null,
      }) as AdminApp;
    const apps = vi
      .spyOn(api.admin, "apps")
      .mockImplementation(async (q) => (q?.attention ? [row(0)] : q?.offset ? [row(50), row(51)] : Array.from({ length: 50 }, (_, i) => row(i))));
    show("/admin?tab=apps");
    const list = await screen.findByRole("list", { name: "Apps" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(50);
    expect(within(list).getByText("3 open reports")).toBeTruthy();
    expect(apps).toHaveBeenLastCalledWith({ status: undefined, attention: false, sort: "updated", q: undefined, limit: 50, offset: 0 });
    // A full page: there may be more.
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    await waitFor(() => expect(within(list).getAllByRole("listitem")).toHaveLength(52));
    expect(apps).toHaveBeenCalledWith(expect.objectContaining({ offset: 50 }));
    expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();

    // Needing attention starts again from one page, most urgent first.
    fireEvent.change(screen.getByRole("combobox", { name: "Show" }), { target: { value: "attention" } });
    await waitFor(() => expect(within(screen.getByRole("list", { name: "Apps" })).getAllByRole("listitem")).toHaveLength(1));
    expect(apps).toHaveBeenLastCalledWith({ status: undefined, attention: true, sort: "urgent", q: undefined, limit: 50, offset: 0 });
    expect((screen.getByRole("combobox", { name: "Sort" }) as HTMLSelectElement).value).toBe("urgent");
    // (Fifty lines in jsdom take a few seconds on a busy machine.)
  }, 30_000);
});
