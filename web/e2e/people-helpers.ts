// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared by the People, jobs, friends and teams browser tests (people-*.e2e.ts): calling the API
// as a seeded account, to set up what a test needs without changing what other tests read.
import { expect, type Page } from "@playwright/test";
import { server, type Who } from "./fixtures";

/** Calls the API as `who`; returns the JSON answer, failing the test on an error status. */
export async function call<T = any>(who: Who, method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(server.url + path, {
    method,
    headers: {
      Cookie: `${server.cookie_name}=${server.cookies[who]}`,
      Origin: server.url,
      Accept: "application/json",
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} as ${who}: ${res.status} ${text}`);
  return (text ? JSON.parse(text) : null) as T;
}

/**
 * A job page's next step (its status panel). Scoped to the page itself: the notices above every
 * page are a status too, and an account may have one from another test (a moderation decision
 * when another file takes its app out of the store, say).
 */
export function jobStatus(page: Page) {
  return page.getByRole("main").getByRole("status");
}

/** Fourteen days from now, as the offer form's date field wants it. */
export function inDays(days: number): string {
  return new Date(Date.now() + days * 86400_000).toISOString().slice(0, 10);
}

/** An offer body for POST /v1/jobs/{id}/offer. */
export function offerBody(title: string, payment: "split" | "upfront" | "on_delivery", extra: Record<string, unknown> = {}) {
  return {
    title,
    scope_md: "Exactly as asked: PNG files, 32×32.",
    price_minor: 10000,
    currency: "usd",
    payment,
    due_date: inDays(14),
    sign_off: false,
    rights: "transfer",
    open_source: false,
    credit: false,
    ai_use: "none",
    refund_policy: "",
    ...extra,
  };
}

/**
 * Cleo (client) asks Vera (creative) for work on Cleo's Tavern, through the API. Returns the job's
 * ID. Cleo may have three open requests with Vera at once (the seed holds one), so tests move or
 * end their jobs before making more.
 */
export async function hireVera(request: string, who: Who = "client", app = "cleos-tavern", inquiry = false): Promise<string> {
  const job = await call<{ id: string }>(who, "POST", "/v1/people/vera-pixels/hire", {
    app_slug: app,
    skill: "pixel-art",
    request_md: request,
    client_is_business: false,
    inquiry,
    exact_wording_ack: true,
    confirm_adult: true,
  });
  return job.id;
}

/** Vera offers; the client accepts (exact terms acknowledged). */
export async function offerAndAccept(job: string, offer: ReturnType<typeof offerBody>, client: Who = "client") {
  const sent = await call<{ offer_count: number }>("creative", "POST", `/v1/jobs/${job}/offer`, offer);
  return call(client, "POST", `/v1/jobs/${job}/accept`, {
    offer_count: sent.offer_count,
    exact_terms_ack: true,
    start_now: true,
  });
}

/** On the fake Stripe Checkout page: pays and waits to be back on the job's page. */
export async function payOnFakeStripe(page: Page, job: string) {
  await expect(page.getByRole("heading", { name: "Fake Stripe Checkout" })).toBeVisible();
  await page.getByRole("button", { name: "Pay" }).click();
  await expect(page).toHaveURL(new RegExp(`/jobs/${job}$`));
}
