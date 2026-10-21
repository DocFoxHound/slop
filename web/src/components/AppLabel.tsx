// SPDX-License-Identifier: AGPL-3.0-or-later
// "What this app does" (crates/slop-api/src/label.rs): what Slop's checks found and what the
// maker says, in plain words, plus accessibility features players confirm or dispute.

import { useState } from "react";

import { api, ApiError, type AppLabel as Label } from "../api";
import { useLoad } from "../hooks";

const TONE: Record<Label["items"][number]["tone"], string> = { good: "✓", info: "ℹ", note: "!" };

export function AppLabel({ slug }: { slug: string }) {
  const label = useLoad(() => api.label(slug), [slug]);
  const [error, setError] = useState<string>();
  if (!label.data) return null;
  const l = label.data;
  const report = async (feature: string, works: boolean | null) => {
    setError(undefined);
    try {
      await api.reportAccessibility(slug, feature, works);
      label.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't save that.");
    }
  };
  return (
    <section className="panel panel-pad app-label" id="what-it-does" aria-labelledby="label-title">
      <h2 id="label-title">What this app does</h2>
      <p className="faint small">
        {l.version ? `From version ${l.version}: ` : ""}
        <b>Checked</b> means Slop's own checks found it; <b>the maker says</b> means it's the maker's word.
      </p>
      <ul className="label-items">
        {l.items.map((i) => (
          <li key={i.key} className={`tone-${i.tone}`}>
            <span className="label-mark" aria-hidden="true">
              {TONE[i.tone]}
            </span>
            <div>
              <b>{i.title}</b> <span className="faint small">{i.checked ? "checked" : "the maker says"}</span>
              {i.detail && <div className="small">{i.detail}</div>}
            </div>
          </li>
        ))}
      </ul>
      {l.accessibility.length > 0 && (
        <>
          <h3>Accessibility</h3>
          <ul className="label-items">
            {l.accessibility.map((f) => (
              <li key={f.key}>
                <div>
                  <b>{f.name}</b>{" "}
                  <span className="faint small">
                    the maker says · {f.confirmed} player{f.confirmed === 1 ? "" : "s"} confirmed
                    {f.disputed > 0 && `, ${f.disputed} say it doesn't work`}
                  </span>
                  {l.can_report && (
                    <div className="row-actions" role="group" aria-label={`Does "${f.name}" work for you?`}>
                      <button type="button" className={`btn small ${f.mine === true ? "slime" : "ghost"}`} aria-pressed={f.mine === true} onClick={() => void report(f.key, f.mine === true ? null : true)}>
                        Works for me
                      </button>
                      <button type="button" className={`btn small ${f.mine === false ? "slime" : "ghost"}`} aria-pressed={f.mine === false} onClick={() => void report(f.key, f.mine === false ? null : false)}>
                        Doesn't work
                      </button>
                    </div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

/** The label's notes, for the install box: what to know before installing. */
export function LabelNotes({ slug }: { slug: string }) {
  const label = useLoad(() => api.label(slug), [slug]);
  const notes = label.data?.items.filter((i) => i.tone === "note") ?? [];
  if (!label.data) return null;
  return (
    <div className="small label-notes">
      {notes.length > 0 && (
        <>
          <b>Before you install:</b>
          <ul>
            {notes.map((n) => (
              <li key={n.key}>{n.title}</li>
            ))}
          </ul>
        </>
      )}
      <a href="#what-it-does">What this app does</a>
    </div>
  );
}
