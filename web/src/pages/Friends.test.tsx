// SPDX-License-Identifier: AGPL-3.0-or-later
// Adding a friend (Friends.tsx): by name, found as you type (the owner's ask, 2026-11-21), or by a
// pasted profile link; and your own profile link to send people.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, type FoundPerson } from "../api";
import { FriendsPage, isProfileLink } from "./Friends";

vi.mock("../session", () => ({ useSession: () => ({ me: { id: "01a1f6ac-11dc-77c1-8e41-c615060db719" } }) }));

const person = (name: string, friend: FoundPerson["friend"] = "none"): FoundPerson => ({
  id: `id-${name}`,
  name,
  joined: "2026-10-03T12:00:00Z",
  avatar_url: null,
  friend,
});

describe("isProfileLink", () => {
  it("tells a pasted link or ID from a name", () => {
    expect(isProfileLink("https://slaporslop.games/u/01a1f6ac-11dc-77c1-8e41-c615060db719")).toBe(true);
    expect(isProfileLink(" slaporslop.games/u/abc/ ")).toBe(true);
    expect(isProfileLink("01A1F6AC-11DC-77C1-8E41-C615060DB719")).toBe(true);
    expect(isProfileLink("Sam Lantern")).toBe(false);
    expect(isProfileLink("AC/DC fan")).toBe(false);
  });
});

describe("the Friends page", () => {
  beforeEach(() => {
    vi.spyOn(api.friends, "list").mockResolvedValue({ friends: [], incoming: [], outgoing: [] });
    vi.spyOn(api.friends, "blocks").mockResolvedValue([]);
    vi.spyOn(api.friends, "chatSettings").mockResolvedValue({ from_anyone: false, locked: false });
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  const box = () => screen.getByRole("textbox", { name: "Add a friend: their name, or their profile link" });

  it("finds people by name as you type, and asks one", async () => {
    const search = vi
      .spyOn(api.friends, "search")
      .mockResolvedValueOnce([person("Sam"), person("Sammy", "requested"), person("Pam Sam", "friends")])
      .mockResolvedValue([person("Sam", "requested"), person("Sammy", "requested"), person("Pam Sam", "friends")]);
    const ask = vi.spyOn(api.friends, "request").mockResolvedValue({ status: "requested" });
    render(<FriendsPage />, { wrapper: MemoryRouter });
    fireEvent.change(box(), { target: { value: "s" } });
    expect((screen.getByRole("button", { name: "Search" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(box(), { target: { value: "sam" } });
    const found = await screen.findByRole("list", { name: "Players found" });
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith("sam");
    const rows = within(found).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringMatching(/Sam joined October 2026＋ Add friend$/),
      expect.stringContaining("Request sent"),
      expect.stringContaining("✓ Friends"),
    ]);
    fireEvent.click(within(found).getByRole("button", { name: "Add Sam as a friend" }));
    await screen.findByText("Request sent to Sam. They'll see it in their notifications.");
    expect(ask).toHaveBeenCalledWith("id-Sam");
    // Looked again: Sam's row now says the request is sent.
    await waitFor(() => expect(within(found).getAllByText("Request sent")).toHaveLength(2));
  });

  it("says when nobody has that name, and why someone might not be found", async () => {
    vi.spyOn(api.friends, "search").mockResolvedValue([]);
    render(<FriendsPage />, { wrapper: MemoryRouter });
    fireEvent.change(box(), { target: { value: "Zed" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await screen.findByText(/Nobody by that name\. Someone in the under-18 view can't be found by name/);
  });

  it("sends a request for a pasted profile link, without searching", async () => {
    const search = vi.spyOn(api.friends, "search").mockResolvedValue([]);
    const ask = vi.spyOn(api.friends, "request").mockResolvedValue({ status: "requested" });
    render(<FriendsPage />, { wrapper: MemoryRouter });
    fireEvent.change(box(), { target: { value: "https://slaporslop.games/u/abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    await screen.findByText("Request sent. They'll see it in their notifications.");
    expect(ask).toHaveBeenCalledWith("https://slaporslop.games/u/abc");
    expect(search).not.toHaveBeenCalled();
  });

  it("shows your own profile link to copy", async () => {
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    render(<FriendsPage />, { wrapper: MemoryRouter });
    const link = screen.getByRole("textbox", { name: "Your profile link" }) as HTMLInputElement;
    expect(link.value).toBe(`${window.location.host}/u/01a1f6ac-11dc-77c1-8e41-c615060db719`);
    fireEvent.click(screen.getByRole("button", { name: "Copy link" }));
    await screen.findByRole("button", { name: "Copied" });
    expect(write).toHaveBeenCalledWith(`${window.location.origin}/u/01a1f6ac-11dc-77c1-8e41-c615060db719`);
  });
});
