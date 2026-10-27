// SPDX-License-Identifier: AGPL-3.0-or-later
// What the forms show for a handoff link from someone's AI assistant (handoff.ts): what it filled
// in and what it left out, that it couldn't be read, or, signed out, that it waits for sign-in.

import { useEffect, useState, type ReactNode } from "react";

import { takeHandoff, type Fill, type HandoffForm, type HandoffLink } from "../handoff";

/**
 * The handoff link for this form, read once per visit to it (none while `enabled` is false), and
 * which one it is: a new link opened in the same tab on the same page only changes the fragment,
 * so it's read as it comes, and the form starts again from it.
 */
export function useHandoff(form: HandoffForm, enabled = true): { link: HandoffLink | null; n: number } {
  const [state, setState] = useState(() => ({ link: enabled ? takeHandoff(form) : null, n: 0 }));
  useEffect(() => {
    if (!enabled) return;
    const opened = () => {
      if (!window.location.hash.startsWith("#fill=")) return;
      const link = takeHandoff(form);
      setState((s) => ({ link, n: s.n + 1 }));
    };
    window.addEventListener("hashchange", opened);
    return () => window.removeEventListener("hashchange", opened);
  }, [form, enabled]);
  return enabled ? state : { link: null, n: 0 };
}

/** "Name, Store address and Tags". */
function names(fields: string[], labels: Record<string, string>): string {
  const words = fields.map((f) => labels[f] ?? f);
  return words.length < 2 ? (words[0] ?? "") : `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

/** At the top of a form an AI assistant filled in: check it all, then press the button yourself. */
export function HandoffNotice<T>({
  fill,
  labels,
  check,
  button,
  onClear,
  clearLabel,
  children,
}: {
  fill: Fill<T>;
  labels: Record<string, string>;
  /** What most needs a person's eye. */
  check: ReactNode;
  /** The form's own button. */
  button: string;
  onClear: () => void;
  clearLabel: string;
  children?: ReactNode;
}) {
  return (
    <section className="notice handoff-notice" role="status" aria-label="Filled in by your AI assistant">
      <p>
        <b>Filled in by your AI assistant.</b> Check everything, especially {check}, then press <b>{button}</b>. Nothing is sent
        until you do.
      </p>
      {fill.filled.length > 0 ? (
        <p className="handoff-filled">Filled in: {names(fill.filled, labels)}.</p>
      ) : (
        <p>It didn't fill in anything this form has.</p>
      )}
      {fill.left.length > 0 && (
        <div className="handoff-left">
          Left out, for you to fill in:
          <ul>
            {fill.left.map((l) => (
              <li key={l.field}>
                {labels[l.field] ?? l.field}: {l.why}
              </li>
            ))}
          </ul>
        </div>
      )}
      {fill.unknown > 0 && (
        <p className="muted small">
          {fill.unknown === 1 ? "One other thing in the link isn't" : `${fill.unknown} other things in the link aren't`} part of
          this form, so {fill.unknown === 1 ? "it was" : "they were"} left out.
        </p>
      )}
      {children}
      <button type="button" className="btn small ghost" onClick={onClear}>
        {clearLabel}
      </button>
    </section>
  );
}

/** A handoff link that couldn't be read: the form is as it would be without it. */
export function HandoffError({ error, then }: { error: string; then: string }) {
  return (
    <p className="notice handoff-error" role="alert">
      The link from your AI assistant couldn't be read: {error}. {then}
    </p>
  );
}

/** Signed out: the fill waits in this tab while they sign in or make an account. */
export function HandoffWaiting({ link }: { link: HandoffLink | null }) {
  if (!link) return null;
  if ("error" in link) return <HandoffError error={link.error} then="Ask your assistant for a new link, or fill in the form yourself after signing in." />;
  return (
    <p className="notice handoff-waiting" role="status">
      <b>Your AI assistant's draft is ready.</b> Sign in, or make an account, and the form opens with it filled in for you to check.
      It's kept in this browser tab only, for two hours.
    </p>
  );
}
