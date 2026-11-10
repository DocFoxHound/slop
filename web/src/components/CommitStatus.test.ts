// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { CommitStatus } from "../api";
import { formatDateTime } from "../format";
import { commitStatusLine, commitStatusProblem } from "./CommitStatus";

const GITHUB = "https://github.com/apps/slaporslop/installations/new";

/** Switched on, installed, permission accepted, in the store, nothing posted yet. */
const on: CommitStatus = {
  on: true,
  since: "2026-11-12T09:30:00Z",
  installed: true,
  permission: true,
  github_url: GITHUB,
  in_store: true,
  problem: null,
  last: null,
};
const off: CommitStatus = { ...on, on: false, since: null, permission: null };
const passed: NonNullable<CommitStatus["last"]> = {
  commit_sha: "a".repeat(40),
  version: "1.2.0",
  state: "success",
  description: "Passed SLAPorSLOP's checks",
  posted_at: "2026-11-12T10:02:11Z",
};

function problem(code: number): CommitStatus {
  return { ...on, problem: { code, at: "2026-11-12T10:05:00Z" } };
}

describe("the line under the switch", () => {
  it("says nothing while it's off or the App isn't installed", () => {
    expect(commitStatusLine(off)).toBeNull();
    expect(commitStatusLine({ ...off, installed: false })).toBeNull();
    expect(commitStatusLine({ ...on, installed: false, last: passed })).toBeNull();
  });

  it("says nothing new is posted while the app is out of the store, even after something was", () => {
    // "New": the correction of a pass whose approval is gone (the hint's exception) can still go.
    const text = "Nothing new is posted while the app isn't in the store. When it is, the newest result is posted.";
    expect(commitStatusLine({ ...on, in_store: false })).toBe(text);
    expect(commitStatusLine({ ...on, in_store: false, last: passed })).toBe(text);
    expect(commitStatusLine({ ...on, in_store: false, installed: false })).toBe(text);
    expect(commitStatusLine({ ...on, in_store: false, permission: false, problem: { code: 403, at: "2026-11-12T10:05:00Z" } })).toBe(text);
  });

  it("names the last status posted, with its release's version when that release is still there", () => {
    const when = formatDateTime(passed.posted_at);
    expect(commitStatusLine({ ...on, last: passed })).toBe(`Last posted ${when} for 1.2.0: Passed SLAPorSLOP's checks.`);
    expect(commitStatusLine({ ...on, last: { ...passed, version: null } })).toBe(`Last posted ${when}: Passed SLAPorSLOP's checks.`);
  });

  it("says what happens next when nothing was posted yet", () => {
    expect(commitStatusLine(on)).toBe("Nothing posted yet: the next release you send gets a status.");
    // Not knowing about the permission yet is no reason to say anything else.
    expect(commitStatusLine({ ...on, permission: null })).toBe("Nothing posted yet: the next release you send gets a status.");
  });

  it("promises nothing while something on GitHub stops it, but still names what was posted", () => {
    expect(commitStatusLine({ ...on, permission: false })).toBeNull();
    expect(commitStatusLine(problem(403))).toBeNull();
    expect(commitStatusLine(problem(502))).toBeNull();
    const when = formatDateTime(passed.posted_at);
    expect(commitStatusLine({ ...problem(422), last: passed })).toBe(`Last posted ${when} for 1.2.0: Passed SLAPorSLOP's checks.`);
  });
});

describe("what stops statuses", () => {
  it("is nothing while it's off, whatever else is true", () => {
    expect(commitStatusProblem(off)).toBeNull();
    expect(commitStatusProblem({ ...off, installed: false })).toBeNull();
    expect(commitStatusProblem({ ...off, permission: false, problem: { code: 403, at: "2026-11-12T10:05:00Z" } })).toBeNull();
  });

  it("is nothing while the app is out of the store: Check again can't help there (a Private app can't even use it)", () => {
    expect(commitStatusProblem({ ...problem(403), in_store: false })).toBeNull();
    expect(commitStatusProblem({ ...on, in_store: false, permission: false })).toBeNull();
    expect(commitStatusProblem({ ...on, in_store: false, installed: false })).toBeNull();
  });

  it("is nothing when all is well", () => {
    expect(commitStatusProblem(on)).toBeNull();
    expect(commitStatusProblem({ ...on, permission: null, last: passed })).toBeNull();
  });

  it("an App no longer installed: install it again, from the link", () => {
    expect(commitStatusProblem({ ...on, installed: false })).toEqual({
      text: "SLAPorSLOP's GitHub App isn't installed on this repository any more, so nothing can be posted. Install it again, then press Check again.",
      link: GITHUB,
    });
  });

  it("a permission not accepted: exactly what to accept on GitHub and where", () => {
    const p = commitStatusProblem({ ...on, permission: false });
    expect(p?.link).toBe(GITHUB);
    expect(p?.text).toBe(
      "GitHub hasn't let SLAPorSLOP's App do this yet. The App asks for one more permission, Commit statuses: Read and write. " +
        "Whoever owns the repository on GitHub accepts it: in GitHub's Settings, under Applications (for an organization: GitHub " +
        "Apps), choose Configure next to SLAPorSLOP's App, then Review request. Then press Check again.",
    );
  });

  it("each refusal from GitHub says what it means and what to do", () => {
    expect(commitStatusProblem(problem(403))).toEqual({
      text:
        "GitHub refused the last status (403). Most often the App's Commit statuses permission hasn't been accepted: in GitHub's " +
        "Settings, under Applications, choose Configure next to SLAPorSLOP's App, then Review request. Then press Check again.",
      link: GITHUB,
    });
    expect(commitStatusProblem(problem(404))).toEqual({
      text:
        "GitHub couldn't find this repository for SLAPorSLOP's App (404): the App may no longer cover it. Add the repository to " +
        "the App on GitHub, then press Check again.",
      link: GITHUB,
    });
    expect(commitStatusProblem(problem(422))).toEqual({
      text:
        "GitHub doesn't have that release's commit in this repository (422), so there was nothing to put the status on. Later " +
        "releases are tried as usual.",
      link: null,
    });
  });

  it("a repository that moved on GitHub (3xx): change the address here, then switch it on again", () => {
    const moved = {
      text:
        "This repository has moved on GitHub, so the status had nowhere to go. Change the repository address under Repository " +
        "on this page to the new one, then switch this on again.",
      link: null,
    };
    for (const code of [300, 301, 307, 308, 399]) expect(commitStatusProblem(problem(code))).toEqual(moved);
  });

  it("any other refusal says GitHub refused it, with its code: it stopped at GitHub's first answer", () => {
    for (const code of [400, 405, 409, 410, 413, 451, 499]) {
      expect(commitStatusProblem(problem(code))).toEqual({
        text: `GitHub refused the last status (${code}). Press Check again to try once more.`,
        link: null,
      });
    }
  });

  it("no answer, GitHub's errors and a token refused twice are GitHub not answering after every try", () => {
    const tried = { text: "SLAPorSLOP couldn't reach GitHub after several tries. Press Check again to try once more.", link: null };
    for (const code of [0, 401, 500, 502, 503]) expect(commitStatusProblem(problem(code))).toEqual(tried);
  });

  it("goes in order: not installed, then the permission, then GitHub's last refusal", () => {
    const notInstalled = commitStatusProblem({ ...on, installed: false })?.text;
    const permission = commitStatusProblem({ ...on, permission: false })?.text;
    expect(commitStatusProblem({ ...problem(403), installed: false, permission: false })?.text).toBe(notInstalled);
    expect(commitStatusProblem({ ...problem(422), permission: false })?.text).toBe(permission);
    expect(commitStatusProblem({ ...problem(502), permission: null })?.text).toMatch(/^SLAPorSLOP couldn't reach GitHub/);
  });

  it("never makes a trust claim", () => {
    const all = [
      commitStatusLine({ ...on, in_store: false }),
      commitStatusLine(on),
      commitStatusProblem({ ...on, installed: false })?.text,
      commitStatusProblem({ ...on, permission: false })?.text,
      ...[403, 404, 422, 301, 410, 502].map((code) => commitStatusProblem(problem(code))?.text),
    ];
    for (const text of all) expect(text).not.toMatch(/safe|secure|verified|trusted|guarantee/i);
  });
});
