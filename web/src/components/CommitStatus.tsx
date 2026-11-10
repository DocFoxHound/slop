// SPDX-License-Identifier: AGPL-3.0-or-later
// Results on GitHub, on Manage: the maker's switch for SLAPorSLOP's result as a commit status on
// each release's GitHub commit (crates/slop-api/src/commit_status.rs; DESIGN §8). The panel shows
// only where the server offers it for this app (the Manage answer's `commit_status`). It says what
// was posted last, and when something on GitHub stops it, exactly what to do there, with Check
// again to try once more from here.

import { useEffect, useRef, useState } from "react";

import { api, ApiError, type CommitStatus } from "../api";
import { formatDateTime } from "../format";
import { ExternalLink } from "./ui";

/** Shown before the maker switches it on: what is posted, and that anyone can see it. */
const HINT =
  "For each release you send from now on, SLAPorSLOP puts its result on the release's commit on GitHub, as a status named " +
  "SLAPorSLOP: being checked, waiting for review, passed, not approved, couldn't be checked, or withdrawn. Anyone who can see " +
  "the commit on GitHub sees it, also when a release isn't approved or is withdrawn. It never says why. Switching this off " +
  "stops new ones; those already on GitHub stay, also if the app later leaves SLAPorSLOP or your account is closed. One " +
  "exception: if a release that showed as passed is later withdrawn by SLAPorSLOP, the status is corrected even after you " +
  "switched this off.";

/** The line under the switch: what was posted last, or why nothing is. Exported for tests. */
export function commitStatusLine(c: CommitStatus): string | null {
  if (!c.on) return null;
  // Out of the store only the correction in the hint can still be posted (never for a Private
  // app), so the line promises nothing new.
  if (!c.in_store) return "Nothing new is posted while the app isn't in the store. When it is, the newest result is posted.";
  if (!c.installed) return null;
  if (c.last) {
    return `Last posted ${formatDateTime(c.last.posted_at)}${c.last.version ? ` for ${c.last.version}` : ""}: ${c.last.description}.`;
  }
  // Something on GitHub stops it: the notice says what, and nothing is promised.
  if (commitStatusProblem(c)) return null;
  return "Nothing posted yet: the next release you send gets a status.";
}

/** While it's on and the app is in the store, what stops statuses and how to fix it; `link` is
 *  where on GitHub. Out of the store there is nothing to fix here (for a Private app Check again
 *  can't work at all), so nothing shows. Exported for tests. */
export function commitStatusProblem(c: CommitStatus): { text: string; link: string | null } | null {
  if (!c.on || !c.in_store) return null;
  if (!c.installed) {
    return {
      text: "SLAPorSLOP's GitHub App isn't installed on this repository any more, so nothing can be posted. Install it again, then press Check again.",
      link: c.github_url,
    };
  }
  if (c.permission === false) {
    return {
      text:
        "GitHub hasn't let SLAPorSLOP's App do this yet. The App asks for one more permission, Commit statuses: Read and write. " +
        "Whoever owns the repository on GitHub accepts it: in GitHub's Settings, under Applications (for an organization: GitHub " +
        "Apps), choose Configure next to SLAPorSLOP's App, then Review request. Then press Check again.",
      link: c.github_url,
    };
  }
  const code = c.problem?.code;
  if (code === undefined) return null;
  if (code === 403) {
    return {
      text:
        "GitHub refused the last status (403). Most often the App's Commit statuses permission hasn't been accepted: in GitHub's " +
        "Settings, under Applications, choose Configure next to SLAPorSLOP's App, then Review request. Then press Check again.",
      link: c.github_url,
    };
  }
  if (code === 404) {
    return {
      text:
        "GitHub couldn't find this repository for SLAPorSLOP's App (404): the App may no longer cover it. Add the repository to " +
        "the App on GitHub, then press Check again.",
      link: c.github_url,
    };
  }
  if (code === 422) {
    return {
      text:
        "GitHub doesn't have that release's commit in this repository (422), so there was nothing to put the status on. Later " +
        "releases are tried as usual.",
      link: null,
    };
  }
  // GitHub answered with a redirect, which is never followed: the repository moved. Changing the
  // address switches this off, so the maker switches it on again after.
  if (code >= 300 && code < 400) {
    return {
      text:
        "This repository has moved on GitHub, so the status had nowhere to go. Change the repository address under Repository " +
        "on this page to the new one, then switch this on again.",
      link: null,
    };
  }
  // Any other refusal stops at GitHub's first answer (a 401 is tried again, so it's below).
  if (code >= 400 && code < 500 && code !== 401) {
    return { text: `GitHub refused the last status (${code}). Press Check again to try once more.`, link: null };
  }
  // No answer (0), GitHub's errors and a token refused again: tried 5 times in all.
  return { text: "SLAPorSLOP couldn't reach GitHub after several tries. Press Check again to try once more.", link: null };
}

export function CommitStatusPanel({ slug, status, onChanged }: { slug: string; status: CommitStatus; onChanged: () => void }) {
  const [fresh, setFresh] = useState<CommitStatus>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Check again found the same problem: said in the live region, or the press would seem to do nothing.
  const [unchanged, setUnchanged] = useState(false);
  const c = fresh ?? status;
  const line = commitStatusLine(c);
  const problem = commitStatusProblem(c);
  const locked = !c.on && !c.installed;
  const section = useRef<HTMLElement>(null);
  const box = useRef<HTMLInputElement>(null);
  const install = useRef<HTMLParagraphElement>(null);
  const wasLocked = useRef(locked);

  // The switch can't be used until the App is installed (for example after switching off once the
  // App was removed). A focused control that becomes disabled loses focus, so it moves to the
  // install link next to it.
  useEffect(() => {
    if (locked && !wasLocked.current && (document.activeElement === box.current || document.activeElement === document.body)) {
      install.current?.querySelector("a")?.focus();
    }
    wasLocked.current = locked;
  }, [locked]);

  // Switching on or off; `true` while it's on is Check again. While busy the controls are
  // aria-disabled, not disabled: a focused control that becomes disabled loses focus.
  const send = async (on: boolean, again = false) => {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    setUnchanged(false);
    try {
      const answer = await api.setCommitStatus(slug, on);
      setFresh(answer);
      // Check again's button goes once nothing is wrong: focus moves to the switch, not the page.
      if (again && !commitStatusProblem(answer)) box.current?.focus();
      if (again && commitStatusProblem(answer)) setUnchanged(true);
      onChanged();
    } catch (err) {
      if (err instanceof ApiError && err.status === 404) {
        // Not offered for this app any more: the page reads the overview again and the panel
        // goes, so focus moves to the page's content instead of being lost with it.
        if (section.current?.contains(document.activeElement)) document.getElementById("main")?.focus();
        onChanged();
        return;
      }
      setError(err instanceof ApiError ? err.message : String(err));
      // The overview is out of date (the App was removed, or the repository changed): read it
      // again, so the panel shows what to do.
      if (err instanceof ApiError && err.status === 409) {
        setFresh(undefined);
        onChanged();
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <section
      ref={section}
      className="panel panel-pad form commit-status-panel"
      aria-labelledby="commit-status-title"
      aria-busy={busy}
    >
      <h2 id="commit-status-title">Results on GitHub</h2>
      <label className="check">
        <input
          ref={box}
          type="checkbox"
          checked={c.on}
          disabled={locked}
          aria-disabled={busy || undefined}
          onChange={(e) => void send(e.target.checked)}
        />
        <span>
          <b>Show SLAPorSLOP's result on each commit</b>
          <span className="muted small" style={{ display: "block" }}>
            {HINT}
          </span>
        </span>
      </label>
      {/* Always there, so what switching or Check again leads to is announced when it changes. */}
      <div className="commit-status-live" role="status">
        {locked && (
          <p className="notice" ref={install}>
            To use this, install SLAPorSLOP's GitHub App on this repository first.{" "}
            <ExternalLink href={c.github_url}>Install on GitHub</ExternalLink>
          </p>
        )}
        {line && <p className="muted small updates-status">{line}</p>}
        {unchanged && problem && <p className="muted small">Checked again: nothing has changed on GitHub yet.</p>}
        {problem && (
          <p className="notice">
            {problem.text}
            {problem.link && (
              <>
                {" "}
                <ExternalLink href={problem.link}>{c.installed ? "Open the App on GitHub" : "Install on GitHub"}</ExternalLink>
              </>
            )}
          </p>
        )}
      </div>
      {c.on && problem && (
        <div className="row-actions">
          <button type="button" className="btn" aria-disabled={busy || undefined} onClick={() => void send(true, true)}>
            Check again
          </button>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
