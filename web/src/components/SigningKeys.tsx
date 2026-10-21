// SPDX-License-Identifier: AGPL-3.0-or-later
// Signing keys for builds a maker makes and attaches by hand (crates/slop-api/src/maker_keys.rs):
// an Ed25519 SSH key, a `.sig` beside each file, and 72 hours before a new key counts.

import { useState } from "react";

import { api, ApiError } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { StepUpPrompt } from "./PasskeyStepUp";

export function SigningKeys() {
  const keys = useLoad(() => api.signingKeys.list(), []);
  const [name, setName] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError | string>();
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      keys.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section id="signing-keys">
      <h2 className="section-title">Signing keys</h2>
      <div className="panel panel-pad">
        <p className="muted small">
          For builds you make yourself instead of in GitHub Actions (on GitLab, Bitbucket, your own server or your computer). Sign
          each file with your SSH key and attach the <code>.sig</code> next to it on the release page:
        </p>
        <pre className="code-block">ssh-keygen -Y sign -f ~/.ssh/id_ed25519 -n slop-release my-game-1.0.0-windows.zip</pre>
        <p className="muted small">
          Only Ed25519 keys. A new key counts 72 hours after you add it, and you're told when one is added, so nobody can
          add one quietly and sign a release in your name.
        </p>
        {keys.data && keys.data.length > 0 && (
          <ul className="report-list">
            {keys.data.map((k) => (
              <li key={k.id}>
                <span>
                  <b>{k.name}</b> <code className="small">{k.fingerprint}</code>{" "}
                  <span className="muted small">
                    {k.active ? `works since ${formatDate(k.active_from)}` : `works from ${formatDate(k.active_from)}`}
                  </span>
                </span>
                <button className="btn small ghost" disabled={busy} onClick={() => void run(() => api.signingKeys.remove(k.id))}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              await api.signingKeys.add(name.trim() || "Signing key", publicKey);
              setName("");
              setPublicKey("");
            });
          }}
        >
          <label>
            Name
            <input value={name} maxLength={60} placeholder="e.g. Laptop" onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Public key (the line in <code>id_ed25519.pub</code>)
            <input value={publicKey} placeholder="ssh-ed25519 AAAA…" spellCheck={false} onChange={(e) => setPublicKey(e.target.value)} />
          </label>
          <div className="row-actions">
            <button className="btn small" disabled={busy || !publicKey.trim()}>
              Add signing key
            </button>
          </div>
        </form>
        {error && (
          <p className="form-error" role="alert">
            {typeof error === "string" ? (
              error
            ) : error.code === "step_up_required" ? (
              <StepUpPrompt signInAgain={`/signin?return_to=${encodeURIComponent("/account#signing-keys")}`} onDone={() => setError(undefined)} />
            ) : (
              error.message
            )}
          </p>
        )}
      </div>
    </section>
  );
}
