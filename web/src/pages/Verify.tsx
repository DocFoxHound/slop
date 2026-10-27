// SPDX-License-Identifier: AGPL-3.0-or-later
// "Does SLAPorSLOP know this file?" (docs/standard/file-verification.md): a player picks a download
// or pastes its SHA-256. The file is hashed here, in the browser, and never uploaded.

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type FileVerification } from "../api";
import { useDocumentTitle } from "../hooks";

/** Bigger files would have to be read into memory at once: ask for their hash instead. */
const MAX_HASH_BYTES = 1024 * 1024 * 1024;

const SCAN: Record<string, string> = {
  passed: "scanned, nothing found",
  note: "scanned; a person checked something it noticed",
  on_hold: "a re-scan found something a person is checking",
  rejected: "rejected by the scan",
  blocked: "blocked by the scan",
  not_scanned: "not scanned (this SLAPorSLOP doesn't scan files)",
};

async function sha256Hex(file: File): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", await file.arrayBuffer());
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

export function Verify() {
  useDocumentTitle("Check a file");
  const [hash, setHash] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [answer, setAnswer] = useState<FileVerification | null>(null);

  const check = async (sha256: string) => {
    setBusy(true);
    setError(null);
    setAnswer(null);
    try {
      setAnswer(await api.verifyFile(sha256));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't check the file. Try again.");
    } finally {
      setBusy(false);
    }
  };

  const pick = async (file: File | undefined) => {
    if (!file) return;
    if (file.size > MAX_HASH_BYTES) {
      setError(
        "That file is over 1 GB, too big to check in the browser. Paste its SHA-256 instead: on Windows, Get-FileHash in PowerShell; on Linux or macOS, sha256sum.",
      );
      return;
    }
    setBusy(true);
    setError(null);
    const sha256 = await sha256Hex(file);
    setHash(sha256);
    await check(sha256);
  };

  const pasted = hash.trim().toLowerCase();
  return (
    <div className="page narrow doc">
      <h1>Check a file</h1>
      <p>
        Got a game or app from somewhere else? See whether it's a build SLAPorSLOP scanned and approved, or one SLAPorSLOP withdrew.
        Your file stays on your computer: only its fingerprint (SHA-256) is sent.
      </p>
      <p>
        <label>
          Choose the file{" "}
          <input type="file" disabled={busy} onChange={(e) => void pick(e.target.files?.[0])} />
        </label>
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void check(pasted);
        }}
      >
        <label htmlFor="verify-hash">Or paste its SHA-256</label>{" "}
        <input
          id="verify-hash"
          value={hash}
          onChange={(e) => setHash(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          size={66}
        />{" "}
        <button className="btn small" disabled={busy || !/^[0-9a-f]{64}$/.test(pasted)}>
          Check
        </button>
      </form>
      <div role="status" aria-live="polite">
        {busy && <p>Checking…</p>}
        {error && <p className="error">{error}</p>}
        {answer &&
          (answer.matches.length === 0 ? (
            <p>
              <strong>SLAPorSLOP doesn't know this file.</strong> It isn't a build of any public app on SLAPorSLOP. That doesn't make
              it bad, but SLAPorSLOP hasn't scanned it.
            </p>
          ) : (
            <ul>
              {answer.matches.map((m) => (
                <li key={`${m.release_id}-${m.platform}`}>
                  {m.status === "revoked" ? (
                    <strong>Withdrawn by SLAPorSLOP</strong>
                  ) : (
                    <strong>Approved on SLAPorSLOP</strong>
                  )}
                  : <Link to={`/app/${m.app_slug}`}>{m.app_title}</Link> {m.version} by {m.maker}, for {m.platform}.{" "}
                  {m.status === "revoked"
                    ? `Withdrawn ${new Date(m.revoked_at ?? m.approved_at).toLocaleDateString()}: don't run it.`
                    : `Approved ${new Date(m.approved_at).toLocaleDateString()}; ${SCAN[m.scan] ?? m.scan}${
                        m.defender ? `; Microsoft Defender: ${m.defender === "clean" ? "nothing found" : "reviewed"}` : ""
                      }.`}
                </li>
              ))}
            </ul>
          ))}
      </div>
      <p className="muted small">
        Other stores and launchers can ask the same question: see <Link to="/source">SLAPorSLOP's open standard</Link>.
      </p>
    </div>
  );
}
