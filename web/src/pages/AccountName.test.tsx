// SPDX-License-Identifier: AGPL-3.0-or-later
// Changing your name on the account page (Account.tsx NameField; the owner's ask, 2026-11-22).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError } from "../api";
import { NameField } from "./Account";

describe("your name on the account page", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("changes in place, and says it's saved", async () => {
    const save = vi.spyOn(api, "updateMe").mockResolvedValue(undefined);
    const onSaved = vi.fn().mockResolvedValue(undefined);
    render(<NameField name="Ann" onSaved={onSaved} />);
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    const box = screen.getByRole("textbox", { name: "Your name" }) as HTMLInputElement;
    expect(box.value).toBe("Ann");
    fireEvent.change(box, { target: { value: "   " } });
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(box, { target: { value: "Ann Archer" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText(/Saved\. Everyone sees the new name now\./);
    expect(save).toHaveBeenCalledWith({ display_name: "Ann Archer" });
    expect(onSaved).toHaveBeenCalled();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("keeps the box open with the server's reason when it's refused", async () => {
    vi.spyOn(api, "updateMe").mockRejectedValue(new ApiError(429, "rate_limited", "too many name changes for now: try again in a while"));
    render(<NameField name="Ann" onSaved={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Your name" }), { target: { value: "Ann 6" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toBe("too many name changes for now: try again in a while");
    expect(screen.getByRole("textbox", { name: "Your name" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("Ann")).toBeTruthy();
  });
});
