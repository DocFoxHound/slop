// SPDX-License-Identifier: AGPL-3.0-or-later
// A pretend desktop app for the browser tests: the website runs as it does inside the Slop app
// (platform.ts `isDesktop`), with the app's Rust side faked in the page. API requests go to the
// test server with the page's sign-in cookie; chat is a small in-page stand-in for the app's
// encrypted chat (crates/slop-client/src/chat), enough to test what the pages do with it.
// Everything the fake was asked is kept in `window.__desktopCalls` for the tests to check.
import type { Page } from "@playwright/test";

export interface FakeChat {
  /** Where chat stands on this device before the test: never set up, waiting, removed, or ready. */
  state: "none" | "pending" | "active" | "revoked";
  /** What setting up turns into: the user's first device is active; later ones wait. */
  setupBecomes?: "active" | "pending";
  /** The device name the app gives a device set up without one (the computer's name). */
  computerName?: string;
  /** Chats this device has, each with no messages yet: [id, title]. */
  conversations?: Array<[string, string]>;
}

/** What the pretend app has installed and is doing, for the library (pages/Library.tsx). */
export interface FakeLibrary {
  /** Installed apps (platform.ts InstalledApp). */
  installed?: Array<{
    slug: string;
    title: string;
    maker?: string;
    version?: string;
    release_id?: string;
    kind?: "archive" | "installer" | "apk";
    build_of?: string | null;
  }>;
  /** Apps Slop started that are still running. */
  running?: string[];
  /** "Update apps automatically" (on unless said). */
  autoUpdate?: boolean;
  /** Sandbox mode works here (with nothing chosen yet). */
  sandbox?: boolean;
  /** What each original's Play button starts. */
  playChoices?: Record<string, string>;
  /** Copies of saves, for every app. */
  saveCopies?: Array<{ id: string; taken_at: string; bytes: number }>;
  /** What installing an app that isn't installed yet puts there (by default, its address as its name). */
  installable?: FakeLibrary["installed"];
  /** Installing waits for `window.__finishInstall()` instead of finishing at once. */
  holdInstalls?: boolean;
}

/** Makes the page the desktop app, before any of its scripts run. `library` sets up what's
 * installed; `window.__emit(event, payload)` sends the page an event from the app (install
 * progress, say). */
export async function asDesktop(page: Page, chat: FakeChat, library: FakeLibrary = {}) {
  await page.addInitScript((all: { chat: FakeChat; library: FakeLibrary }) => {
    const opts = all.chat;
    const lib = all.library;
    const installedApps = (lib.installed ?? []).map((a) => ({
      maker: "Someone",
      version: "1.0.0",
      release_id: `release-${a.slug}`,
      kind: "archive",
      build_of: null,
      installed_at: "2026-10-01T00:00:00Z",
      ...a,
    }));
    const sandboxSettings = { all: false, apps: {} as Record<string, { on: boolean; block_network: boolean }> };
    let autoUpdate = lib.autoUpdate ?? true;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Array<{ event: string; handler: number }> = [];
    type Call = { cmd: string; args: Record<string, unknown> };
    const w = window as unknown as Record<string, unknown>;
    const calls: Call[] = [];
    w.__desktopCalls = calls;
    const device = { id: "0190a000-0000-7000-8000-000000000001", name: opts.computerName ?? "test-pc", status: opts.state, last_seen_at: "", this_device: true };
    const chatState = {
      state: opts.state,
      backups: false,
      conversations: (opts.conversations ?? []).map(([id, title]) => ({
        id,
        kind: "direct",
        title,
        members: [],
        team_slug: null,
        unread: 0,
        muted: false,
        ready: true,
        closed: false,
        last: null,
        last_at: null,
        owner: false,
      })),
    };
    const status = () => ({
      state: chatState.state,
      device_id: chatState.state === "none" ? null : device.id,
      code: chatState.state === "none" ? null : "ABC-DEF-GHI",
      pending: [],
      devices: chatState.state === "none" ? [] : [{ ...device, status: chatState.state }],
      backups: chatState.backups,
    });
    const chat = (a: Record<string, unknown>): unknown => {
      switch (a.action) {
        case "status":
          return status();
        case "setup":
          if (chatState.state === "none" || chatState.state === "revoked") {
            device.name = typeof a.name === "string" && a.name ? a.name : (opts.computerName ?? "test-pc");
            chatState.state = opts.setupBecomes ?? "active";
          }
          return status();
        case "set_backups":
          chatState.backups = Boolean(a.on);
          return a.on ? "RCVR-CODE-1234-5678-ABCD-EFGH" : null;
        case "conversations":
          return chatState.conversations;
        case "messages":
          return [];
        case "start_direct":
          // The pretend app has one chat per person it was given; it opens the first.
          return (chatState.conversations[0] as { id: string } | undefined)?.id ?? null;
        default:
          return null;
      }
    };
    let callback = 0;
    w.__emit = (event: string, payload: unknown) => {
      for (const l of listeners) if (l.event === event) callbacks.get(l.handler)?.({ event, id: l.handler, payload });
    };
    let finish: (() => void) | undefined;
    w.__finishInstall = () => finish?.();
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
      transformCallback: (cb?: (e: unknown) => void) => {
        callback += 1;
        if (cb) callbacks.set(callback, cb);
        return callback;
      },
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string) => p,
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
        switch (cmd) {
          case "api_request": {
            const body = args.body;
            const res = await fetch(String(args.path), {
              method: String(args.methodName),
              credentials: "same-origin",
              headers: body == null ? { Accept: "application/json" } : { Accept: "application/json", "Content-Type": "application/json" },
              body: body == null ? undefined : JSON.stringify(body),
            });
            const text = await res.text();
            let parsed: unknown = null;
            try {
              parsed = text ? JSON.parse(text) : null;
            } catch {
              parsed = text;
            }
            return { status: res.status, body: parsed };
          }
          case "session_status":
            return { signed_in: true };
          case "chat":
            return chat(args.action as Record<string, unknown>);
          case "plugin:event|listen":
            listeners.push({ event: String(args.event), handler: Number(args.handler) });
            return ++callback;
          case "installed_apps":
            return installedApps;
          case "running_apps":
            return lib.running ?? [];
          case "save_copies":
            return lib.saveCopies ?? [];
          case "installed_mods":
          case "missing_runtimes":
            return [];
          case "play_choices":
            return lib.playChoices ?? {};
          case "save_size":
            return 0;
          case "sandbox_status":
            return lib.sandbox ? { available: true, reason: null, limits: null, settings: sandboxSettings } : { available: false, reason: "not on this computer", settings: sandboxSettings };
          case "set_sandbox": {
            const slug = args.slug as string | null;
            if (slug === null) sandboxSettings.all = Boolean(args.on);
            else sandboxSettings.apps[slug] = { on: Boolean(args.on), block_network: Boolean(args.blockNetwork) };
            return sandboxSettings;
          }
          case "save_sync_status":
            return { available: false, on: false };
          case "launch_app":
            return { sandboxed: false, quarantined: [], copy_skipped: false, unapproved_mods: [], save_sync: null };
          case "install_app": {
            const slug = String(args.slug);
            if (lib.holdInstalls) await new Promise<void>((done) => (finish = done));
            const found = installedApps.find((a) => a.slug === slug);
            if (found) return found;
            const app = {
              title: slug,
              maker: "Someone",
              version: "1.0.0",
              release_id: `release-${slug}`,
              kind: "archive",
              build_of: null,
              installed_at: "2026-10-01T00:00:00Z",
              ...(lib.installable ?? []).find((a) => a.slug === slug),
              slug,
            };
            installedApps.push(app);
            return app;
          }
          case "uninstall_app": {
            const at = installedApps.findIndex((a) => a.slug === args.slug);
            if (at >= 0) installedApps.splice(at, 1);
            return null;
          }
          case "auto_update_setting":
            if (typeof args.on === "boolean") autoUpdate = args.on;
            return autoUpdate;
          case "platform_info":
            return { platform: "linux", can_install: false };
          default:
            return null;
        }
      },
    };
  }, { chat, library });
}

/** What the page asked the pretend app's chat to do, in order. */
export async function chatCalls(page: Page): Promise<Array<Record<string, unknown>>> {
  return page.evaluate(() =>
    ((window as unknown as { __desktopCalls: Array<{ cmd: string; args: { action?: Record<string, unknown> } }> }).__desktopCalls ?? [])
      .filter((c) => c.cmd === "chat")
      .map((c) => c.args.action ?? {}),
  );
}
