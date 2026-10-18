// Where the UI is running: the website, or the Tauri desktop app.

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export const isDesktop: boolean = typeof window !== "undefined" && window.__TAURI_INTERNALS__ !== undefined;

export type Os = "windows" | "mac" | "linux" | "android" | "ios";

export function detectOs(userAgent: string = navigator.userAgent): Os {
  const ua = userAgent.toLowerCase();
  if (/android/.test(ua)) return "android";
  if (/iphone|ipad|ipod/.test(ua)) return "ios";
  if (/mac os|macintosh/.test(ua)) return "mac";
  if (/windows/.test(ua)) return "windows";
  return "linux";
}

export const OS_LABEL: Record<Os, string> = {
  windows: "Windows",
  mac: "macOS",
  linux: "Linux",
  android: "Android",
  ios: "iOS",
};

/**
 * Opens a link outside Slop. In the desktop app links must go to the system browser; following
 * them inside the app's window would navigate away from Slop.
 */
export async function openExternal(url: string): Promise<void> {
  if (isDesktop) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

// ---------------------------------------------------------------------------------------------
// Desktop commands (crates/slop-desktop). Only call these when `isDesktop` is true.

export interface InstalledApp {
  slug: string;
  title: string;
  maker: string;
  version: string;
  release_id: string;
  kind: "archive" | "installer" | "apk";
  installed_at: string;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
}

export interface DesktopError {
  code: string;
  message: string;
}

/** Rust error messages start lowercase; show them as sentences. */
export function sentence(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1);
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    const e = err as Partial<DesktopError>;
    throw Object.assign(new Error(sentence(e.message ?? String(err))), { code: e.code ?? "unknown" });
  }
}

export const desktop = {
  sessionStatus: () => call<{ signed_in: boolean }>("session_status"),
  /** Asks Slop for an approval page and tries to open it in the system browser. */
  signInStart: () => call<{ approve_url: string; browser_opened: boolean }>("sign_in_start"),
  /** Resolves once the user has approved this device; the app then opens the store. */
  signInFinish: () => call<void>("sign_in_finish"),
  /** Signs out; the app closes the store and shows the sign-in window. */
  signOut: () => call<void>("sign_out"),
  showLogin: () => call<void>("show_login"),
  continueSignedOut: () => call<void>("continue_signed_out"),
  /** A newer version of the app, if one has been found. */
  updateStatus: () => call<UpdateInfo | null>("update_status"),
  /** Downloads, verifies and installs the update, then restarts. Only when the player asks. */
  installUpdate: () => call<void>("install_update"),
  onUpdateAvailable: async (handler: (u: UpdateInfo) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<UpdateInfo>("update-available", (e) => handler(e.payload));
  },
  /** Fires when this device signs in or out (from any window). */
  onSessionChanged: async (handler: () => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("session-changed", () => handler());
  },
  platformInfo: () => call<{ platform: Os | null; can_install: boolean }>("platform_info"),
  installed: () => call<InstalledApp[]>("installed_apps"),
  install: (slug: string) => call<InstalledApp>("install_app", { slug }),
  uninstall: (slug: string) => call<void>("uninstall_app", { slug }),
  launch: (slug: string) => call<void>("launch_app", { slug }),
  runInstaller: (slug: string) => call<void>("run_installer", { slug }),
  /** Install progress events: bytes downloaded so far. Returns an unsubscribe function. */
  onProgress: async (handler: (p: { slug: string; got: number; total: number }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ slug: string; got: number; total: number }>("install-progress", (e) => handler(e.payload));
  },
};
