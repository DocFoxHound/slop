// SPDX-License-Identifier: AGPL-3.0-or-later
// Helpers for the accounts browser tests (accounts-*.e2e.ts): calling the API as a seeded
// account, the desktop app's half of signing in, and answering the few API calls the test server
// can't (it runs on 127.0.0.1, where passkeys can't work, and its fake GitHub can't sign anyone in).
import { createHash, randomBytes } from "node:crypto";
import type { Page, Route } from "@playwright/test";

import { server, type Who } from "./fixtures";

export interface Answer<T = unknown> {
  status: number;
  body: T;
}

/** Calls the API as a seeded account (or signed out), the way the website does. */
export async function tryCall<T = any>(who: Who | null, method: string, path: string, body?: unknown): Promise<Answer<T>> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (who) headers.Cookie = `${server.cookie_name}=${server.cookies[who]}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const res = await fetch(server.url + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let parsed: unknown = text;
  try {
    parsed = text ? JSON.parse(text) : undefined;
  } catch {
    // Not JSON: the text itself.
  }
  return { status: res.status, body: parsed as T };
}

/** Like `tryCall`, but the call must work. */
export async function call<T = any>(who: Who | null, method: string, path: string, body?: unknown): Promise<T> {
  const res = await tryCall<T>(who, method, path, body);
  if (res.status < 200 || res.status > 299) {
    throw new Error(`${method} ${path} as ${who ?? "nobody"}: ${res.status} ${JSON.stringify(res.body)}`);
  }
  return res.body;
}

/** An API error, as crates/slop-api/src/error.rs sends it. */
export const apiError = (code: string, message: string, field?: string) => ({ error: { code, message, ...(field ? { field } : {}) } });

/**
 * Answers an API call with a fixed response instead of the server: for states the test server
 * can't reach. `times` answers only the first so many calls; later ones go to the server.
 */
export async function fake(
  page: Page,
  method: string,
  path: string | RegExp,
  answer: Answer | ((route: Route) => Answer | Promise<Answer>),
  { times }: { times?: number } = {},
): Promise<{ calls: Array<{ url: string; body: unknown }> }> {
  const calls: Array<{ url: string; body: unknown }> = [];
  const matcher = typeof path === "string" ? (url: URL) => url.pathname === path : (url: URL) => path.test(url.pathname + url.search);
  await page.route(
    (url) => matcher(url),
    async (route) => {
      const req = route.request();
      if (req.method() !== method || (times !== undefined && calls.length >= times)) return route.fallback();
      let body: unknown;
      try {
        body = req.postDataJSON();
      } catch {
        body = req.postData();
      }
      calls.push({ url: req.url(), body });
      const a = typeof answer === "function" ? await answer(route) : answer;
      await route.fulfill({
        status: a.status,
        contentType: "application/json",
        body: a.body === undefined ? "" : JSON.stringify(a.body),
      });
    },
  );
  return { calls };
}

/** Lets the server answer, then changes what the page sees (one field of /v1/me, say). */
export async function tweak<T = any>(page: Page, path: string, change: (body: T) => T) {
  await page.route(
    (url) => url.pathname === path,
    async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      // A navigation (an old /account#… link moving to its section, say) can cancel a request
      // that's still on its way; only the page's next one matters then.
      try {
        const res = await route.fetch();
        const body = (await res.json()) as T;
        await route.fulfill({ response: res, json: change(body) });
      } catch {
        // Cancelled by the navigation.
      }
    },
  );
}

/**
 * Stands in for the browser's passkey prompt (navigator.credentials): passkeys can't work on the
 * test server's 127.0.0.1 address, for the browser or for the server. "cancel" is the person
 * closing the prompt. The calls are counted in `window.__passkeyPrompts`.
 */
export async function stubPasskeys(page: Page, mode: "ok" | "cancel" = "ok") {
  await page.addInitScript((mode) => {
    const buf = () => new Uint8Array([1, 2, 3, 4]).buffer;
    const credential = {
      id: "AQIDBA",
      rawId: buf(),
      type: "public-key",
      response: {
        attestationObject: buf(),
        clientDataJSON: buf(),
        authenticatorData: buf(),
        signature: buf(),
        userHandle: null,
        getTransports: () => ["internal"],
      },
      getClientExtensionResults: () => ({}),
    };
    const w = window as unknown as { __passkeyPrompts: number };
    w.__passkeyPrompts = 0;
    const prompt = async () => {
      w.__passkeyPrompts++;
      if (mode === "cancel") throw new DOMException("The operation either timed out or was not allowed.", "NotAllowedError");
      return credential;
    };
    Object.defineProperty(navigator, "credentials", { value: { create: prompt, get: prompt }, configurable: true });
  }, mode);
}

/** A browser without passkeys at all. */
export async function noPasskeys(page: Page) {
  await page.addInitScript(() => {
    delete (window as { PublicKeyCredential?: unknown }).PublicKeyCredential;
  });
}

/** WebAuthn options as the server sends them (just enough for web/src/passkeys.ts). */
export const passkeyOptions = {
  publicKey: {
    challenge: "AAAAAAAAAAAAAAAAAAAAAA",
    rp: { id: "127.0.0.1", name: "Slop" },
    user: { id: "AAAAAAAAAAAAAAAAAAAAAA", name: "Someone", displayName: "Someone" },
    pubKeyCredParams: [{ type: "public-key", alg: -7 }],
  },
};

/** The desktop app's half of signing in (crates/slop-api/src/auth/desktop.rs): a request to approve. */
export async function desktopSignInRequest(deviceName: string, port: number) {
  const verifier = randomBytes(32).toString("base64url");
  const state = randomBytes(16).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const made = await call<{ id: string; approve_url: string }>(null, "POST", "/v1/auth/desktop/requests", {
    code_challenge: challenge,
    redirect_port: port,
    state,
    device_name: deviceName,
  });
  return { ...made, verifier, state };
}

/** A unique, valid address for something a test makes. */
export const unique = (prefix: string) => `${prefix}-${randomBytes(3).toString("hex")}`;

/**
 * The account page's sections (web/src/accountNav.ts): each at its own address, one shown at a
 * time. Profile is /account itself.
 */
export type AccountSection = "profile" | "security" | "device" | "agents" | "notifications" | "money" | "moderation" | "agreements" | "data";

/** The address of a section, with a part of it (`#passkeys`) if given. */
export const accountUrl = (section: AccountSection, part = "") => (section === "profile" ? "/account" : `/account/${section}`) + part;

/** Opens the account page at a section, and waits for it to show. */
export async function openAccount(page: Page, section: AccountSection = "profile", part = "") {
  await page.goto(accountUrl(section, part));
  await accountNav(page).locator('[aria-current="page"]').waitFor();
}

/** The list of the account page's sections. */
export function accountNav(page: Page) {
  return page.getByRole("navigation", { name: "Account settings" });
}
