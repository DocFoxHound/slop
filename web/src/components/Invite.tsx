// SPDX-License-Identifier: AGPL-3.0-or-later
// The beta's invite code for going public (crates/slop-api/src/invites.rs). While SLAPorSLOP staff
// have it switched on, publishing a release, putting an app in the store and asking for a private
// app to go public answer "invite_required" until the account has entered it. The page then asks
// for the code, and the action runs again once it's entered; nothing they did is lost.

import { useCallback, useEffect, useId, useRef, useState, type FormEvent, type ReactNode, type RefObject } from "react";

import { api, ApiError } from "../api";
import { useLoad } from "../hooks";
import { ExternalLink } from "./ui";

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

/** Why publishing waits, in the same words on the banner and in the dialog. */
function ClosedBeta({ discordUrl, id }: { discordUrl: string; id?: string }) {
  const discord = discordUrl ? <ExternalLink href={discordUrl}>SLAPorSLOP Discord</ExternalLink> : "SLAPorSLOP Discord";
  return (
    <div id={id}>
      <p>
        SLAPorSLOP is a closed-beta test at the moment, so publishing is invite-only while a small group of makers tries
        it out. You can make store pages and send releases through the checks now. Publishing a release, putting an app
        in the store or making a private app public needs an invite code.
      </p>
      <p>
        To get a code, ask on the {discord}. You enter it once: your account can publish from then on, even after the code
        changes.
      </p>
    </div>
  );
}

/**
 * The code field and its button. `onEntered` runs once the code is right; a wrong one is said
 * out loud next to the field, which keeps focus.
 */
function CodeForm({
  onEntered,
  describedBy,
  submitLabel,
  field,
  children,
}: {
  onEntered: () => void;
  describedBy?: string;
  submitLabel: string;
  field?: RefObject<HTMLInputElement | null>;
  children?: ReactNode;
}) {
  const own = useRef<HTMLInputElement>(null);
  const input = field ?? own;
  const [code, setCode] = useState("");
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const errorId = useId();

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
      input.current?.focus();
    }
  };
  const onField = error?.field === "code";
  const described = [describedBy, onField ? errorId : undefined].filter(Boolean).join(" ") || undefined;

  return (
    <form className="form invite-form" onSubmit={(e) => void submit(e)}>
      <label>
        Invite code
        <input
          ref={input}
          type="text"
          autoComplete="off"
          spellCheck={false}
          maxLength={200}
          required
          value={code}
          onChange={(e) => setCode(e.target.value)}
          aria-describedby={described}
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
          {busy ? "Checking…" : submitLabel}
        </button>
        {children}
      </div>
    </form>
  );
}

/** The invite code, asked for in a dialog when going public is refused for want of it. */
export function EnterInvite({ onEntered, onCancel }: { onEntered: () => void; onCancel: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const status = useLoad(() => api.publishInvite(), []);
  const titleId = useId();
  const hintId = useId();
  useEffect(() => {
    // Focus goes back where it was (the button that led here) once the dialog closes.
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    field.current?.focus();
    return () => before?.focus();
  }, []);

  return (
    <dialog ref={dialog} className="dialog" aria-labelledby={titleId} aria-describedby={hintId} onCancel={onCancel}>
      <h2 id={titleId}>Publishing is invite-only for now</h2>
      <div className="small">
        <ClosedBeta id={hintId} discordUrl={status.data?.discord_url ?? ""} />
      </div>
      <CodeForm field={field} describedBy={hintId} submitLabel="Enter and continue" onEntered={onEntered}>
        <button type="button" className="btn ghost" onClick={onCancel}>
          Not now
        </button>
      </CodeForm>
    </dialog>
  );
}

/**
 * The closed-beta banner on the maker's pages (Your apps, a new store page, and each app's maker
 * tabs) while the code is needed and this account hasn't entered it: why publishing waits, where to
 * ask for a code, and the field to enter it right there. Nothing otherwise.
 */
export function InviteNotice() {
  const status = useLoad(() => api.publishInvite(), []);
  const titleId = useId();
  const textId = useId();
  const { reload } = status;
  useEffect(() => {
    window.addEventListener(ENTERED, reload);
    return () => window.removeEventListener(ENTERED, reload);
  }, [reload]);
  if (!status.data?.required || status.data.invited) return null;
  return (
    <section className="notice invite-notice" aria-labelledby={titleId}>
      <h2 id={titleId}>Closed beta: publishing is invite-only for now</h2>
      <div className="small">
        <ClosedBeta id={textId} discordUrl={status.data.discord_url} />
      </div>
      <CodeForm describedBy={textId} submitLabel="Enter code" onEntered={reload} />
    </section>
  );
}
