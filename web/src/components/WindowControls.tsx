// SPDX-License-Identifier: AGPL-3.0-or-later
// The desktop app is borderless (no system title bar), like Steam: SLAPorSLOP's own top bar is where the
// window is dragged, and these are its minimize / maximize / close buttons.

import { useEffect, useState } from "react";

import { isDesktop } from "../platform";

async function currentWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export function WindowControls({ minimal = false }: { minimal?: boolean }) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isDesktop || minimal) return;
    let unlisten: (() => void) | undefined;
    void currentWindow().then(async (w) => {
      setMaximized(await w.isMaximized());
      unlisten = await w.onResized(async () => setMaximized(await w.isMaximized()));
    });
    return () => unlisten?.();
  }, [minimal]);

  if (!isDesktop) return null;
  const act = (fn: (w: Awaited<ReturnType<typeof currentWindow>>) => Promise<void>) => () => void currentWindow().then(fn);

  return (
    <div className="window-controls">
      {!minimal && (
        <>
          <button type="button" aria-label="Minimize" title="Minimize" onClick={act((w) => w.minimize())}>
            <svg viewBox="0 0 10 10" aria-hidden="true">
              <path d="M1 5.5h8" />
            </svg>
          </button>
          <button
            type="button"
            aria-label={maximized ? "Restore" : "Maximize"}
            title={maximized ? "Restore" : "Maximize"}
            onClick={act((w) => w.toggleMaximize())}
          >
            <svg viewBox="0 0 10 10" aria-hidden="true">
              {maximized ? <path d="M2.5 3.5h5v5h-5zM3.5 3.5V1.5h5v5h-1" /> : <path d="M1.5 1.5h7v7h-7z" />}
            </svg>
          </button>
        </>
      )}
      <button type="button" className="close" aria-label="Close" title="Close" onClick={act((w) => w.close())}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" />
        </svg>
      </button>
    </div>
  );
}
