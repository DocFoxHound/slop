// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AdminQueue, ReleaseFile } from "../../api";
import { BOXES, boxCounts, defaultBox, inboxUrl, releaseBlurb, rowsFor, sortRows, totalWaiting, waited } from "./queue";

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
  cleared_reason: null,
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
