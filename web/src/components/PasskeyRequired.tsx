// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing and payout changes need a passkey (crates/slop-api/src/passkeys.rs `require`): when
// the API answers "passkey_required", this asks the person to confirm with theirs, or to add one
// first, then the action runs again (useTermsGate in Terms.tsx). Nothing they typed is lost.

import { useEffect, useRef, useState } from "react";

import { api, ApiError } from "../api";
import { passkeysSupported, signWithPasskey } from "../passkeys";

export function PasskeyRequired({ add, onDone, onCancel }: { add: boolean; onDone: () => void; onCancel: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => dialog.current?.showModal(), []);
  const confirm = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const started = await api.passkeys.stepUpStart();
      const credential = await signWithPasskey(started.options);
      await api.passkeys.stepUpFinish(started.challenge_id, credential);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "The passkey wasn't used. Try again.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <dialog ref={dialog} className="dialog" aria-label="Passkey needed" onCancel={onCancel}>
      <h2>{add ? "Add a passkey first" : "Confirm with your passkey"}</h2>
      <p>
        Publishing on Slop and changing where money goes need a passkey, so someone who gets into your sign-in elsewhere
        can't do either.
        {add
          ? " Add one under Account → Sign-ins (it opens in a new tab), then come back and try again."
          : " Use it once, and it counts for the next 12 hours."}
      </p>
      {!add && !passkeysSupported() && (
        <p className="muted small">This browser can't use passkeys. Try another browser or device.</p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="actions">
        <button type="button" className="btn ghost" onClick={onCancel}>
          Cancel
        </button>
        {add ? (
          <>
            <a className="btn" href="/account/security#passkeys" target="_blank" rel="noopener">
              Add a passkey
            </a>
            <button type="button" className="btn slime" onClick={onDone}>
              I've added one: try again
            </button>
          </>
        ) : (
          <button type="button" className="btn slime" disabled={busy || !passkeysSupported()} onClick={() => void confirm()}>
            {busy ? "Waiting for your passkey…" : "Confirm with your passkey"}
          </button>
        )}
      </div>
    </dialog>
  );
}
