// SPDX-License-Identifier: AGPL-3.0-or-later
// The slim bar under the top navigation: Back, Forward, Reload and the page's address, like a
// browser's. The desktop app's window has no browser around it, so this is how to go back
// there (and the browser's shortcuts work as they would in one); on the website it shows on wide
// screens only, quieter, and never on phones (styles.css, "page bar").

import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useNavigationType } from "react-router";

import { browsingKey, displayAddress, furthest, historyIndex, isTyping, siteOrigin } from "../browsing";
import { detectOs, isDesktop } from "../platform";

/** The site the desktop app was built for (vite.config.ts, from SLOP_API_URL). */
declare const __SLOP_SITE_URL__: string;

const FURTHEST_KEY = "slop.history-furthest";

function readFurthest(): number {
  try {
    const n = Number(sessionStorage.getItem(FURTHEST_KEY));
    return Number.isFinite(n) ? n : 0;
  } catch {
    return 0;
  }
}

function saveFurthest(n: number) {
  try {
    sessionStorage.setItem(FURTHEST_KEY, String(n));
  } catch {
    // Private windows: Forward is only known for this page view.
  }
}

/** Where this tab is in its history, and how far Forward goes. */
function useHistorySpot(): { index: number; furthest: number } {
  const location = useLocation();
  const action = useNavigationType();
  const [spot, setSpot] = useState(() => {
    const index = historyIndex();
    return { index, furthest: Math.max(readFurthest(), index) };
  });
  useEffect(() => {
    const index = historyIndex();
    const next = furthest(readFurthest(), index, action);
    saveFurthest(next);
    setSpot({ index, furthest: next });
  }, [location.key, action]);
  return spot;
}

export function BrowserBar({ onReload }: { onReload: () => void }) {
  const location = useLocation();
  const navigate = useNavigate();
  const { index, furthest: last } = useHistorySpot();
  const [spinning, setSpinning] = useState(false);
  const [copied, setCopied] = useState(false);
  const address = useRef<HTMLSpanElement>(null);
  const canBack = index > 0;
  const canForward = index < last;

  const reload = useCallback(() => {
    setSpinning(true);
    window.setTimeout(() => setSpinning(false), 600);
    onReload();
  }, [onReload]);

  // The desktop app's window has no browser shortcuts of its own (and in Windows' WebView, F5
  // would reload the whole app): the bar's buttons answer them. On the website the browser does.
  useEffect(() => {
    if (!isDesktop) return;
    const mac = detectOs() === "mac";
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const what = browsingKey(e, mac, isTyping(document.activeElement));
      if (!what) return;
      e.preventDefault();
      if (what === "back") {
        if (historyIndex() > 0) navigate(-1);
      } else if (what === "forward") navigate(1);
      else if (what === "reload") reload();
      else window.location.reload();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate, reload]);

  const origin = siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : "");
  const path = location.pathname + location.search + location.hash;
  const link = origin + path;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // No clipboard here (an insecure page, or permission refused): select it to copy by hand.
      selectAddress();
    }
  };
  const selectAddress = () => {
    const el = address.current;
    const sel = window.getSelection();
    if (!el || !sel) return;
    const range = document.createRange();
    range.selectNodeContents(el);
    sel.removeAllRanges();
    sel.addRange(range);
  };

  return (
    // In the desktop app the window is dragged by the bar's empty space too, not by its controls.
    <div className={`browserbar ${isDesktop ? "in-app" : ""}`} role="group" aria-label="Page" data-tauri-drag-region>
      <button type="button" className="bb-btn" aria-label="Back" title={isDesktop ? "Back (Alt+Left)" : "Back"} disabled={!canBack} onClick={() => navigate(-1)}>
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M10 3L5 8l5 5" />
        </svg>
      </button>
      <button type="button" className="bb-btn" aria-label="Forward" title={isDesktop ? "Forward (Alt+Right)" : "Forward"} disabled={!canForward} onClick={() => navigate(1)}>
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6 3l5 5-5 5" />
        </svg>
      </button>
      <button type="button" className={`bb-btn ${spinning ? "spinning" : ""}`} aria-label="Reload this page" title={isDesktop ? "Reload this page (F5)" : "Reload this page"} onClick={reload}>
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M13 8a5 5 0 1 1-1.6-3.7" />
          <path d="M13 2.5v3h-3" />
        </svg>
      </button>
      <div className="bb-address" data-tauri-drag-region>
        <span className="sr-only">Address: </span>
        <span className="bb-text" ref={address} title={link} onDoubleClick={selectAddress}>
          {displayAddress(origin, path)}
        </span>
        <button type="button" className="bb-btn bb-copy" aria-label="Copy link" title="Copy link" onClick={() => void copy()}>
          {copied ? (
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M3 8.5l3 3 7-7" />
            </svg>
          ) : (
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M6.5 9.5l3-3M7 4.5l1.2-1.2a2.5 2.5 0 0 1 3.5 3.5L10.5 8M9 11.5l-1.2 1.2a2.5 2.5 0 0 1-3.5-3.5L5.5 8" />
            </svg>
          )}
        </button>
        {/* A live region, not role="status": pages have their own status messages. */}
        <span className="sr-only bb-said" aria-live="polite">
          {copied ? "Link copied" : ""}
        </span>
      </div>
      <div className="grow" data-tauri-drag-region />
    </div>
  );
}
