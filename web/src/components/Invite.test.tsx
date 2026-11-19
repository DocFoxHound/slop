// SPDX-License-Identifier: AGPL-3.0-or-later
// The beta's invite code: asked for when going public needs it, then the action runs again; a
// note where makers publish while it's needed.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError } from "../api";
import { InviteNotice, useInviteGate } from "./Invite";

// jsdom has no modal dialogs: here showModal only opens one.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const needed = () =>
  new ApiError(409, "invite_required", "SLAPorSLOP is in beta: going public needs an invite code. Enter it when you publish on the website.");

/** A button that runs `action` through the gate and shows how it ended. */
function Gated({ action }: { action: () => Promise<string> }) {
  const invite = useInviteGate();
  const [result, setResult] = useState("");
  return (
    <>
      <button
        type="button"
        onClick={() =>
          void invite.guard(action).then(
            (r) => setResult(`done: ${r}`),
            (e: unknown) => setResult(`refused: ${e instanceof ApiError ? e.code : String(e)}`),
          )
        }
      >
        Go public
      </button>
      <output>{result}</output>
      {invite.prompt}
    </>
  );
}

describe("useInviteGate", () => {
  it("asks for the code, says when it's wrong, and runs the action again once it's right", async () => {
    const action = vi.fn().mockRejectedValueOnce(needed()).mockResolvedValue("published");
    const enter = vi
      .spyOn(api, "enterPublishInvite")
      .mockRejectedValueOnce(new ApiError(422, "invalid", "That isn't the invite code. Check it and try again.", "code"))
      .mockResolvedValue({ required: true, invited: true });
    render(<Gated action={action} />);
    fireEvent.click(screen.getByRole("button", { name: "Go public" }));

    const dialog = await screen.findByRole("dialog", { name: "Enter your invite code" });
    const field = screen.getByRole("textbox", { name: "Invite code" });
    expect(dialog.contains(field)).toBe(true);
    // Keyboard users start in the field.
    expect(document.activeElement).toBe(field);
    const submit = screen.getByRole("button", { name: "Enter and continue" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    fireEvent.change(field, { target: { value: "beta-wolf" } });
    fireEvent.click(submit);
    const error = await screen.findByRole("alert");
    expect(error.textContent).toBe("That isn't the invite code. Check it and try again.");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toContain(error.id);
    expect(document.activeElement).toBe(field);
    expect(action).toHaveBeenCalledTimes(1);

    fireEvent.change(field, { target: { value: "Beta-Fox" } });
    fireEvent.click(screen.getByRole("button", { name: "Enter and continue" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("done: published"));
    expect(enter).toHaveBeenLastCalledWith("Beta-Fox");
    expect(action).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Not now gives the refusal back, so the page says why", async () => {
    const action = vi.fn().mockRejectedValue(needed());
    render(<Gated action={action} />);
    fireEvent.click(screen.getByRole("button", { name: "Go public" }));
    fireEvent.click(await screen.findByRole("button", { name: "Not now" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("refused: invite_required"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("leaves every other refusal alone", async () => {
    const action = vi.fn().mockRejectedValue(new ApiError(409, "conflict", "Version 1.5 is newer and already out"));
    render(<Gated action={action} />);
    fireEvent.click(screen.getByRole("button", { name: "Go public" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("refused: conflict"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("InviteNotice", () => {
  it("says so before Publish while the code is needed, and goes once it's entered", async () => {
    vi.spyOn(api, "publishInvite").mockResolvedValueOnce({ required: true, invited: false }).mockResolvedValue({ required: true, invited: true });
    vi.spyOn(api, "enterPublishInvite").mockResolvedValue({ required: true, invited: true });
    render(<InviteNotice />);
    expect((await screen.findByText(/going public needs an invite code/)).textContent).toContain(
      "You can make store pages and send releases through the checks now",
    );
    const open = screen.getByRole("button", { name: "Enter invite code" });
    open.focus();
    fireEvent.click(open);
    fireEvent.change(screen.getByRole("textbox", { name: "Invite code" }), { target: { value: "beta-fox" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Enter and continue" }));
    });
    await waitFor(() => expect(screen.queryByText(/going public needs an invite code/)).toBeNull());
  });

  it("shows nothing when the code isn't needed, or the account has it", async () => {
    const status = vi.spyOn(api, "publishInvite").mockResolvedValue({ required: false, invited: false });
    const { container, unmount } = render(<InviteNotice />);
    await waitFor(() => expect(status).toHaveBeenCalled());
    expect(container.textContent).toBe("");
    unmount();
    status.mockResolvedValue({ required: true, invited: true });
    const again = render(<InviteNotice />);
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));
    expect(again.container.textContent).toBe("");
  });
});
