// SPDX-License-Identifier: AGPL-3.0-or-later
// What the page bar (components/BrowserBar.tsx) needs to know: whether Back and Forward lead
// anywhere inside SLAPorSLOP, the page's address, and the browser's own keyboard shortcuts, which the
// desktop app's window doesn't have.

/** React Router numbers this tab's history entries from 0 (`window.history.state.idx`). */
export function historyIndex(): number {
  const idx = (window.history.state as { idx?: unknown } | null)?.idx;
  return typeof idx === "number" ? idx : 0;
}

/**
 * The furthest entry Forward can reach, after a navigation to entry `idx`: a new page drops
 * whatever was ahead (as browsers do); Back, Forward and replacing the page keep it.
 */
export function furthest(previous: number, idx: number, action: "PUSH" | "POP" | "REPLACE"): number {
  return action === "PUSH" ? idx : Math.max(previous, idx);
}

/**
 * Where the website lives. In the packaged desktop app the page itself comes from the app
 * (tauri://localhost or http://tauri.localhost), so its address is the site the app was built
 * for (SLOP_API_URL, as in crates/slop-desktop), so a copied link works in a browser.
 */
export function siteOrigin(location: Pick<Location, "origin" | "protocol" | "hostname">, builtFor: string): string {
  if (location.protocol === "tauri:" || location.hostname === "tauri.localhost") {
    try {
      return new URL(builtFor || "https://slaporslop.games").origin;
    } catch {
      return "https://slaporslop.games";
    }
  }
  return location.origin;
}

/**
 * Whether `origin` is a test site (the test server, or a developer's own machine): the same rule
 * as crates/slop-desktop/build.rs and the desktop build scripts in deploy/, which build its "Slop Dev" app.
 */
export function isTestSite(origin: string): boolean {
  try {
    const host = new URL(origin).hostname;
    return host === "localhost" || host === "127.0.0.1" || host.endsWith(".sslip.io");
  } catch {
    return false;
  }
}

declare const __SLOP_SITE_URL__: string;

/** Whether this page belongs to a test site, in a browser or in the desktop app built for one. */
export function onTestSite(): boolean {
  return isTestSite(siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : ""));
}

/** The name in window and tab titles: "SLAPorSLOP Dev" on a test site, so nobody takes it for the real one. */
export function siteName(): string {
  return onTestSite() ? "SLAPorSLOP Dev" : "SLAPorSLOP";
}

/** "slaporslop.games/app/lantern-keep": the address as people read it, without https://. */
export function displayAddress(origin: string, path: string): string {
  let shown = path;
  try {
    shown = decodeURI(path);
  } catch {
    // Not valid escapes: shown as it is.
  }
  return origin.replace(/^https?:\/\//, "") + shown;
}

export type BrowsingKey = "back" | "forward" | "reload" | "hard-reload";

/**
 * The browser shortcut a key press is, if any: Alt+Left/Right (Cmd+[ and Cmd+] on a Mac), F5
 * or Ctrl+R (Cmd+R), and with Shift (or Ctrl+F5) a full reload. Not while typing: moving in a
 * text with Alt+arrows mustn't throw away what was typed.
 */
export function browsingKey(
  e: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">,
  mac: boolean,
  typing: boolean,
): BrowsingKey | null {
  const mod = mac ? e.metaKey : e.ctrlKey;
  if (e.key === "F5") return e.shiftKey || e.ctrlKey ? "hard-reload" : "reload";
  if (mod && !e.altKey && e.key.toLowerCase() === "r") return e.shiftKey ? "hard-reload" : "reload";
  if (typing) return null;
  if (mac) {
    if (e.metaKey && !e.altKey && !e.shiftKey && !e.ctrlKey && e.key === "[") return "back";
    if (e.metaKey && !e.altKey && !e.shiftKey && !e.ctrlKey && e.key === "]") return "forward";
    return null;
  }
  if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.key === "ArrowLeft") return "back";
  if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.key === "ArrowRight") return "forward";
  return null;
}

/** Whether an element takes typing (a text field or an editable area). */
export function isTyping(el: Element | null): boolean {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return true;
  if (el instanceof HTMLInputElement) return !["button", "checkbox", "radio", "submit", "reset", "range", "color", "file"].includes(el.type);
  return (el as HTMLElement).isContentEditable === true;
}

/**
 * The element a link's fragment points at (`#decisions`), or "" when it doesn't name one: a
 * handoff link's `#fill=…` (handoff.ts), or a fragment with a broken %-escape, which used to stop
 * the whole page.
 */
export function fragmentId(hash: string): string {
  if (hash.startsWith("#fill=")) return "";
  try {
    return decodeURIComponent(hash.slice(1));
  } catch {
    return "";
  }
}
