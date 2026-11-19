// SPDX-License-Identifier: AGPL-3.0-or-later
// Staff switch the beta's invite code on and set it, and see how many accounts have it.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type AdminPublishInvite } from "../../api";
import { PublishInviteEditor } from "./StoreTab";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const off: AdminPublishInvite = {
  required: false,
  code: "",
  updated_at: "2026-11-19T10:00:00Z",
  invited: 12,
  invited_since_update: 0,
};

describe("Publishing invite", () => {
  it("switches it on with a code, and says how many accounts have it", async () => {
    vi.spyOn(api.admin, "publishInvite")
      .mockResolvedValueOnce(off)
      .mockResolvedValue({ ...off, required: true, code: "beta-fox" });
    const set = vi.spyOn(api.admin, "setPublishInvite").mockResolvedValue({ ...off, required: true, code: "beta-fox" });
    render(<PublishInviteEditor />);
    const toggle = await screen.findByRole("switch", { name: "Going public needs the invite code" });
    expect(screen.getByText(/12 accounts can go public/)).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.click(toggle);
    fireEvent.change(screen.getByRole("textbox", { name: "Invite code" }), { target: { value: "beta-fox" } });
    fireEvent.click(save);
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Saved."));
    expect(set).toHaveBeenCalledWith(true, "beta-fox");
  });

  it("puts a refused code's reason on the field", async () => {
    vi.spyOn(api.admin, "publishInvite").mockResolvedValue(off);
    vi.spyOn(api.admin, "setPublishInvite").mockRejectedValue(
      new ApiError(422, "invalid", "Make the code 4 to 64 characters.", "code"),
    );
    render(<PublishInviteEditor />);
    fireEvent.click(await screen.findByRole("switch", { name: "Going public needs the invite code" }));
    const field = screen.getByRole("textbox", { name: "Invite code" });
    fireEvent.change(field, { target: { value: "abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const error = await screen.findByRole("alert");
    expect(error.textContent).toBe("Make the code 4 to 64 characters.");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toContain(error.id);
  });
});
