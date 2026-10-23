// SPDX-License-Identifier: AGPL-3.0-or-later
// What the store's browser tests (store-*.e2e.ts) make for themselves through the API, so they
// never change what the seed made for everyone: apps of their own, listed by staff, with approved
// releases, and API calls as any account; plus ways to make the API fail or answer differently
// for one page (page.route). The apps' makers already have a passkey deadline (Remy Pending, Vera
// Okafor, Bo Builder), so publishing changes nothing else about them. Each account may start ten
// apps a day (limits.rs): the files share that, so creating is idempotent (a file's hooks run
// again after a failed test), and public apps leave the store when their file is done.
import type { Page, Route } from "@playwright/test";
import { server, type Who } from "./fixtures";

/** Calls the API as `who` (or signed out with null). Throws unless it answers 2xx. */
export async function call<T = any>(who: Who | null, method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(server.url + path, {
    method,
    headers: {
      Accept: "application/json",
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      ...(who ? { Cookie: `${server.cookie_name}=${server.cookies[who]}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} as ${who ?? "nobody"}: ${res.status} ${text}`);
  return (text ? JSON.parse(text) : undefined) as T;
}

/** Calls the API and returns the status and the JSON answer, whatever the status. */
export async function tryCall(who: Who | null, method: string, path: string, body?: unknown) {
  const res = await fetch(server.url + path, {
    method,
    headers: {
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      ...(who ? { Cookie: `${server.cookie_name}=${server.cookies[who]}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  return { status: res.status, body: text ? JSON.parse(text) : null };
}

export interface AppOptions {
  who?: Who;
  stage?: "coming_soon" | "beta" | "released";
  tags?: string[];
  min_age?: number;
  content_labels?: string[];
  accessibility?: string[];
  description_md?: string;
  summary?: string;
  platforms?: string[];
  visibility?: "public" | "private";
  /** On GitHub (pictures can be read from it) rather than the fake (releases can be fetched). */
  github?: boolean;
}

/** The app as its maker sees it, or null when there's none at that address. */
async function mine(who: Who, slug: string): Promise<{ status: string; stage: string; visibility?: string } | null> {
  const r = await tryCall(who, "GET", `/v1/apps/${slug}`);
  return r.status === 200 ? r.body : null;
}

/** A draft store page (POST /v1/apps), unless it's there already (a test run again after a
 * failure starts its file's hooks again). Returns its address. */
export async function draftApp(slug: string, title: string, o: AppOptions = {}): Promise<string> {
  const who = o.who ?? "maker2";
  if (await mine(who, slug)) return slug;
  await call(who, "POST", "/v1/apps", {
    slug,
    title,
    summary: o.summary ?? `${title}, made by the store's browser tests.`,
    description_md: o.description_md ?? `# ${title}\n\nMade by the store's browser tests.`,
    kind: "game",
    stage: o.stage ?? "released",
    repo_url: o.github ? `https://github.com/e2e/${slug}` : `${server.fake}bo/${slug}`,
    license_spdx: "MIT",
    min_age: o.min_age ?? 0,
    content_labels: o.content_labels ?? [],
    tags: o.tags ?? [],
    platforms: o.platforms ?? ["windows"],
    accessibility: o.accessibility ?? [],
    visibility: o.visibility ?? "public",
  });
  return slug;
}

/** An approved release, unless the app has one already. */
export async function released(slug: string, who: Who = "maker2"): Promise<void> {
  const releases = await call<Array<{ status: string }>>(who, "GET", `/v1/apps/${slug}/releases`);
  if (!releases.some((r) => r.status === "approved")) await approvedRelease(slug, "1.0.0", who);
}

/** Submits a release from the fake GitHub, waits for Slop to fetch it, and has staff approve it. */
export async function approvedRelease(slug: string, version = "1.0.0", who: Who = "maker2"): Promise<string> {
  const release = await call<{ id: string }>(who, "POST", `/v1/apps/${slug}/releases`, {
    version,
    commit_sha: "c".repeat(40),
    files: [
      {
        platform: "windows",
        kind: "archive",
        url: `${server.fake}bo/${slug}/releases/download/v${version}/${slug}.zip`,
        exec: "pong.exe",
      },
    ],
  });
  for (let i = 0; ; i++) {
    const releases = await call<Array<{ id: string; status: string }>>(who, "GET", `/v1/apps/${slug}/releases`);
    const status = releases.find((r) => r.id === release.id)?.status;
    if (status === "awaiting_review") break;
    if (i > 100 || (status && !["fetching", "scanning"].includes(status))) {
      throw new Error(`release ${slug} ${version} is ${status}`);
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  await call("admin", "POST", `/v1/admin/releases/${release.id}/approve`);
  return release.id;
}

/** Staff put a draft in the store. */
export async function list(slug: string) {
  await call("admin", "POST", `/v1/admin/apps/${slug}/list`);
}

/** A game in the store: Bo's unless said, released with an approved 1.0.0 unless said. Picks
 * up where it left off when it's there already (and lists it again if a test took it out). */
export async function listedApp(slug: string, title: string, o: AppOptions = {}): Promise<string> {
  const who = o.who ?? "maker2";
  await draftApp(slug, title, o);
  if ((o.stage ?? "released") !== "coming_soon") await released(slug, who);
  if ((await mine(who, slug))?.status !== "listed") await list(slug);
  return slug;
}

/** Takes the tests' public apps out of the store again when a file is done with them. */
export async function delist(...slugs: string[]) {
  for (const slug of slugs) {
    await tryCall("admin", "POST", `/v1/admin/apps/${slug}/delist`, {
      rule: "misleading",
      reason: "The store's browser tests are done with it.",
    });
  }
}

/** `who` played it through Slop (needed to vote, review and confirm accessibility). */
export async function played(who: Who, slug: string) {
  await call(who, "POST", `/v1/apps/${slug}/launch?platform=windows`);
}

type Match = string | RegExp | ((u: URL) => boolean);

/** Answers matching API requests (of `method`, when given) with an error, as a failing server would. */
export async function failApi(page: Page, url: Match, o: { status?: number; message?: string; code?: string; field?: string; method?: string } = {}) {
  await page.route(url, (route: Route) => {
    if (o.method && route.request().method() !== o.method) return route.fallback();
    return route.fulfill({
      status: o.status ?? 500,
      contentType: "application/json",
      body: JSON.stringify({ error: { code: o.code ?? "internal", message: o.message ?? "Something broke on Slop's side.", field: o.field } }),
    });
  });
}

/** Changes the JSON the API answers for matching requests. */
export async function patchJson<T = any>(page: Page, url: Match, change: (body: T, route: Route) => T) {
  await page.route(url, async (route) => {
    const res = await route.fetch();
    const body = (await res.json()) as T;
    await route.fulfill({ response: res, json: change(body, route) });
  });
}

/** Matches one API path exactly, whatever its query, for page.route. */
export const path = (p: string) => (u: URL) => u.pathname === p;
