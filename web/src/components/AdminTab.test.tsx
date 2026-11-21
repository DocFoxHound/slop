// SPDX-License-Identifier: AGPL-3.0-or-later
// The Admin tab (Layout.tsx AdminTab): only for staff, with a mark like the bell's saying how many
// things wait in the review queue (the owner's ask, 2026-11-21).
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type AdminWaiting } from "../api";
import { AdminTab } from "./Layout";

const session = vi.hoisted(() => ({ me: null as null | { is_admin: boolean } }));
vi.mock("../session", () => ({ useSession: () => session }));

const counts = (total: number): AdminWaiting => ({
  releases: total,
  changes: 0,
  pictures: 0,
  listing: 0,
  rescans: 0,
  people: 0,
  problems: 0,
  disputes: 0,
  reports: 0,
  appeals: 0,
  abuse: 0,
  legal: null,
  approvals: 0,
  ai: 0,
  donations: 0,
  total,
});

describe("the Admin tab", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("isn't there for players, and asks nothing", () => {
    session.me = { is_admin: false };
    const waiting = vi.spyOn(api.admin, "waiting");
    render(<AdminTab />, { wrapper: MemoryRouter });
    expect(screen.queryByRole("link")).toBeNull();
    expect(waiting).not.toHaveBeenCalled();
  });

  it("shows how many wait, and nothing when none do", async () => {
    session.me = { is_admin: true };
    vi.spyOn(api.admin, "waiting").mockResolvedValueOnce(counts(3)).mockResolvedValue(counts(0));
    render(<AdminTab />, { wrapper: MemoryRouter });
    const link = await screen.findByRole("link", { name: "ADMIN, 3 waiting" });
    expect(link.textContent).toBe("ADMIN3, 3 waiting");
    cleanup();
    render(<AdminTab />, { wrapper: MemoryRouter });
    await vi.waitFor(() => expect(api.admin.waiting).toHaveBeenCalledTimes(2));
    expect((await screen.findByRole("link", { name: "ADMIN" })).textContent).toBe("ADMIN");
  });

  it("says 99+ for a hundred or more", async () => {
    session.me = { is_admin: true };
    vi.spyOn(api.admin, "waiting").mockResolvedValue(counts(140));
    render(<AdminTab />, { wrapper: MemoryRouter });
    expect((await screen.findByRole("link", { name: "ADMIN, 140 waiting" })).textContent).toBe("ADMIN99+, 140 waiting");
  });
});
