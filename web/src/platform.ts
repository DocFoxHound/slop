// SPDX-License-Identifier: AGPL-3.0-or-later
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
/** Saves text as a file the user downloads (job records, conversation copies, data exports). */
export function saveFile(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

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
  /** A community build: the original app's address, from its signed manifest (DESIGN §9). */
  build_of?: string | null;
}

/** A mod added to an installed game (DESIGN §9). */
export interface InstalledMod {
  slug: string;
  title: string;
  maker: string;
  version: string;
  release_id: string;
  game_slug: string;
  game_title: string;
  installed_at: string;
  /** Why Slop pulled it; a pulled mod isn't handed to its game. */
  revoked: string | null;
}

/** Sandbox mode (DESIGN §6): whether it works here, and the player's choices. */
export interface SandboxStatus {
  available: boolean;
  reason: string | null;
  /** What sandbox mode can't protect against on this desktop (X11), to show the player. */
  limits?: string | null;
  settings: { all: boolean; apps: Record<string, { on: boolean; block_network: boolean }> };
}

/** A copy of an app's save folder, taken before a sandboxed session. */
export interface SaveCopy {
  id: string;
  taken_at: string;
  bytes: number;
}

export interface Launched {
  sandboxed: boolean;
  /** Programs found in the save folder and moved out before the start. */
  quarantined: string[];
  copy_skipped: boolean;
  /** Things in the game's own mods folder that aren't mods from Slop, moved out (DESIGN §9). */
  unapproved_mods: string[];
  /** What save sync did before the start, when it's on. */
  save_sync?: SaveSyncOutcome | null;
}

/** crates/slop-client/src/save_sync.rs Outcome. */
export type SaveSyncOutcome =
  | { outcome: "up_to_date" }
  | { outcome: "fetched"; generation: number }
  | { outcome: "sent"; generation: number }
  | { outcome: "both_changed"; other_device: string }
  | { outcome: "too_big" };

/** A running game asking for mods, for the player to confirm (Slop Link, DESIGN §9). */
export interface ModRequest {
  id: string;
  game: string;
  /** Who wants them, in the game's words: untrusted text. */
  from: string | null;
  mods: Array<{ slug: string; version: string | null }>;
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

// Chat (crates/slop-client/src/chat): end-to-end encrypted in the app's Rust core. The page sees
// only what it shows; keys and the encrypted state never reach it (docs/SECURITY.md §8.5).

export interface ChatStatus {
  state: "none" | "pending" | "active" | "revoked";
  device_id: string | null;
  code: string | null;
  pending: Array<{ id: string; name: string; code: string }>;
  devices: Array<{ id: string; name: string; status: string; last_seen_at: string; this_device: boolean }>;
  backups: boolean;
}

export interface ChatPerson {
  id: string;
  name: string;
  /** Their profile picture, served by Slop; null: the letter. */
  avatar_url?: string | null;
}

export interface ChatConversation {
  id: string;
  kind: "direct" | "group" | "team";
  title: string;
  members: ChatPerson[];
  team_slug: string | null;
  unread: number;
  muted: boolean;
  ready: boolean;
  closed: boolean;
  last: string | null;
  last_at: string | null;
  owner: boolean;
}

export interface ChatAttachment {
  id: string;
  mime: string;
  name: string;
  size: number;
  source_url: string | null;
}

export interface ChatMessage {
  id: string;
  seq: number;
  sender: ChatPerson;
  mine: boolean;
  at: string;
  text: string;
  reply_to: { id: string; sender: string; text: string } | null;
  attachments: ChatAttachment[];
  preview: { url: string; site: string; title: string | null; description: string | null; image: ChatAttachment | null } | null;
  reactions: Array<{ emoji: string; count: number; mine: boolean; names: string[] }>;
  edited: boolean;
  deleted: boolean;
  notice: boolean;
  warnings: string[];
  reportable: boolean;
}

export interface ChatChanges {
  conversations: string[];
  list: boolean;
  devices: boolean;
  /** Who is typing where right now; shown for a few seconds. */
  typing: Array<{ conversation: string; user: string; name: string }>;
  new_messages: Array<{ conversation: string; title: string; sender: string; text: string }>;
}

export type ChatAction =
  | { action: "status" }
  | { action: "setup"; name: string }
  | { action: "approve"; device: string; code: string }
  | { action: "start_over" }
  | { action: "remove_device"; device: string }
  | { action: "conversations" }
  | { action: "messages"; conversation: string; before?: number | null }
  | {
      action: "send";
      conversation: string;
      text: string;
      reply_to?: string | null;
      files?: Array<{ name: string; mime: string; data: string }>;
      link_preview: boolean;
    }
  | { action: "react"; conversation: string; message: string; emoji: string; on: boolean }
  | { action: "edit"; conversation: string; message: string; text: string }
  | { action: "delete"; conversation: string; message: string }
  | { action: "rename"; conversation: string; name: string }
  | { action: "start_direct"; user: string }
  | { action: "start_group"; users: string[]; name?: string | null }
  | { action: "start_team"; team: string }
  | { action: "add_people"; conversation: string; users: string[] }
  | { action: "leave"; conversation: string }
  | { action: "remove_person"; conversation: string; user: string }
  | { action: "hide"; conversation: string }
  | { action: "mute"; conversation: string; muted: boolean }
  | { action: "mark_read"; conversation: string }
  | { action: "attachment"; conversation: string; message: string; attachment: string }
  | { action: "security_codes"; conversation: string }
  | { action: "report"; conversation: string; message: string; reason: string; details: string }
  | { action: "set_backups"; on: boolean }
  | { action: "restore"; code: string }
  | { action: "typing"; conversation: string; typing: boolean }
  | { action: "notifications"; on: boolean; show_text: boolean }
  | { action: "notification_settings" };

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
  /** "Update apps automatically": read, or switch on or off. On by default. */
  autoUpdateSetting: (on?: boolean) => call<boolean>("auto_update_setting", { on: on ?? null }),
  /** Apps Slop started that are still running; they're updated once they close. */
  running: () => call<string[]>("running_apps"),
  install: (slug: string) => call<InstalledApp>("install_app", { slug }),
  uninstall: (slug: string, deleteSaves = false) => call<void>("uninstall_app", { slug, deleteSaves }),
  launch: (slug: string, normalOnce = false) => call<Launched>("launch_app", { slug, normalOnce }),
  sandboxStatus: () => call<SandboxStatus>("sandbox_status"),
  /** One app's choice, or "run every app in sandbox mode" when `slug` is null. */
  setSandbox: (slug: string | null, on: boolean, blockNetwork = false) =>
    call<SandboxStatus["settings"]>("set_sandbox", { slug, on, blockNetwork }),
  saveCopies: (slug: string) => call<SaveCopy[]>("save_copies", { slug }),
  restoreSaves: (slug: string, id: string) => call<void>("restore_saves", { slug, id }),
  copySaves: (from: string, to: string) => call<void>("copy_saves", { from, to }),
  saveSize: (slug: string) => call<number>("save_size", { slug }),
  /** Shared runtimes (Visual C++, .NET, DirectX) an installed app needs and this computer lacks. */
  missingRuntimes: (slug: string) =>
    call<Array<{ id: string; name: string; vendor: string }>>("missing_runtimes", { slug }),
  installRuntimes: (slug: string) => call<void>("install_runtimes", { slug }),
  /** What each original's Play button starts, when the player picked a community build. */
  playChoices: () => call<Record<string, string>>("play_choices"),
  /** Picks the build `app`'s Play button starts; null for the original. */
  setPlayChoice: (app: string, build: string | null) => call<void>("set_play_choice", { app, build }),
  /** An app closed with an error right after starting in sandbox mode. */
  onSandboxFailed: async (handler: (e: { slug: string }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ slug: string }>("sandbox-failed", (e) => handler(e.payload));
  },
  /** An app closed with an error: its last output, home folder blanked out (never sent unasked). */
  onAppCrashed: async (handler: (e: { slug: string; code: number | null; log: string }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ slug: string; code: number | null; log: string }>("app-crashed", (e) => handler(e.payload));
  },
  /** Save sync: whether this computer can (signed in, chat set up with backups) and it's on. */
  saveSyncStatus: () => call<{ available: boolean; on: boolean }>("save_sync_status"),
  setSaveSync: (on: boolean) => call<void>("set_save_sync", { on }),
  /** Both computers changed an app's saves: keep this computer's or the other's. */
  resolveSaveConflict: (slug: string, keep: "here" | "other") =>
    call<SaveSyncOutcome | null>("resolve_save_conflict", { slug, keep }),
  /** Saves were sent after an app closed, or couldn't be. */
  onSavesSynced: async (handler: (e: { slug: string; result: SaveSyncOutcome }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ slug: string; result: SaveSyncOutcome }>("saves-synced", (e) => handler(e.payload));
  },
  /** A running game asked for mods (Slop Link). */
  onModRequest: async (handler: (r: ModRequest) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<ModRequest>("mod-request", (e) => handler(e.payload));
  },
  /** Mods were added to `game` from a request. */
  onModsChanged: async (handler: (game: string) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<string>("mods-changed", (e) => handler(e.payload));
  },
  answerModRequest: (id: string, accept: boolean) => call<void>("answer_mod_request", { id, accept }),
  /** Mods: files for an installed game to load, handed to it at launch in SLOP_MODS. */
  installedMods: (game: string) => call<InstalledMod[]>("installed_mods", { game }),
  installMod: (slug: string) => call<InstalledMod>("install_mod", { slug }),
  uninstallMod: (slug: string) => call<void>("uninstall_mod", { slug }),
  runInstaller: (slug: string) => call<void>("run_installer", { slug }),
  /** Chat, encrypted in the app (see ChatAction). */
  chat: <T>(action: ChatAction) => call<T>("chat", { action }),
  /** The in-game overlay (Shift+Tab over a fullscreen Slop game): read, or switch on or off. */
  overlaySetting: (on?: boolean) => call<boolean>("overlay_setting", { on: on ?? null }),
  /** Closes the overlay and returns to the game. */
  overlayHide: () => call<void>("overlay_hide"),
  onOverlayShown: async (handler: (e: { game: string | null }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ game: string | null }>("overlay-shown", (e) => handler(e.payload));
  },
  /** Fires when chat has something new. Returns an unsubscribe function. */
  onChatChanged: async (handler: (c: ChatChanges) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<ChatChanges>("chat-changed", (e) => handler(e.payload));
  },
  /** Install progress events: bytes downloaded so far. Returns an unsubscribe function. */
  onProgress: async (handler: (p: { slug: string; got: number; total: number }) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ slug: string; got: number; total: number }>("install-progress", (e) => handler(e.payload));
  },
};
