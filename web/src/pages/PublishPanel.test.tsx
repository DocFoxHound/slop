// SPDX-License-Identifier: AGPL-3.0-or-later
// Publish, on a release that passed its checks: what to know first, what it waits for, and one
// press after a confirm. No checkbox, and no signing in again.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type Release } from "../api";
import { PublishPanel } from "./Releases";

// jsdom has no modal dialogs: here showModal only opens one.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const release = (over: Partial<Release> = {}): Release => ({
  id: "rel4",
  version: "1.4",
  commit_sha: "a".repeat(40),
  status: "ready",
  commit_verified: true,
  created_at: "2026-11-07T10:00:00Z",
  source: "maker",
  auto_approved: true,
  files: [],
  source_checks: null,
  repo_scan: { status: "checked" },
  advisories: [],
  ...over,
});

function show(over: Partial<Parameters<typeof PublishPanel>[0]> = {}) {
  const props = {
    slug: "harbour",
    release: release(),
    browser: false,
    comingSoon: false,
    playsNow: null,
    picturesPending: false,
    onPublished: vi.fn(),
    onRefused: vi.fn(),
    ...over,
  };
  render(
    <MemoryRouter>
      <PublishPanel {...props} />
    </MemoryRouter>,
  );
  return props;
}

describe("Publish", () => {
  it("publishes after one confirm, with no checkbox or password", async () => {
    const published = release({ status: "approved" });
    const publish = vi.spyOn(api, "publishRelease").mockResolvedValue(published);
    const props = show({ heldAt: "1.2" });
    expect(screen.getByRole("region", { name: "Publish version 1.4" }).textContent).toContain(
      "It passed SLAPorSLOP's checks. Once you publish it, SLAPorSLOP signs it and players get it.",
    );
    expect(screen.getByRole("list", { name: "Before you publish" }).textContent).toBe(
      "Players are held at 1.2. Publishing 1.4 ends the hold.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Publish version 1.4…" }));
    const confirm = screen.getByRole("group", { name: "Publish version 1.4" });
    expect(confirm.textContent).toContain(
      "Players get it straight away, and the hold at 1.2 ends. To roll back later, hold players at an earlier version on Manage, or publish a newer one.",
    );
    // Holding players at an earlier version is doable from here: Manage is a link.
    expect(screen.getByRole("link", { name: "Manage" }).getAttribute("href")).toBe("/publish/harbour/manage");
    expect(confirm.querySelector("input")).toBeNull();
    expect(document.querySelector("input[type=password], input[type=checkbox]")).toBeNull();
    const yes = screen.getByRole("button", { name: "Yes, publish it" });
    // Keyboard users land on it.
    expect(document.activeElement).toBe(yes);
    fireEvent.click(yes);
    await waitFor(() => expect(props.onPublished).toHaveBeenCalledWith(published));
    expect(publish).toHaveBeenCalledWith("harbour", "rel4");
  });

  it("can be put off: Not yet goes back, with focus on the button again", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Publish version 1.4…" }));
    fireEvent.click(screen.getByRole("button", { name: "Not yet" }));
    expect(screen.queryByRole("group", { name: "Publish version 1.4" })).toBeNull();
    const again = screen.getByRole("button", { name: "Publish version 1.4…" });
    expect(document.activeElement).toBe(again);
  });

  it("waits for new pictures with the reason, and the button can't be pressed and says why", () => {
    show({ picturesPending: true });
    const wait = screen.getByRole("status");
    expect(wait.textContent).toBe("Its new pictures are waiting for review. You can publish once they're reviewed.");
    const button = screen.getByRole("button", { name: "Publish version 1.4…" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("aria-describedby")).toBe(wait.id);
  });

  it("a draft: players get it once its maker puts it in the store", () => {
    show({ draft: true });
    expect(screen.getByRole("region", { name: "Publish version 1.4" }).textContent).toContain(
      "It passed SLAPorSLOP's checks. Once you publish it, SLAPorSLOP signs it. Players can get it once you put the app in the store.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Publish version 1.4…" }));
    expect(screen.getByRole("group", { name: "Publish version 1.4" }).textContent).toContain(
      "Players can get it once you put the app in the store.",
    );
  });

  it("shows new source-check concerns and a new site as information, not as something to confirm", () => {
    show({
      browser: true,
      playsNow: "https://old.example.com/",
      release: release({
        play_address: "https://new.example.net/",
        new_play_address: true,
        new_concerns: "the source check found code loading that 1.3 didn't have",
      }),
    });
    const notes = screen.getByRole("list", { name: "Before you publish" }).textContent;
    expect(notes).toContain("A new site: Play will open https://new.example.net/ instead of https://old.example.com/.");
    expect(notes).toContain("The source check found code loading that 1.3 didn't have. See the source check on this release.");
    expect(screen.getByRole("region", { name: "Publish version 1.4" }).textContent).toContain("Once you publish it, Play opens its address.");
    expect((screen.getByRole("button", { name: "Publish version 1.4…" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("asks for the beta's invite code when it's needed, then publishes", async () => {
    const published = release({ status: "approved" });
    const publish = vi
      .spyOn(api, "publishRelease")
      .mockRejectedValueOnce(new ApiError(409, "invite_required", "SLAPorSLOP is in beta: going public needs an invite code."))
      .mockResolvedValue(published);
    const enter = vi.spyOn(api, "enterPublishInvite").mockResolvedValue({ required: true, invited: true });
    const props = show();
    fireEvent.click(screen.getByRole("button", { name: "Publish version 1.4…" }));
    fireEvent.click(screen.getByRole("button", { name: "Yes, publish it" }));
    await screen.findByRole("dialog", { name: "Enter your invite code" });
    fireEvent.change(screen.getByRole("textbox", { name: "Invite code" }), { target: { value: "beta-fox" } });
    fireEvent.click(screen.getByRole("button", { name: "Enter and continue" }));
    await waitFor(() => expect(props.onPublished).toHaveBeenCalledWith(published));
    expect(enter).toHaveBeenCalledWith("beta-fox");
    expect(publish).toHaveBeenCalledTimes(2);
    expect(props.onRefused).not.toHaveBeenCalled();
  });

  it("says why it couldn't be published, and the page looks again", async () => {
    vi.spyOn(api, "publishRelease").mockRejectedValue(
      new ApiError(409, "conflict", "It lowers the age rating, so a SLAPorSLOP staff member checks it before it can be published"),
    );
    const props = show();
    fireEvent.click(screen.getByRole("button", { name: "Publish version 1.4…" }));
    fireEvent.click(screen.getByRole("button", { name: "Yes, publish it" }));
    expect((await screen.findByRole("alert")).textContent).toContain("a SLAPorSLOP staff member checks it");
    expect(props.onRefused).toHaveBeenCalled();
    expect(props.onPublished).not.toHaveBeenCalled();
  });
});
