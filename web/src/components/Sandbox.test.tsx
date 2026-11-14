// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SaveSyncNotice } from "./Sandbox";

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
