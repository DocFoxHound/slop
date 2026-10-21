// What the tests share: the test server's address and accounts.
import { test as base, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { STATE } from "./setup";

export interface Server {
  url: string;
  fake: string;
  cookie_name: string;
  cookies: Record<"maker" | "player" | "admin" | "overdue", string>;
}

export const server: Server = JSON.parse(readFileSync(STATE, "utf8"));

/** Signs the page's browser in as one of the seeded accounts. */
export async function signIn(page: Page, who: keyof Server["cookies"]) {
  await page.context().addCookies([
    { name: server.cookie_name, value: server.cookies[who], url: server.url, httpOnly: true, sameSite: "Lax" },
  ]);
}

export const test = base.extend({
  baseURL: async ({}, use) => use(server.url),
});
export { expect } from "@playwright/test";
