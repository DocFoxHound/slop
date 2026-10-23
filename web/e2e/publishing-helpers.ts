// SPDX-License-Identifier: AGPL-3.0-or-later
// What the publishing tests (publishing-*.e2e.ts) share: calling the API as a seeded account,
// making their own apps and releases (never changing the seeded ones), and cleaning up the
// notifications their actions send to seeded accounts, so other tests' unread counts don't move.
//
// Making an app counts against the account's "10 new apps a day" (crates/slop-api/src/limits.rs),
// failed attempts included. The whole suite makes far more than that as the same few seeded
// accounts, so the test server raises every per-account limit (examples/e2e_server/main.rs). The
// publishing tests send form errors as Fin Finance, who publishes nothing here (though the admin
// tests make apps as Fin).
import { expect, type Page, type Route } from "@playwright/test";

import { server, type Who } from "./fixtures";

/** A slug no other test uses. */
export function uniq(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}${Math.floor(Math.random() * 1296).toString(36)}`;
}

/** Calls the API as `who` (or signed out); fails the test unless it answers 2xx. */
export async function call<T = unknown>(who: Who | null, method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(server.url + path, {
    method,
    headers: {
      accept: "application/json",
      origin: server.url,
      ...(who ? { cookie: `${server.cookie_name}=${server.cookies[who]}` } : {}),
      ...(body !== undefined ? { "content-type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} answered ${res.status}: ${text}`);
  return (text ? JSON.parse(text) : undefined) as T;
}

/** A repository on the fake GitHub host (not github.com): release files there download for real. */
export function fakeRepo(owner: string, slug: string): string {
  return `${server.fake}${owner}/${slug}`;
}

/** A Windows build on the fake host's release page (the same small zip for every name). */
export function fakeZip(repo: string, version: string): string {
  return `${repo}/releases/download/v${version}/game-${version}.zip`;
}

/** A draft made through the API, as the Publish form would. */
export async function createApp(
  who: Who,
  slug: string,
  title: string,
  repo: string,
  extra: Record<string, unknown> = {},
): Promise<void> {
  await call(who, "POST", "/v1/apps", {
    slug,
    title,
    summary: `${title}, for the publishing browser tests.`,
    description_md: `# ${title}\n\nMade for the publishing browser tests.`,
    kind: "game",
    stage: "released",
    repo_url: repo,
    license_spdx: "MIT",
    min_age: 0,
    tags: [],
    platforms: ["windows"],
    ...extra,
  });
}

interface ReleaseRow {
  id: string;
  version: string;
  status: string;
}

/** Submits a Windows release from the fake host, waits for Slop to fetch it, and returns its ID. */
export async function submitRelease(who: Who, slug: string, repo: string, version: string, notes?: string): Promise<string> {
  const release = await call<ReleaseRow>(who, "POST", `/v1/apps/${slug}/releases`, {
    version,
    commit_sha: "c".repeat(40),
    files: [{ platform: "windows", kind: "archive", url: fakeZip(repo, version), exec: "pong.exe", args: [] }],
    notes,
  });
  await expect
    .poll(async () => (await call<ReleaseRow[]>(who, "GET", `/v1/apps/${slug}/releases`)).find((r) => r.id === release.id)?.status, {
      timeout: 20_000,
    })
    .toBe("awaiting_review");
  return release.id;
}

/** Staff approve (and sign) a release, through the review queue's route. */
export async function approve(releaseId: string): Promise<void> {
  await call("admin", "POST", `/v1/admin/releases/${releaseId}/approve`, {});
}

/** A seeded account's unread notifications that link to `path`, marked read. */
export async function readNotices(who: Who, path: string): Promise<void> {
  const list = await call<{ items: Array<{ id: string; link: string | null; read_at?: string | null }> }>(
    who,
    "GET",
    "/v1/me/notifications",
  );
  const ids = list.items.filter((n) => n.link?.includes(path)).map((n) => n.id);
  if (ids.length > 0) await call(who, "POST", "/v1/me/notifications/read", { ids });
}

/** Answers one API route with `body` (for states the test server can't reach, like GitHub's). */
export async function answer(page: Page, path: string | RegExp, body: unknown, status = 200, method?: string) {
  await page.route(typeof path === "string" ? `**${path}` : path, (route: Route) => {
    if (method && route.request().method() !== method) return route.fallback();
    return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
  });
}

/** The API's error answer. */
export function apiError(code: string, message: string, field?: string) {
  return { error: { code, message, ...(field ? { field } : {}) } };
}
