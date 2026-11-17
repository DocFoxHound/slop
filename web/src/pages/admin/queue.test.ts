// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AdminQueue, ReleaseFile } from "../../api";
import { BOXES, boxCounts, defaultBox, holdStatus, inboxUrl, releaseBlurb, rowsFor, sortRows, totalWaiting, waited } from "./queue";

const NOW = Date.parse("2026-10-25T12:00:00Z");

const file = (over: Partial<ReleaseFile>): ReleaseFile => ({
  platform: "windows",
  kind: "archive",
  url: "https://example.com/a.zip",
  file_name: "a.zip",
  exec: "a.exe",
  args: [],
  size_bytes: 10,
  sha256: "0".repeat(64),
  provenance: { built_by_slop_build: true } as ReleaseFile["provenance"],
  provenance_problem: null,
  maker_key: null,
  scan_result: "passed",
  scan_detail: null,
  scan_report: null,
  defender: { sha256: "", engine: "", signatures: "", signatures_updated: null, detections: [], signed_by: null },
  cleared: false,
  cleared_reason: null,
  ...over,
});

const hold = (over: object = {}) => ({
  release_id: "r",
  version: "1.0.0",
  release_status: "awaiting_review",
  status: "open",
  kinds: ["scan"],
  needs_staff: true,
  problems: [],
  you_can: [],
  rescans_left: 3,
  next_rescan_at: null,
  rescan_requested: false,
  rescan_running: false,
  asked_staff_at: null,
  unanswered: false,
  messages: [],
  ...over,
});

const release = (id: string, created_at: string, files: ReleaseFile[], over: object = {}) =>
  ({
    id,
    version: "1.0.0",
    commit_sha: "a".repeat(40),
    status: "awaiting_review",
    commit_verified: true,
    created_at,
    source: "maker",
    auto_approved: false,
    files,
    source_checks: null,
    repo_scan: null,
    advisories: [],
    app: { slug: "golf", title: "Pocket Golf", stage: "released", status: "listed", repo_url: "https://github.com/o/golf", maker: "Bo" },
    hold: hold(),
    notes: [],
    ...over,
  }) as unknown as AdminQueue["releases"][number];

const queue = (over: Partial<AdminQueue>): AdminQueue =>
  ({
    abuse: [],
    rescans: [],
    releases: [],
    changes: [],
    media: [],
    portfolios: [],
    problems: [],
    review_disputes: [],
    ready_to_list: [],
    reports: [],
    appeals: [],
    rules: [],
    checks: {},
    ...over,
  }) as AdminQueue;

describe("the review inbox", () => {
  it("counts each category, leaving out what this staff member can't see", () => {
    const counts = boxCounts({
      queue: queue({ releases: [release("r1", "2026-10-20T00:00:00Z", [])] }),
      actions: { approvals_required: 1, actions: [{ status: "pending" }, { status: "done" }] } as never,
      ai: [{ status: "open" }, { status: "closed" }] as never,
      holds: [{ lifted_at: null, requests: [{ outcome: null }] }, { lifted_at: null, requests: [] }] as never,
    });
    expect(counts.releases).toBe(1);
    expect(counts.people).toBe(0);
    expect(counts.approvals).toBe(1);
    expect(counts.ai).toBe(1);
    expect(counts.donations).toBe(1);
    // Legal requests weren't loaded (not for this role): unknown, not 0.
    expect(counts.legal).toBeUndefined();
    expect(totalWaiting(counts)).toBe(4);
    expect(defaultBox(counts)).toBe("releases");
    expect(defaultBox({ ...counts, releases: 0, changes: 0 })).toBe("approvals");
    // Every category has a label, a group and a help line.
    for (const b of BOXES) expect(b.label && b.group && b.help).toBeTruthy();
  });

  it("sums up a release in a line", () => {
    expect(releaseBlurb(release("r", "2026-10-20T00:00:00Z", [file({})]))).toBe("1 file · scans passed · Defender clean · signed");
    expect(
      releaseBlurb(
        release("r", "2026-10-20T00:00:00Z", [
          file({ scan_result: "on_hold", defender: null, provenance: null }),
          file({ platform: "linux", scan_result: "passed", defender: null }),
        ]),
      ),
    ).toBe("2 files · scan found something · no Defender check · not signed");
    expect(releaseBlurb(release("r", "2026-10-20T00:00:00Z", [file({ scan_result: null })], { notes: [{}, {}] }))).toBe(
      "1 file · scan pending · Defender clean · signed · 2 notes",
    );
    expect(releaseBlurb(release("r", "2026-10-20T00:00:00Z", [file({})], { status: "fetching" }))).toBe("1 file · still fetching");
    const [row] = rowsFor("releases", { queue: queue({ releases: [release("r", "2026-10-20T00:00:00Z", [file({ scan_result: "on_hold" })])] }) });
    expect(row).toMatchObject({ key: "r", title: "Pocket Golf 1.0.0", by: "by Bo", status: { label: "Scan hold", tone: "warn" } });
    expect(row.since).toBe("2026-10-20T00:00:00Z");
  });

  it("says what each hold is about, and waits from when the maker last wrote", () => {
    expect(holdStatus({ kinds: ["scan", "defender"] })).toEqual({ label: "Scan hold", tone: "warn" });
    expect(holdStatus({ kinds: ["defender"] })).toEqual({ label: "Defender hold", tone: "warn" });
    expect(holdStatus({ kinds: ["rating_lowered"] })).toEqual({ label: "Lower rating", tone: "warn" });
    expect(holdStatus({ kinds: ["defender_missing"] })).toEqual({ label: "No Defender check", tone: "neutral" });
    expect(holdStatus({ kinds: ["question"] })).toEqual({ label: "Question", tone: "soon" });
    const wrote = release("r", "2026-10-20T00:00:00Z", [file({ scan_result: "on_hold" })], {
      hold: hold({ unanswered: true, asked_staff_at: "2026-10-24T00:00:00Z" }),
    });
    expect(releaseBlurb(wrote)).toBe("1 file · scan found something · Defender clean · signed · the maker wrote");
    expect(rowsFor("releases", { queue: queue({ releases: [wrote] }) })[0].since).toBe("2026-10-24T00:00:00Z");
    // A lowered rating says from and to; a question about a failed release says why it failed.
    const rating = release("r", "2026-10-20T00:00:00Z", [file({})], {
      hold: hold({ kinds: ["rating_lowered"], rating: { from_age: 16, to_age: 12, dropped_labels: [], content_labels: [] } }),
    });
    expect(releaseBlurb(rating)).toBe("1 file · scans passed · Defender clean · signed · rating 16+ → 12+");
    const question = release("r", "2026-10-20T00:00:00Z", [file({})], {
      status: "failed",
      status_reason: "Windows (.zip) build has no maker signature: no attestation.",
      hold: hold({ kinds: ["question"], release_status: "failed", unanswered: true }),
    });
    expect(releaseBlurb(question)).toBe("Failed · Windows (.zip) build has no maker signature: no attestation · the maker asks about it");
    expect(rowsFor("releases", { queue: queue({ releases: [question] }) })[0].status).toEqual({ label: "Question", tone: "soon" });
  });

  it("sums up a browser game's release by its play address: never files or signing", () => {
    const checks = {
      https: true,
      public_host: true,
      bad_links: { listed_by: null, lists_loaded: true },
      status: 200,
      final_address: "https://games.example.com/v2/",
      redirects: 0,
      content_type: "text/html",
      checked_at: "2026-10-20T00:00:00Z",
      ok: true,
      problem: null,
    };
    const address = (over: object) =>
      release("p", "2026-10-20T00:00:00Z", [], {
        play_address: "https://games.example.com/v2/",
        address_checks: checks,
        current_play_address: null,
        app: { slug: "tap", title: "Orbit Tap", stage: "beta", status: "draft", repo_url: "https://github.com/o/tap", maker: "Sam", platforms: ["browser"] },
        ...over,
      });
    expect(releaseBlurb(address({}))).toBe("Plays in the browser · games.example.com · address checks passed · no address published before");
    // Another address on the same host isn't "the same site": it can be someone else's game.
    expect(releaseBlurb(address({ current_play_address: "https://games.example.com/play/" }))).toBe(
      "Plays in the browser · games.example.com · address checks passed · same host",
    );
    expect(releaseBlurb(address({ current_play_address: "https://games.example.com/v2/" }))).toBe(
      "Plays in the browser · games.example.com · address checks passed · same address",
    );
    expect(releaseBlurb(address({ current_play_address: "https://old.example.net/", notes: [{}] }))).toBe(
      "Plays in the browser · games.example.com · address checks passed · new site · 1 note",
    );
    expect(releaseBlurb(address({ status: "fetching", address_checks: null }))).toBe(
      "Plays in the browser · games.example.com · still checking · no address published before",
    );
    // After SLAPorSLOP withdrew the address Play opened, it's never a first address.
    expect(releaseBlurb(address({ withdrawn_play_address: "https://games.example.com/v2/" }))).toBe(
      "Plays in the browser · games.example.com · address checks passed · the withdrawn address again",
    );
    expect(releaseBlurb(address({ withdrawn_play_address: "https://games.example.com/old/" }))).toContain("same host as a withdrawn address");
    expect(releaseBlurb(address({ withdrawn_play_address: "https://old.example.net/" }))).toContain("new site after a withdrawal");
    expect(
      releaseBlurb(
        address({
          current_play_address: "https://games.example.com/v3/",
          newer_approved: { version: "1.3.0", play_address: "https://games.example.com/v3/", note: "Version 1.3.0 is approved and newer" },
        }),
      ),
    ).toBe("Plays in the browser · games.example.com · address checks passed · same host · older than published 1.3.0");
    expect(
      releaseBlurb(address({ status: "failed", address_checks: { ...checks, ok: false, bad_links: { listed_by: "URLhaus", lists_loaded: true } } })),
    ).toContain("on URLhaus");
    for (const r of [address({}), address({ status: "failed", address_checks: { ...checks, ok: false } })]) {
      expect(releaseBlurb(r)).not.toMatch(/file|sign|scan|Defender/);
    }
    const [held] = rowsFor("releases", {
      queue: queue({ releases: [address({ hold: hold({ kinds: ["rating_lowered"] }) })] }),
    });
    expect(held.status).toEqual({ label: "Lower rating", tone: "warn" });
    const [ready] = rowsFor("listing", {
      queue: queue({
        ready_to_list: [
          {
            slug: "tap",
            title: "Orbit Tap",
            stage: "beta",
            maker: "Sam",
            platforms: ["browser"],
            play_address: "https://games.example.com/play/",
            approved_releases: 1,
            pictures_pending: false,
            going_public: false,
            after_appeal: false,
            waiting_since: "2026-10-20T00:00:00Z",
          },
        ] as never,
      }),
    });
    expect(ready.blurb).toBe("Beta · 1 published release · plays at games.example.com");
  });

  it("says how long something waited, and sorts by it", () => {
    expect(waited("2026-10-25T11:59:40Z", NOW)).toBe("just now");
    expect(waited("2026-10-25T11:48:00Z", NOW)).toBe("12 min");
    expect(waited("2026-10-24T07:00:00Z", NOW)).toBe("29 h");
    expect(waited("2026-10-23T11:00:00Z", NOW)).toBe("2 days");
    const rows = [
      { key: "b", since: "2026-10-22T00:00:00Z" },
      { key: "a", since: "2026-10-20T00:00:00Z" },
      { key: "c", since: "2026-10-24T00:00:00Z" },
    ];
    expect(sortRows(rows, "oldest").map((r) => r.key)).toEqual(["a", "b", "c"]);
    expect(sortRows(rows, "newest").map((r) => r.key)).toEqual(["c", "b", "a"]);
  });

  it("puts the category and the item in the address", () => {
    expect(inboxUrl()).toBe("/admin?tab=queue");
    expect(inboxUrl("people")).toBe("/admin?tab=queue&box=people");
    expect(inboxUrl("people", "abc", "newest")).toBe("/admin?tab=queue&box=people&item=abc&sort=newest");
  });

  it("gives People pages waiting for approval a line each", () => {
    const [row] = rowsFor("people", {
      queue: queue({
        portfolios: [
          {
            handle: "remy",
            name: "Remy",
            ai_use: "none",
            set: { id: "s1", works: [{}, {}], avatar: {}, banner: null, created_at: "2026-10-24T00:00:00Z" },
            live: null,
            notes: [],
          },
        ] as never,
      }),
    });
    expect(row).toMatchObject({
      key: "s1",
      title: "Remy",
      by: "@remy",
      blurb: "2 works · avatar · AI use: none",
      status: { label: "First portfolio" },
    });
  });
});
