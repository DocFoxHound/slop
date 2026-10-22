// SPDX-License-Identifier: AGPL-3.0-or-later
// Confirming it's you for a sensitive action (SECURITY §4.3): with a passkey when you have one,
// otherwise by signing in again.

import { useState } from "react";

import { api, ApiError } from "../api";
import { passkeysSupported, signWithPasskey } from "../passkeys";

/** Explains a step_up_required refusal and offers the way through. `onDone` retries. */
export function StepUpPrompt({ signInAgain, onDone }: { signInAgain: string; onDone?: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [done, setDone] = useState(false);
  const confirm = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const started = await api.passkeys.stepUpStart();
      const credential = await signWithPasskey(started.options);
      await api.passkeys.stepUpFinish(started.challenge_id, credential);
      setDone(true);
      onDone?.();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "The passkey wasn't used. Try again, or sign in again instead.");
    } finally {
      setBusy(false);
    }
  };
  if (done) return <span role="status">Confirmed. Try once more.</span>;
  return (
    <span>
      For your safety, this needs a sign-in from the last few minutes.{" "}
      {passkeysSupported() && (
        <>
          <button type="button" className="link-btn" disabled={busy} onClick={() => void confirm()}>
            {busy ? "Waiting for your passkey…" : "Confirm with a passkey"}
          </button>{" "}
          or{" "}
        </>
      )}
      <a href={signInAgain}>sign in again</a> (with a passkey, if your account has one), then try once more.
      {error && (
        <span className="form-error" role="alert">
          {" "}
          {error}
        </span>
      )}
    </span>
  );
}
