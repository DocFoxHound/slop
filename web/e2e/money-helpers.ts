// SPDX-License-Identifier: AGPL-3.0-or-later
// What the money tests (money-*.e2e.ts) make for themselves through the API, so they never change
// the seeded apps, bounties or donations other tests read: their own listed apps (with an approved
// release), a community build of one, bounties and pledges on them. Everything is made once per
// test server; a second call finds it.
import { createHash } from "node:crypto";

import { server, type Who } from "./fixtures";

export interface Answer<T> {
  status: number;
  body: T;
}

/** One API call as a seeded account (or nobody), as the website would make it. */
export async function call<T = any>(who: Who | null, method: string, path: string, body?: unknown): Promise<Answer<T>> {
  const headers: Record<string, string> = { Accept: "application/json", Origin: server.url };
  if (who) headers.Cookie = `${server.cookie_name}=${server.cookies[who]}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const res = await fetch(server.url + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  return { status: res.status, body: (text ? JSON.parse(text) : null) as T };
}

/** `call`, failing unless it worked. */
export async function api<T = any>(who: Who | null, method: string, path: string, body?: unknown): Promise<T> {
  const res = await call<T>(who, method, path, body);
  if (res.status >= 300) throw new Error(`${method} ${path} as ${who}: ${res.status} ${JSON.stringify(res.body)}`);
  return res.body;
}

/** Pays a (fake) Stripe Checkout link as its payer would: Slop gets the webhooks before this returns. */
export async function payCheckout(url: string): Promise<void> {
  const res = await fetch(`${url}/pay`, { method: "POST", redirect: "manual" });
  if (res.status >= 400) throw new Error(`paying ${url}: ${res.status} ${await res.text()}`);
}

async function exists(who: Who, slug: string): Promise<boolean> {
  return (await call(who, "GET", `/v1/apps/${slug}`)).status === 200;
}

/** Submits release 1.0.0 (the fake GitHub's zip), waits for Slop to fetch it, and has staff approve it. */
async function approvedRelease(maker: Who, slug: string) {
  const releases = await api<Array<{ id: string; status: string }>>(maker, "GET", `/v1/apps/${slug}/releases`);
  if (releases.some((r) => r.status === "approved")) return;
  let release = releases.find((r) => r.status !== "withdrawn" && r.status !== "rejected");
  if (!release) {
    release = await api(maker, "POST", `/v1/apps/${slug}/releases`, {
      version: "1.0.0",
      commit_sha: "c".repeat(40),
      files: [
        {
          platform: "windows",
          kind: "archive",
          url: `${server.fake}e2e/${slug}/releases/download/v1.0.0/${slug}.zip`,
          exec: "pong.exe",
        },
      ],
    });
  }
  for (let i = 0; i < 100 && release!.status !== "awaiting_review"; i++) {
    await new Promise((r) => setTimeout(r, 200));
    const now = await api<Array<{ id: string; status: string }>>(maker, "GET", `/v1/apps/${slug}/releases`);
    release = now.find((r) => r.id === release!.id)!;
  }
  if (release!.status !== "awaiting_review") throw new Error(`release of ${slug} is ${release!.status}`);
  await api("admin", "POST", `/v1/admin/releases/${release!.id}/approve`, {});
}

async function donationsOn(maker: Who, slug: string, servers: string | null) {
  await api(maker, "PUT", `/v1/apps/${slug}/donations/settings`, {
    donations_on: true,
    donation_suggested_minor: 300,
    donation_currency: "usd",
    runs_servers: servers !== null,
    servers_statement: servers,
  });
}

/** A maker's own listed game with an approved release (and donations on, when asked). */
export async function listedApp(
  maker: Who,
  slug: string,
  title: string,
  opts: { donations?: boolean; servers?: string } = {},
): Promise<string> {
  if (!(await exists(maker, slug))) {
    await api(maker, "POST", "/v1/apps", {
      slug,
      title,
      summary: `${title}, for the money tests.`,
      description_md: `# ${title}\n\nMade for Slop's browser tests of donations and bounties.`,
      kind: "game",
      stage: "released",
      repo_url: `${server.fake}e2e/${slug}`,
      license_spdx: "MIT",
      min_age: 12,
      tags: ["puzzle"],
      platforms: ["windows"],
    });
    await approvedRelease(maker, slug);
    await api("admin", "POST", `/v1/admin/apps/${slug}/list`, {});
    if (opts.donations) await donationsOn(maker, slug, opts.servers ?? null);
  }
  return slug;
}

/** A community build of `original` by `author`; released (and listed) unless `released` is false. */
export async function communityBuild(
  author: Who,
  original: string,
  slug: string,
  title: string,
  opts: { released?: boolean; donations?: boolean } = {},
): Promise<string> {
  if (!(await exists(author, slug))) {
    await api(author, "POST", `/v1/apps/${original}/branches`, {
      slug,
      title,
      summary: `${title}: a community build for the money tests.`,
      repo_url: `${server.fake}e2e/${slug}`,
      license_spdx: "MIT",
      kind: "port",
      note: "Faster loading.",
      platforms: ["windows"],
    });
    if (opts.released !== false) {
      await approvedRelease(author, slug);
      await api("admin", "POST", `/v1/admin/apps/${slug}/list`, {});
      if (opts.donations) await donationsOn(author, slug, null);
    }
  }
  return slug;
}

/**
 * The apps the money tests share: Mia's "Tip Jar Tycoon" (donations on), Bo's community build of it
 * "Tip Jar Tycoon Plus" (released, donations on: one-off donations to it are shared with Mia) and his
 * unreleased "Tip Jar Tycoon Nightly".
 */
export async function moneyApps() {
  const original = await listedApp("maker", "tip-jar", "Tip Jar Tycoon", { donations: true });
  const build = await communityBuild("maker2", original, "tip-jar-plus", "Tip Jar Tycoon Plus", { donations: true });
  const nightly = await communityBuild("maker2", original, "tip-jar-nightly", "Tip Jar Tycoon Nightly", { released: false });
  return { original, build, nightly };
}

/** A new bounty on `slug`, opened through the API. */
export async function openBounty(who: Who, slug: string, title: string, body = "Please add it."): Promise<string> {
  const b = await api<{ id: string }>(who, "POST", `/v1/apps/${slug}/bounties`, { title, body, currency: "usd" });
  return b.id;
}

/** Answers Slop's proof-of-work check (challenge.rs) as the browser would. */
async function challenge(who: Who) {
  const c = await api<{ id: string; prefix: string; difficulty: number }>(who, "POST", "/v1/donations/challenge");
  for (let n = 0; ; n++) {
    const hash = createHash("sha256").update(`${c.prefix}:${n}`).digest();
    let zeros = 0;
    for (const byte of hash) {
      zeros += byte === 0 ? 8 : Math.clz32(byte) - 24;
      if (byte !== 0) break;
    }
    if (zeros >= c.difficulty) return { id: c.id, nonce: String(n) };
  }
}

/** A POST that may ask for the proof-of-work check first (donations and pledges). */
async function postChecked<T>(who: Who, path: string, body: Record<string, unknown>): Promise<T> {
  const first = await call<T>(who, "POST", path, body);
  if ((first.body as any)?.error?.code !== "challenge_required") {
    if (first.status >= 300) throw new Error(`POST ${path} as ${who}: ${first.status} ${JSON.stringify(first.body)}`);
    return first.body;
  }
  return api(who, "POST", path, { ...body, challenge: await challenge(who) });
}

/** Starts a pledge (its card not saved yet). */
export async function startPledge(who: Who, bounty: string, amountMinor: number): Promise<{ url: string | null }> {
  return postChecked(who, `/v1/bounties/${bounty}/pledge`, { amount_minor: amountMinor, confirm_adult: true });
}

/** A donation paid through the (fake) Stripe Checkout; its ID. */
export async function donate(
  who: Who,
  slug: string,
  body: { amount_minor: number; monthly?: boolean; share_name?: boolean; message?: string; consent?: boolean },
): Promise<string> {
  const started = await postChecked<{ id: string; url: string | null }>(who, `/v1/apps/${slug}/donate`, {
    confirm_adult: true,
    ...body,
  });
  if (started.url) await payCheckout(started.url);
  return started.id;
}

/** A pledge whose card is saved (the fake Stripe Checkout, paid). */
export async function pledge(who: Who, bounty: string, amountMinor: number): Promise<void> {
  const started = await startPledge(who, bounty, amountMinor);
  if (started.url) await payCheckout(started.url);
}

/**
 * For offering fixes, whose pull requests must be on an https repository: Mia's coming-soon
 * "Firefly Garden" (listed, no release needed) and Bo's community build of it.
 */
export async function offerApps() {
  if (!(await exists("maker", "firefly-garden"))) {
    await api("maker", "POST", "/v1/apps", {
      slug: "firefly-garden",
      title: "Firefly Garden",
      summary: "Catch fireflies before dawn. Coming soon.",
      description_md: "# Firefly Garden\n\nFor the money tests' offered fixes.",
      kind: "game",
      stage: "coming_soon",
      repo_url: "https://github.com/e2e/firefly-garden",
      license_spdx: "MIT",
      min_age: 12,
      tags: ["puzzle"],
      platforms: ["windows"],
    });
    await api("admin", "POST", "/v1/admin/apps/firefly-garden/list", {});
  }
  if (!(await exists("maker2", "firefly-garden-remix"))) {
    await api("maker2", "POST", "/v1/apps/firefly-garden/branches", {
      slug: "firefly-garden-remix",
      title: "Firefly Garden Remix",
      summary: "Firefly Garden with night-vision colours.",
      repo_url: "https://github.com/bo/firefly-garden-remix",
      license_spdx: "MIT",
      kind: "port",
      note: "Night-vision colours.",
      platforms: ["windows"],
    });
  }
  return { original: "firefly-garden", build: "firefly-garden-remix", repo: "https://github.com/e2e/firefly-garden" };
}
