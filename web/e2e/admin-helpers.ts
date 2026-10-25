// SPDX-License-Identifier: AGPL-3.0-or-later
// What the admin-*.e2e.ts tests share: calling the API as a seeded account (to make the things
// a test changes, so the seed stays as the other tests expect), and making apps, releases,
// teams, reports and People jobs of their own.
import { expect, type Browser, type BrowserContext, type Page } from "@playwright/test";

import { server, type Who } from "./fixtures";

const opened: BrowserContext[] = [];

/** Another person's browser in the same test; closed after it (`test.afterEach(closePages)`). */
export async function newPage(browser: Browser): Promise<Page> {
  const context = await browser.newContext();
  opened.push(context);
  return context.newPage();
}

export async function closePages() {
  for (const context of opened.splice(0)) await context.close();
}

/** A short tag that keeps this run's names apart from everyone else's. */
export const tag = () => Math.random().toString(36).slice(2, 7);

export interface Answer {
  status: number;
  // The API's answers are JSON of every shape; tests read the fields they need.
  json: any;
}

/** Calls the API as `who` (or signed out); returns the status and the answer. */
export async function call(who: Who | null, method: string, path: string, body?: unknown): Promise<Answer> {
  const headers: Record<string, string> = { accept: "application/json" };
  if (who) headers.cookie = `${server.cookie_name}=${server.cookies[who]}`;
  if (body !== undefined) headers["content-type"] = "application/json";
  const res = await fetch(server.url + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    redirect: "manual",
  });
  const text = await res.text();
  let json: unknown = text;
  try {
    json = JSON.parse(text);
  } catch {
    // Not JSON: the text itself.
  }
  return { status: res.status, json };
}

/** Like `call`, but the call must succeed. */
export async function ok(who: Who | null, method: string, path: string, body?: unknown): Promise<Answer["json"]> {
  const r = await call(who, method, path, body);
  expect(r.status, `${method} ${path} as ${who}: ${JSON.stringify(r.json)}`).toBeLessThan(300);
  return r.json;
}

/** Calls the admin API with an admin API token (an AI reviewer). */
export async function withToken(token: string, method: string, path: string, body?: unknown): Promise<Answer> {
  const res = await fetch(server.url + path, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json", accept: "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json: unknown = text;
  try {
    json = JSON.parse(text);
  } catch {
    // Not JSON.
  }
  return { status: res.status, json };
}

/** A category of the review queue, the inbox (web/src/pages/admin/queue.ts): "releases", "people"… */
export const inbox = (box: string) => `/admin?tab=queue&box=${box}`;

/** The line for an item in the inbox's list. */
export const inboxRow = (page: Page, text: string | RegExp) => page.locator(".inbox-row").filter({ hasText: text });

/** Opens an item of a review-queue category from its line, as staff would: its full card shows. */
export async function openItem(page: Page, box: string, text: string | RegExp) {
  await page.goto(inbox(box));
  await inboxRow(page, text).getByRole("link").first().click();
  await expect(page).toHaveURL(/[?&]item=/);
  await expect(page.locator(".inbox-detail")).toBeVisible();
}

/** A repository on the fake GitHub, which serves release files (so releases can be fetched). */
export const fakeRepo = (slug: string) => `${server.fake}bo/${slug}`;
/** A github.com repository: its store pictures come from the test server's /raw/ (main.rs). */
export const githubRepo = (slug: string) => `https://github.com/e2e/${slug}`;

export interface NewApp {
  slug: string;
  title: string;
  stage?: "coming_soon" | "beta" | "released";
  repo?: string;
  visibility?: "public" | "private";
  summary?: string;
}

/**
 * Who makes the tests' apps. New apps are limited per account and day (limits.rs), so the
 * seeded makers' allowance is left for the tests that need them; staff can publish too.
 */
export const MAKER: Who = "finance";
export const MAKER_NAME = "Fin Finance";

/** A store page (a draft) made by `who`. */
export async function newApp(app: NewApp, who: Who = MAKER) {
  return ok(who, "POST", "/v1/apps", {
    slug: app.slug,
    title: app.title,
    summary: app.summary ?? "Made by the admin browser tests.",
    description_md: `# ${app.title}\n\nMade by the admin browser tests.`,
    kind: "game",
    stage: app.stage ?? "released",
    repo_url: app.repo ?? fakeRepo(app.slug),
    license_spdx: "MIT",
    min_age: 12,
    tags: [],
    platforms: ["windows"],
    ...(app.visibility ? { visibility: app.visibility } : {}),
  });
}

/** Submits a Windows release from the fake GitHub and waits until it's waiting for review. */
export async function newRelease(slug: string, version: string, who: Who = MAKER): Promise<string> {
  const r = await ok(who, "POST", `/v1/apps/${slug}/releases`, {
    version,
    commit_sha: "b".repeat(40),
    files: [
      {
        platform: "windows",
        kind: "archive",
        url: `${fakeRepo(slug)}/releases/download/v${version}/${slug}.zip`,
        exec: "pong.exe",
      },
    ],
  });
  const id: string = r.id;
  await expect
    .poll(
      async () => {
        const all = await ok(who, "GET", `/v1/apps/${slug}/releases`);
        return (all as Array<{ id: string; status: string }>).find((x) => x.id === id)?.status;
      },
      { timeout: 20_000 },
    )
    .toBe("awaiting_review");
  return id;
}

/** Approves a release as staff (Ada), as the review queue would. */
export const approveRelease = (id: string) => ok("admin", "POST", `/v1/admin/releases/${id}/approve`);

/** An app in the store with an approved release. */
export async function listedApp(app: NewApp, who: Who = MAKER) {
  await newApp(app, who);
  const release = await newRelease(app.slug, "1.0.0", who);
  await approveRelease(release);
  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/list`);
  return release;
}

/** Opens the decisions about `who` the way their account page does, so no "new decision" notice
 *  stays on their pages for the tests that come later. */
export const seeDecisions = (who: Who) => ok(who, "GET", "/v1/me/decisions");

/** A team of `who`'s, open to anyone (so anyone can see it, and report it). */
export async function newTeam(who: Who, slug: string, name: string) {
  return ok(who, "POST", "/v1/teams", {
    slug,
    name,
    kind: "player",
    join_policy: "open",
    about: "Made by the admin browser tests.",
  });
}

/** Sends a report as `who`; returns its reference. */
export async function report(who: Who, kind: string, subject: string, reason: string, details: string): Promise<string> {
  const r = await ok(who, "POST", "/v1/reports", { kind, subject, reason, details, good_faith: true });
  return r.reference;
}

/** An admin API token of Ada's. */
export async function adminToken(name: string, scope: "read" | "review" | "moderate"): Promise<{ id: string; token: string }> {
  return ok("admin", "POST", "/v1/admin/tokens", { name, scope, days: 7 });
}

// ---------------------------------------------------------------------------------------------
// People: Cleo (client) hires Vera (creative) for Cleo's Tavern, as the seed does.

const offer = (title: string) => ({
  title,
  scope_md: "Made by the admin browser tests.",
  price_minor: 5000,
  currency: "usd",
  payment: "on_delivery",
  due_date: new Date(Date.now() + 14 * 86_400_000).toISOString().slice(0, 10),
  revisions: 1,
  rights: "transfer",
  open_source: false,
  credit: true,
  ai_use: "none",
});

/** A job Vera accepted: returns its ID. */
export async function acceptedJob(request: string, title: string): Promise<string> {
  const job = await ok("client", "POST", "/v1/people/vera-pixels/hire", {
    app_slug: "cleos-tavern",
    skill: "pixel-art",
    service_id: null,
    request_md: request,
    confirm_adult: true,
    exact_wording_ack: true,
    inquiry: false,
  });
  await ok("creative", "POST", `/v1/jobs/${job.id}/offer`, offer(title));
  await ok("client", "POST", `/v1/jobs/${job.id}/accept`, { offer_count: 1, exact_terms_ack: true });
  return job.id;
}

/** A job delivered, paid through (fake) Stripe Checkout and finished: returns its ID. */
export async function completedJob(request: string, title: string): Promise<string> {
  const id = await acceptedJob(request, title);
  await ok("creative", "POST", `/v1/jobs/${id}/deliver`, { links: ["https://drive.example/preview.png"] });
  const pay = await ok("client", "POST", `/v1/jobs/${id}/pay`, {});
  // The fake Stripe's Pay button.
  const paid = await fetch(`${pay.url}/pay`, { method: "POST", redirect: "manual" });
  expect(paid.status).toBeLessThan(400);
  await expect.poll(async () => (await ok("client", "GET", `/v1/jobs/${id}`)).status, { timeout: 10_000 }).not.toBe("delivered");
  await ok("creative", "POST", `/v1/jobs/${id}/final-files`, { links: ["https://drive.example/final.zip"] });
  return id;
}
