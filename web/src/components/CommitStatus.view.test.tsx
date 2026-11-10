// SPDX-License-Identifier: AGPL-3.0-or-later
// The Results on GitHub panel as the maker uses it: the switch, Check again, busy, and what an
// error or a 404 does. The sentences themselves are in CommitStatus.test.ts.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type CommitStatus } from "../api";
import { CommitStatusPanel } from "./CommitStatus";

const GITHUB = "https://github.com/apps/slaporslop/installations/new";

const off: CommitStatus = {
  on: false,
  since: null,
  installed: true,
  permission: null,
  github_url: GITHUB,
  in_store: true,
  problem: null,
  last: null,
};
const on: CommitStatus = { ...off, on: true, since: "2026-11-12T09:30:00Z", permission: true };

const panel = () => screen.getByRole("region", { name: "Results on GitHub" });
const box = () => within(panel()).getByRole<HTMLInputElement>("checkbox", { name: /Show SLAPorSLOP's result on each commit/ });

describe("CommitStatusPanel", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("says who sees it before it's switched on, then switches it on", async () => {
    let answer: (c: CommitStatus) => void = () => {};
    const set = vi.spyOn(api, "setCommitStatus").mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const changed = vi.fn();
    render(<CommitStatusPanel slug="pong" status={off} onChanged={changed} />);
    expect(box().checked).toBe(false);
    expect(box().disabled).toBe(false);
    expect(screen.getByText(/Anyone who can see the commit on GitHub sees it, also when a release isn't approved or is withdrawn\. It never says why\./)).toBeTruthy();
    // The live region is there before anything is in it, so what switching leads to is announced.
    expect(within(panel()).getByRole("status").textContent).toBe("");

    box().focus();
    fireEvent.click(box());
    expect(set).toHaveBeenCalledWith("pong", true);
    // Busy: the switch stays where it is, a second press sends nothing, and the panel says it's
    // busy. It is aria-disabled, not disabled, so it keeps focus.
    expect(box().getAttribute("aria-disabled")).toBe("true");
    expect(box().disabled).toBe(false);
    expect(panel().getAttribute("aria-busy")).toBe("true");
    fireEvent.click(box());
    expect(set).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(box());
    answer(on);
    await waitFor(() => expect(box().checked).toBe(true));
    expect(box().getAttribute("aria-disabled")).toBeNull();
    expect(panel().getAttribute("aria-busy")).toBe("false");
    expect(document.activeElement).toBe(box());
    expect(within(panel()).getByRole("status").textContent).toBe("Nothing posted yet: the next release you send gets a status.");
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it("switches it off", async () => {
    const set = vi.spyOn(api, "setCommitStatus").mockResolvedValue(off);
    render(<CommitStatusPanel slug="pong" status={on} onChanged={() => {}} />);
    fireEvent.click(box());
    await waitFor(() => expect(box().checked).toBe(false));
    expect(set).toHaveBeenCalledWith("pong", false);
  });

  it("can't be switched on before the App is installed, and links to installing it", () => {
    render(<CommitStatusPanel slug="pong" status={{ ...off, installed: false }} onChanged={() => {}} />);
    expect(box().disabled).toBe(true);
    expect(screen.getByText(/To use this, install SLAPorSLOP's GitHub App on this repository first\./)).toBeTruthy();
    expect(within(panel()).getByRole("link", { name: "Install on GitHub" }).getAttribute("href")).toBe(GITHUB);
    expect(within(panel()).queryByRole("button", { name: "Check again" })).toBeNull();
  });

  it("can always be switched off, also once the App is gone", () => {
    render(<CommitStatusPanel slug="pong" status={{ ...on, installed: false }} onChanged={() => {}} />);
    expect(box().disabled).toBe(false);
    expect(within(panel()).getByRole("link", { name: "Install on GitHub" }).getAttribute("href")).toBe(GITHUB);
    expect(within(panel()).getByRole("button", { name: "Check again" })).toBeTruthy();
  });

  it("switched off once the App is gone, focus moves from the switch it can't use any more to the install link", async () => {
    vi.spyOn(api, "setCommitStatus").mockResolvedValue({ ...off, installed: false });
    render(<CommitStatusPanel slug="pong" status={{ ...on, installed: false }} onChanged={() => {}} />);
    box().focus();
    fireEvent.click(box());
    await waitFor(() => expect(box().disabled).toBe(true));
    const link = within(panel()).getByRole("link", { name: "Install on GitHub" });
    await waitFor(() => expect(document.activeElement).toBe(link));
    expect(within(panel()).getByRole("status").textContent).toMatch(/^To use this, install SLAPorSLOP's GitHub App on this repository first\./);
  });

  it("out of the store, there's nothing to check again: it says nothing new is posted, and can be switched off", () => {
    render(
      <CommitStatusPanel slug="pong" status={{ ...on, in_store: false, problem: { code: 403, at: "2026-11-12T10:05:00Z" } }} onChanged={() => {}} />,
    );
    expect(within(panel()).getByRole("status").textContent).toBe(
      "Nothing new is posted while the app isn't in the store. When it is, the newest result is posted.",
    );
    expect(within(panel()).queryByRole("button", { name: "Check again" })).toBeNull();
    expect(box().disabled).toBe(false);
  });

  it("Check again asks once more and shows the answer", async () => {
    const set = vi.spyOn(api, "setCommitStatus").mockResolvedValue(on);
    render(<CommitStatusPanel slug="pong" status={{ ...on, permission: false }} onChanged={() => {}} />);
    expect(within(panel()).getByRole("link", { name: "Open the App on GitHub" }).getAttribute("href")).toBe(GITHUB);
    const again = within(panel()).getByRole("button", { name: "Check again" });
    again.focus();
    fireEvent.click(again);
    expect(set).toHaveBeenCalledWith("pong", true);
    await waitFor(() => expect(within(panel()).queryByRole("button", { name: "Check again" })).toBeNull());
    expect(screen.queryByText(/Commit statuses: Read and write/)).toBeNull();
    // The button went with the problem: focus is on the switch, not lost.
    expect(document.activeElement).toBe(box());
  });

  it("Check again that finds the problem still there keeps focus on its button", async () => {
    const still: CommitStatus = { ...on, permission: false };
    vi.spyOn(api, "setCommitStatus").mockResolvedValue(still);
    render(<CommitStatusPanel slug="pong" status={still} onChanged={() => {}} />);
    const again = within(panel()).getByRole("button", { name: "Check again" });
    again.focus();
    fireEvent.click(again);
    expect(again.getAttribute("aria-disabled")).toBe("true");
    await waitFor(() => expect(again.getAttribute("aria-disabled")).toBeNull());
    expect(document.activeElement).toBe(again);
    // The press is heard, though the notice is the same: the live region says it checked.
    const live = within(panel()).getByRole("status");
    expect(live.textContent).toMatch(/^Checked again: nothing has changed on GitHub yet\.GitHub hasn't let SLAPorSLOP's App do this yet\./);
    // Pressed again, the sentence goes while it asks and comes back, so it is announced again.
    fireEvent.click(again);
    expect(live.textContent).not.toMatch(/Checked again/);
    await waitFor(() => expect(live.textContent).toMatch(/^Checked again: /));
  });

  it("the switch's answer for a new problem says it in the live region, with no promise beside it", async () => {
    vi.spyOn(api, "setCommitStatus").mockResolvedValue({ ...on, permission: false });
    render(<CommitStatusPanel slug="pong" status={off} onChanged={() => {}} />);
    expect(within(panel()).getByRole("status").textContent).toBe("");
    fireEvent.click(box());
    const live = within(panel()).getByRole("status");
    await waitFor(() => expect(live.textContent).toMatch(/^GitHub hasn't let SLAPorSLOP's App do this yet\./));
    expect(live.textContent).not.toMatch(/next release you send gets a status/);
  });

  it("shows a refusal, keeps the switch as it was, and reads the overview again", async () => {
    vi.spyOn(api, "setCommitStatus").mockRejectedValue(
      new ApiError(409, "conflict", "SLAPorSLOP's GitHub App isn't installed on this repository: install it on GitHub, then try again"),
    );
    const changed = vi.fn();
    render(<CommitStatusPanel slug="pong" status={off} onChanged={changed} />);
    fireEvent.click(box());
    const alert = await within(panel()).findByRole("alert");
    expect(alert.textContent).toBe("SLAPorSLOP's GitHub App isn't installed on this repository: install it on GitHub, then try again");
    expect(box().checked).toBe(false);
    // A 409 means the overview is out of date: the page reads it again, so the panel shows what to do.
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it("a 404 means it isn't offered any more: the page reads the overview again, and no error shows", async () => {
    vi.spyOn(api, "setCommitStatus").mockRejectedValue(new ApiError(404, "not_found", "not found"));
    const changed = vi.fn();
    render(
      <main id="main" tabIndex={-1}>
        <CommitStatusPanel slug="pong" status={on} onChanged={changed} />
      </main>,
    );
    box().focus();
    fireEvent.click(box());
    await waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
    expect(within(panel()).queryByRole("alert")).toBeNull();
    // The panel goes with the reload: focus moves to the page's content, not lost with it.
    expect(document.activeElement).toBe(document.getElementById("main"));
  });
});
