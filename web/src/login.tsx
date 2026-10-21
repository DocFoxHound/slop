// SPDX-License-Identifier: AGPL-3.0-or-later
// The desktop app's sign-in window: a small window shown before the store, like Steam's.
// Skipped at launch when a remembered sign-in still works (crates/slop-desktop, `startup`).
//
// Signing in happens in the system browser, never in this window (docs/SECURITY.md §4.1): the
// user approves this device on the Slop website, and the app picks up the session by itself.

import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";

import { WindowControls } from "./components/WindowControls";
import { desktop, openExternal } from "./platform";
import "./styles.css";

type State =
  | { step: "idle"; error?: string }
  | { step: "starting" }
  | { step: "waiting"; url: string; browserOpened: boolean; copied?: boolean };

function LoginWindow() {
  const [state, setState] = useState<State>({ step: "idle" });

  const start = async () => {
    setState({ step: "starting" });
    try {
      const started = await desktop.signInStart();
      setState({ step: "waiting", url: started.approve_url, browserOpened: started.browser_opened });
      // On success the app closes this window and opens the store.
      await desktop.signInFinish();
    } catch (err) {
      setState({ step: "idle", error: err instanceof Error ? err.message : String(err) });
    }
  };

  const copy = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setState((s) => (s.step === "waiting" ? { ...s, copied: true } : s));
    } catch {
      // Clipboard not allowed: the link is selectable in the box below.
    }
  };

  return (
    <main className="login-window">
      <div className="login-titlebar" data-tauri-drag-region>
        <WindowControls minimal />
      </div>
      <div className="login-brand" data-tauri-drag-region>
        <img src="/logo.svg" alt="" width={64} height={64} />
        <div className="login-logo" aria-label="Slop">
          SL<b>O</b>P
        </div>
        <p className="muted">Apps and games made with AI. Open source, every one.</p>
      </div>

      {state.step === "waiting" ? (
        <div className="login-body" aria-live="polite">
          <p>
            <b>Approve this device in your browser.</b>
          </p>
          <p className="muted small">
            {state.browserOpened
              ? "Sign in on the Slop website and choose Allow. This window closes by itself."
              : "Your browser didn't open. Open this link to sign in and approve this device:"}
          </p>
          <div className="login-link">
            <input readOnly value={state.url} aria-label="Sign-in link" onFocus={(e) => e.currentTarget.select()} />
            <button className="btn small" onClick={() => void copy(state.url)}>
              {state.copied ? "Copied" : "Copy"}
            </button>
          </div>
          <div className="login-actions">
            <button className="btn ghost" onClick={() => void openExternal(state.url)}>
              Open browser again
            </button>
            <button className="btn ghost" onClick={() => setState({ step: "idle" })}>
              Cancel
            </button>
          </div>
          <div className="login-spinner" aria-hidden="true" />
        </div>
      ) : (
        <div className="login-body">
          {state.step === "idle" && state.error && (
            <p className="form-error" role="alert">
              {state.error}
            </p>
          )}
          <button className="btn big slime login-primary" disabled={state.step === "starting"} onClick={() => void start()}>
            {state.step === "starting" ? "Opening your browser…" : "Sign in with your browser"}
          </button>
          <p className="muted small">
            New to Slop? You'll create your account in the browser. Slop never sees your password.
          </p>
        </div>
      )}

      <button className="login-skip" onClick={() => void desktop.continueSignedOut()}>
        Browse without signing in
      </button>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LoginWindow />
  </StrictMode>,
);
