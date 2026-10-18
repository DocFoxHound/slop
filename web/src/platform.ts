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
