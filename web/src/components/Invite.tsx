// SPDX-License-Identifier: AGPL-3.0-or-later
// The beta's invite code for going public (crates/slop-api/src/invites.rs). While SLAPorSLOP staff
// have it switched on, publishing a release, putting an app in the store and asking for a private
// app to go public answer "invite_required" until the account has entered it. The page then asks
// for the code, and the action runs again once it's entered; nothing they did is lost.

import { useCallback, useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { api, ApiError } from "../api";
import { useLoad } from "../hooks";

/** Sent on `window` once the code is entered, so notices on the page go away. */
const ENTERED = "slop:invite-entered";

interface Pending {
  resolve: () => void;
  reject: () => void;
}

/**
 * `guard(fn)` runs `fn`; if going public needs the invite code first, `prompt` asks for it, and
 * `fn` runs again once it's entered. Cancelling rejects with the original refusal, which says why.
 */
export function useInviteGate(): { guard: <T>(fn: () => Promise<T>) => Promise<T>; prompt: ReactNode } {
  const [pending, setPending] = useState<Pending>();
  const guard = useCallback(async <T,>(fn: () => Promise<T>): Promise<T> => {
    for (;;) {
      try {
        return await fn();
      } catch (err) {
        if (!(err instanceof ApiError) || err.code !== "invite_required") throw err;
        await new Promise<void>((resolve, reject) => setPending({ resolve, reject: () => reject(err) }));
      }
    }
  }, []);
  const prompt = pending ? (
    <EnterInvite
      onEntered={() => {
        setPending(undefined);
        pending.resolve();
      }}
      onCancel={() => {
        setPending(undefined);
        pending.reject();
      }}
    />
  ) : null;
  return { guard, prompt };
}

/** The invite code, asked for in a dialog: one labelled field, its error said out loud. */
export function EnterInvite({ onEntered, onCancel }: { onEntered: () => void; onCancel: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const [code, setCode] = useState("");
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const hintId = useId();
  const errorId = useId();
  useEffect(() => {
    // Focus goes back where it was (the button that led here) once the dialog closes.
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    field.current?.focus();
    return () => before?.focus();
  }, []);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.enterPublishInvite(code);
      window.dispatchEvent(new Event(ENTERED));
      onEntered();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
      setBusy(false);
      field.current?.focus();
    }
  };
  const onField = error?.field === "code";

  return (
    <dialog ref={dialog} className="dialog" aria-labelledby={titleId} onCancel={onCancel}>
      <form className="form" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId}>Enter your invite code</h2>
        <p id={hintId} className="small">
          SLAPorSLOP is in beta: going public needs an invite code. You enter it once; your account keeps it even when the
          code changes.
        </p>
        <label>
          Invite code
          <input
            ref={field}
            type="text"
            autoComplete="off"
            spellCheck={false}
            maxLength={200}
            required
            value={code}
            onChange={(e) => setCode(e.target.value)}
            aria-describedby={`${hintId}${onField ? ` ${errorId}` : ""}`}
            aria-invalid={onField || undefined}
          />
        </label>
        {error && (
          <p id={errorId} className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <div className="row-actions">
          <button type="submit" className="btn slime" disabled={busy || !code.trim()}>
            {busy ? "Checking…" : "Enter and continue"}
          </button>
          <button type="button" className="btn ghost" onClick={onCancel}>
            Not now
          </button>
        </div>
      </form>
    </dialog>
  );
}

/**
 * Where makers publish: a short note while the code is needed and not yet entered, so it isn't a
 * surprise at Publish, with a button to enter it now. Nothing otherwise.
 */
export function InviteNotice() {
  const status = useLoad(() => api.publishInvite(), []);
  const [asking, setAsking] = useState(false);
  const { reload } = status;
  useEffect(() => {
    window.addEventListener(ENTERED, reload);
    return () => window.removeEventListener(ENTERED, reload);
  }, [reload]);
  if (!status.data?.required || status.data.invited) return null;
  return (
    <div className="notice small invite-notice">
      <p>
        <b>SLAPorSLOP is in beta:</b> going public needs an invite code. You can make store pages and send releases through
        the checks now; enter the code before you publish or put an app in the store.
      </p>
      <button type="button" className="btn small" aria-haspopup="dialog" onClick={() => setAsking(true)}>
        Enter invite code
      </button>
      {asking && <EnterInvite onEntered={() => setAsking(false)} onCancel={() => setAsking(false)} />}
    </div>
  );
}
