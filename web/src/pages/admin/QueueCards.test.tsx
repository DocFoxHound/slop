// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue's card for a browser game's release, its play address: what the reviewer gets
// to judge it by, and nothing about files, scans or signing.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type AddressChecks, type AdminQueue } from "../../api";
import { ReadyCard, ReleaseCard } from "./QueueCards";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const checks = (over: Partial<AddressChecks> = {}): AddressChecks => ({
  https: true,
  public_host: true,
  bad_links: { listed_by: null, lists_loaded: false },
  status: 200,
  final_address: "https://new.example.net/tap/",
  redirects: 0,
  content_type: "text/html",
  checked_at: "2026-11-07T10:00:00Z",
  ok: true,
  problem: null,
  ...over,
});

const address = (over: object = {}) =>
  ({
    id: "rel1",
    version: "1.1.0",
    commit_sha: "a".repeat(40),
    status: "awaiting_review",
    status_reason: "Waiting for a person: a new play address is always checked by a person",
    commit_verified: true,
    created_at: "2026-11-07T10:00:00Z",
    source: "maker",
    auto_approved: false,
    files: [],
    play_address: "https://new.example.net/tap/",
    address_checks: checks(),
    current_play_address: "https://games.example.com/play/",
    source_checks: null,
    repo_scan: null,
    advisories: [],
    app: {
      slug: "orbit-tap",
      title: "Orbit Tap",
      stage: "released",
      status: "listed",
      repo_url: "https://github.com/sam/orbit-tap",
      maker: "Sam Seed",
      platforms: ["browser"],
    },
    notes: [],
    ...over,
  }) as unknown as AdminQueue["releases"][number];

const show = (r: AdminQueue["releases"][number]) =>
  render(
    <MemoryRouter>
      <ul>
        <ReleaseCard r={r} onDone={() => {}} />
      </ul>
    </MemoryRouter>,
  );

describe("a browser game's release in the review queue", () => {
  it("gives the full address to open and play, in a new tab with no referrer, and where Play goes now", () => {
    show(address());
    const card = screen.getAllByRole("listitem")[0];
    expect(within(card).getByText("https://new.example.net/tap/", { selector: "code" })).toBeTruthy();
    const open = within(card).getByRole("link", { name: "Open the game" });
    expect(open.getAttribute("href")).toBe("https://new.example.net/tap/");
    expect(open.getAttribute("target")).toBe("_blank");
    expect(open.getAttribute("rel")).toBe("noopener noreferrer");
    expect(card.textContent).toContain("Open it in a private window; don't sign in or type anything there.");
    expect(within(card).getByText("https://games.example.com/play/")).toBeTruthy();
    expect(within(card).getByText("NEW SITE")).toBeTruthy();
    expect(card.textContent).toContain("SLAPorSLOP can't check that the site runs this code.");
    // Each check in words, and no claim about bad-link lists this server doesn't have.
    expect(card.textContent).toContain("No bad-link lists are loaded on this server");
    expect(card.textContent).toContain("The page loads: Yes (HTTP 200)");
    // Nothing about files, scans or signing; approving approves the address.
    expect(card.textContent).not.toMatch(/SIGNED|SCAN|SHA-256|Defender|\.zip/);
    expect(within(card).queryByRole("button", { name: "Approve and sign" })).toBeNull();
    expect(within(card).getByRole("button", { name: "Approve address" })).toBeTruthy();
  });

  it("approves it through the usual route; Copy puts the address on the clipboard", async () => {
    const approve = vi.spyOn(api.admin, "approve").mockResolvedValue(undefined as never);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    show(address());
    fireEvent.click(screen.getByRole("button", { name: "Copy address" }));
    expect(writeText).toHaveBeenCalledWith("https://new.example.net/tap/");
    expect(await screen.findByText("Copied.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Approve address" }));
    expect(approve).toHaveBeenCalledWith("rel1");
  });

  it("another address on the same host isn't called the same site; nothing approved before has nothing to compare; checks still running can't be approved", () => {
    show(address({ current_play_address: "https://new.example.net/old/" }));
    expect(screen.queryByText("NEW SITE")).toBeNull();
    expect(screen.getByText("(the same host, another address: check it's still this game)")).toBeTruthy();
    expect(screen.queryByText(/same site/)).toBeNull();
    cleanup();
    show(address({ current_play_address: "https://new.example.net/tap/" }));
    expect(screen.getByText("(the same address)")).toBeTruthy();
    cleanup();
    show(address({ current_play_address: null, status: "fetching", address_checks: null }));
    expect(screen.getByText("Nothing yet: no address of this game has been approved before.")).toBeTruthy();
    expect(screen.getByText("CHECKING")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Approve address" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("after a withdrawal, says Play opens nothing and compares the address with the withdrawn one; never a first address", () => {
    show(address({ current_play_address: null, withdrawn_play_address: "https://new.example.net/tap/" }));
    const card = screen.getAllByRole("listitem")[0];
    expect(card.textContent).toContain("Nothing: SLAPorSLOP withdrew https://new.example.net/tap/");
    expect(within(card).getByText("THE WITHDRAWN ADDRESS")).toBeTruthy();
    expect(card.textContent).not.toMatch(/first address|approved before/);
    cleanup();
    show(address({ current_play_address: null, withdrawn_play_address: "https://new.example.net/other/" }));
    expect(screen.getByText("SAME HOST AS THE WITHDRAWN ONE")).toBeTruthy();
    cleanup();
    show(address({ current_play_address: null, withdrawn_play_address: "https://games.example.com/play/" }));
    expect(screen.getByText("(a different site)")).toBeTruthy();
  });

  it("says when approving it moves Play back from a newer approved address", () => {
    const note = "Version 1.2.0 (https://games.example.com/v2/) is approved and newer: approving this makes Play open this older address";
    show(address({ newer_approved: { version: "1.2.0", play_address: "https://games.example.com/v2/", note } }));
    expect(screen.getByText(`${note}.`)).toBeTruthy();
  });

  it("a download's release keeps its own card", () => {
    show(address({ play_address: null, address_checks: null, current_play_address: null, app: { ...address().app, platforms: ["windows"] } }));
    expect(screen.getByRole("button", { name: "Approve and sign" })).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Open the game" })).toBeNull();
  });
});

describe("a browser game waiting to be listed", () => {
  const ready = (over: object = {}) =>
    ({
      slug: "orbit-tap",
      title: "Orbit Tap",
      stage: "beta",
      status: "draft",
      repo_url: "https://github.com/sam/orbit-tap",
      maker: "Sam Seed",
      platforms: ["browser"],
      play_address: "https://games.example.com/play/",
      approved_releases: 1,
      pictures_pending: false,
      going_public: false,
      after_appeal: false,
      waiting_since: "2026-11-07T10:00:00Z",
      notes: [],
      ...over,
    }) as unknown as AdminQueue["ready_to_list"][number];

  it("says where it plays, for a last look at the site", () => {
    render(
      <MemoryRouter>
        <ul>
          <ReadyCard a={ready()} onDone={() => {}} />
        </ul>
      </MemoryRouter>,
    );
    expect(screen.getByRole("link", { name: "https://games.example.com/play/" }).getAttribute("rel")).toBe("noopener noreferrer");
    expect(screen.getAllByRole("listitem")[0].textContent).toContain("Plays at https://games.example.com/play/");
  });
});
