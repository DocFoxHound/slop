// SPDX-License-Identifier: AGPL-3.0-or-later
// The service worker (public/sw.js): a push carries nothing, so it fetches the newest unread
// notification with the user's own sign-in and shows that; a click opens its page.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

type Handler = (event: { waitUntil: (p: Promise<unknown>) => void; notification?: unknown }) => void;

/** Runs public/sw.js against a stand-in for the worker's `self`, returning its handlers. */
function load(fetchImpl: typeof fetch, windows: Array<{ url: string; focus: () => Promise<void>; navigate: (u: string) => Promise<void> }> = []) {
  const handlers: Record<string, Handler> = {};
  const self = {
    location: { origin: "https://slop.example" },
    addEventListener: (type: string, h: Handler) => (handlers[type] = h),
    skipWaiting: vi.fn(),
    registration: { showNotification: vi.fn(async () => {}) },
    clients: {
      claim: vi.fn(async () => {}),
      matchAll: vi.fn(async () => windows),
      openWindow: vi.fn(async () => null),
    },
  };
  const code = readFileSync(join(__dirname, "../public/sw.js"), "utf8");
  new Function("self", "fetch", code)(self, fetchImpl);
  const fire = async (type: string, extra: Record<string, unknown> = {}) => {
    let done: Promise<unknown> = Promise.resolve();
    handlers[type]({ waitUntil: (p) => (done = p), ...extra });
    await done;
  };
  return { self, fire };
}

const inbox = (items: unknown[]) =>
  vi.fn(async () => new Response(JSON.stringify({ unread: items.length, items }), { status: 200 })) as unknown as typeof fetch;

describe("public/sw.js", () => {
  it("shows the newest unread notification, with its count and link", async () => {
    const fetchImpl = inbox([
      { id: "n1", title: "Old", body: "", link: "/x", count: 1, read: true },
      { id: "n2", title: "Bo Builder sent a message", body: "Lantern night?", link: "/chat", count: 3, read: false },
    ]);
    const { self, fire } = load(fetchImpl);
    await fire("push");
    expect(fetchImpl).toHaveBeenCalledWith("/v1/me/notifications", expect.objectContaining({ credentials: "include" }));
    expect(self.registration.showNotification).toHaveBeenCalledWith(
      "Bo Builder sent a message (3)",
      expect.objectContaining({ body: "Lantern night?", tag: "n2", data: { link: "/chat" } }),
    );
  });

  it("says something general when signed out or offline", async () => {
    const signedOut = load(vi.fn(async () => new Response("", { status: 401 })) as unknown as typeof fetch);
    await signedOut.fire("push");
    expect(signedOut.self.registration.showNotification).toHaveBeenCalledWith(
      "SLAPorSLOP",
      expect.objectContaining({ body: "You have something new on SLAPorSLOP.", data: { link: "/notifications" } }),
    );
    const offline = load(vi.fn(async () => Promise.reject(new TypeError("offline"))) as unknown as typeof fetch);
    await offline.fire("push");
    expect(offline.self.registration.showNotification).toHaveBeenCalledWith("SLAPorSLOP", expect.anything());
  });

  it("with nothing unread, says something general", async () => {
    const { self, fire } = load(inbox([{ id: "n1", title: "Old", body: "", link: "/x", count: 1, read: true }]));
    await fire("push");
    expect(self.registration.showNotification).toHaveBeenCalledWith("SLAPorSLOP", expect.objectContaining({ data: { link: "/notifications" } }));
  });

  it("a click opens the notification's page in SLAPorSLOP's open window", async () => {
    const win = { url: "https://slop.example/library", focus: vi.fn(async () => {}), navigate: vi.fn(async () => {}) };
    const other = { url: "https://elsewhere.example/", focus: vi.fn(async () => {}), navigate: vi.fn(async () => {}) };
    const { self, fire } = load(inbox([]), [other, win]);
    const close = vi.fn();
    await fire("notificationclick", { notification: { close, data: { link: "/chat" } } });
    expect(close).toHaveBeenCalled();
    expect(win.focus).toHaveBeenCalled();
    expect(win.navigate).toHaveBeenCalledWith("https://slop.example/chat");
    expect(other.navigate).not.toHaveBeenCalled();
    expect(self.clients.openWindow).not.toHaveBeenCalled();
  });

  it("a click with no SLAPorSLOP window open opens one", async () => {
    const { self, fire } = load(inbox([]));
    await fire("notificationclick", { notification: { close: vi.fn(), data: {} } });
    expect(self.clients.openWindow).toHaveBeenCalledWith("https://slop.example/notifications");
  });
});
