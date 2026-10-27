// SPDX-License-Identifier: AGPL-3.0-or-later
// "A new version of SLAPorSLOP is available": a dismissible notice in the desktop app. Updates are
// never forced; the player chooses when to install (crates/slop-desktop, watch_for_updates).

import { useEffect, useState } from "react";

import { desktop, isDesktop, type UpdateInfo } from "../platform";

/** "Later" hides the notice for this version until the app restarts. */
const dismissed = new Set<string>();

export function UpdateNotice() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [state, setState] = useState<"idle" | "installing" | "failed">("idle");
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    void desktop.updateStatus().then((u) => u && setUpdate(u), () => {});
    void desktop.onUpdateAvailable(setUpdate).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  if (!update || dismissed.has(update.version)) return null;

  const install = async () => {
    setState("installing");
    try {
      await desktop.installUpdate(); // restarts the app on success
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setState("failed");
    }
  };

  return (
    <div className="update-notice" role="status">
      <span>
        <b>SLAPorSLOP {update.version} is available.</b>{" "}
        <span className="muted">
          You have {update.current_version}.
          {state === "installing" && " Downloading and checking the update…"}
          {state === "failed" && ` ${error}`}
        </span>
      </span>
      <span className="row-actions">
        <button className="btn small slime" disabled={state === "installing"} onClick={() => void install()}>
          {state === "failed" ? "Try again" : "Update and restart"}
        </button>
        <button
          className="btn small ghost"
          disabled={state === "installing"}
          onClick={() => {
            dismissed.add(update.version);
            setUpdate(null);
          }}
        >
          Later
        </button>
      </span>
    </div>
  );
}
