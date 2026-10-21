// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../platform", () => {
  const listeners: Array<(u: unknown) => void> = [];
  return {
    isDesktop: true,
    desktop: {
      updateStatus: vi.fn(async () => null),
      installUpdate: vi.fn(async () => {}),
      onUpdateAvailable: vi.fn(async (h: (u: unknown) => void) => {
        listeners.push(h);
        return () => {};
      }),
      __emit: (u: unknown) => listeners.forEach((h) => h(u)),
    },
  };
});

import { desktop } from "../platform";
import { UpdateNotice } from "./UpdateNotice";

const emit = (u: unknown) => (desktop as unknown as { __emit: (u: unknown) => void }).__emit(u);

describe("UpdateNotice", () => {
  afterEach(cleanup);

  it("stays hidden until an update is found, and never installs on its own", async () => {
    render(<UpdateNotice />);
    await act(async () => {});
    expect(screen.queryByRole("status")).toBeNull();
    await act(async () => emit({ version: "0.1.40", current_version: "0.1.33", notes: null }));
    expect(screen.getByText(/Slop 0.1.40 is available/)).toBeTruthy();
    expect(desktop.installUpdate).not.toHaveBeenCalled();
  });

  it("installs only when asked", async () => {
    render(<UpdateNotice />);
    await act(async () => emit({ version: "0.1.41", current_version: "0.1.33", notes: null }));
    fireEvent.click(screen.getByRole("button", { name: "Update and restart" }));
    expect(desktop.installUpdate).toHaveBeenCalledTimes(1);
  });

  it("Later hides it", async () => {
    render(<UpdateNotice />);
    await act(async () => emit({ version: "0.1.42", current_version: "0.1.33", notes: null }));
    fireEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(screen.queryByRole("status")).toBeNull();
  });
});
