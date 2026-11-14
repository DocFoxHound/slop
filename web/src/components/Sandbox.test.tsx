// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { SaveSyncOutcome } from "../platform";

type Synced = { slug: string; result: SaveSyncOutcome };
const platform = vi.hoisted(() => ({ handler: undefined as ((e: Synced) => void) | undefined }));
vi.mock("../platform", () => ({
  isDesktop: true,
  desktop: {
    onSavesSynced: async (h: (e: Synced) => void) => {
      platform.handler = h;
      return () => {};
    },
    installed: async () => [{ slug: "pocket-golf", title: "Pocket Golf" }],
  },
}));

import { SaveSyncEvents, SaveSyncNotice } from "./Sandbox";

afterEach(cleanup);

describe("SaveSyncNotice", () => {
  it("closes the over-20 MB notice with its ×", () => {
    render(<SaveSyncNotice slug="pong" title="Pong" result={{ outcome: "too_big" }} />);
    expect(screen.getByRole("status").textContent).toContain("Pong's saves are over 20 MB, so they stay on this computer only.");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("closes the fetched notice with its ×", () => {
    render(<SaveSyncNotice slug="pong" title="Pong" result={{ outcome: "fetched", generation: 2 }} />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("SaveSyncEvents", () => {
  it("names the app by its title, and shows a new notice after one was closed", async () => {
    render(<SaveSyncEvents />);
    await vi.waitFor(() => expect(platform.handler).toBeDefined());
    act(() => platform.handler!({ slug: "pocket-golf", result: { outcome: "too_big" } }));
    expect((await screen.findByRole("status")).textContent).toContain("Pocket Golf's saves are over 20 MB");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status")).toBeNull();
    act(() => platform.handler!({ slug: "unknown-app", result: { outcome: "too_big" } }));
    expect((await screen.findByRole("status")).textContent).toContain("unknown-app's saves are over 20 MB");
  });
});
