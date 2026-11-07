// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing progress as the maker sees it: which lists of files or checks fold away. A download's
// files that weren't checked show open, as they always did; a browser game's checks that a test
// setup doesn't make fold away with the rest.
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { Release, ReleaseFile } from "../api";
import { PublishProgress } from "./PublishProgress";

afterEach(cleanup);

const server = { scanning: false, maker_signatures_required: false };

const file = (over: Partial<ReleaseFile> = {}): ReleaseFile => ({
  platform: "windows",
  kind: "archive",
  url: "https://github.com/me/game/releases/download/v1/game.zip",
  file_name: "game.zip",
  exec: "game.exe",
  args: [],
  size_bytes: 10,
  sha256: null,
  provenance: null,
  provenance_problem: null,
  maker_key: null,
  scan_result: null,
  scan_detail: null,
  scan_report: null,
  defender: null,
  cleared_reason: null,
  ...over,
});

const release = (over: Partial<Release> = {}): Release => ({
  id: "r1",
  version: "1.0.0",
  commit_sha: "a".repeat(40),
  status: "failed",
  status_reason: "the download answered HTTP 404",
  commit_verified: false,
  created_at: "2026-10-18T00:00:00Z",
  source: "maker",
  auto_approved: false,
  files: [file({ check: "not_checked" }), file({ platform: "linux", file_name: "game.tar.gz", check: "not_checked" })],
  source_checks: null,
  repo_scan: null,
  advisories: [],
  ...over,
});

function show(releases: Release[], plays_in_browser = false) {
  render(
    <MemoryRouter>
      <PublishProgress app={{ slug: "game", stage: "beta", status: "draft", plays_in_browser }} releases={releases} server={server} />
    </MemoryRouter>,
  );
}

describe("Publishing progress's lists", () => {
  it("shows a download's files that weren't checked, open", () => {
    show([release()]);
    const files = screen.getByRole("list", { name: "Files checked: each file" });
    expect(files.closest("details")).toBeNull();
    expect(files.textContent).toContain("Windows (.zip) Not checked");
    expect(files.textContent).toContain("Linux (.tar.gz) Not checked");
  });

  it("folds a browser game's checks that passed, or that a test setup doesn't make", () => {
    show(
      [
        release({
          status: "awaiting_review",
          status_reason: undefined,
          files: [],
          play_address: "http://127.0.0.1:9/play/",
          address_checks: {
            https: false,
            public_host: null,
            public_host_skipped: "test_setup",
            bad_links: { listed_by: null, lists_loaded: true },
            status: 200,
            final_address: "http://127.0.0.1:9/play/",
            redirects: 0,
            content_type: "text/html",
            checked_at: "2026-11-07T10:00:00Z",
            ok: true,
            problem: null,
          },
        }),
      ],
      true,
    );
    const checks = screen.getByRole("list", { name: "Address checked: each check" });
    expect(checks.closest("details")).not.toBeNull();
  });
});
