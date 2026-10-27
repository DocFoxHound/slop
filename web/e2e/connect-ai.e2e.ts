// SPDX-License-Identifier: AGPL-3.0-or-later
// Connecting an AI tool by approving it in the browser (web/src/pages/ConnectAi.tsx,
// crates/slop-api/src/agent_oauth.rs): the tool's half (registering, opening the browser,
// swapping the code) is played by the test; the page is the real one.
import { createHash, randomBytes } from "node:crypto";
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { openAccount } from "./accounts-helpers";
import { expect, server, signIn, test } from "./fixtures";

/** A tool registering itself, as Claude Code or Codex would. */
async function registerTool(name: string, redirect: string): Promise<string> {
  const res = await fetch(`${server.url}/v1/oauth/register`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ client_name: name, redirect_uris: [redirect], token_endpoint_auth_method: "none" }),
  });
  expect(res.status).toBe(201);
  return ((await res.json()) as { client_id: string }).client_id;
}

interface Start {
  url: string;
  verifier: string;
  state: string;
}

/** The address the tool opens in the browser. */
function authorizeUrl(clientId: string, redirect: string): Start {
  const verifier = randomBytes(32).toString("base64url");
  const state = randomBytes(12).toString("base64url");
  const q = new URLSearchParams({
    response_type: "code",
    client_id: clientId,
    redirect_uri: redirect,
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    code_challenge_method: "S256",
    state,
    resource: `${server.url}/mcp`,
    scope: "read publish",
  });
  return { url: `/v1/oauth/authorize?${q}`, verifier, state };
}

/** The tool's own listener: records where the browser came back to. */
async function listen(page: Page, prefix: string): Promise<() => URL | undefined> {
  let seen: URL | undefined;
  await page.route(`${prefix}/**`, async (route) => {
    seen = new URL(route.request().url());
    await route.fulfill({ status: 200, contentType: "text/html", body: "<title>Tool</title><p>You can close this tab.</p>" });
  });
  return () => seen;
}

async function tokenFor(clientId: string, code: string, verifier: string, redirect: string) {
  const res = await fetch(`${server.url}/v1/oauth/token`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      code,
      code_verifier: verifier,
      client_id: clientId,
      redirect_uri: redirect,
      resource: `${server.url}/mcp`,
    }),
  });
  expect(res.status).toBe(200);
  return (await res.json()) as { access_token: string; refresh_token: string; scope: string };
}

const listTools = (accessToken: string) =>
  fetch(`${server.url}/mcp`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${accessToken}` },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });

test.describe("approving an AI tool in the browser", () => {
  test("signed in: Approve hands the tool its code, it works, and removing it on AI agents ends it", async ({ page }) => {
    const port = 46_000 + Math.floor(Math.random() * 1000);
    const redirect = `http://127.0.0.1:${port}/callback`;
    const clientId = await registerTool("E2E Code Tool", redirect);
    const start = authorizeUrl(clientId, redirect);
    const callback = await listen(page, `http://127.0.0.1:${port}`);
    await signIn(page, "maker");
    await page.goto(start.url);
    await expect(page).toHaveURL(new RegExp(`^${server.url}/connect-ai/[0-9a-f-]+$`));
    await expect(page.getByRole("heading", { name: "Connect E2E Code Tool to SLAPorSLOP?" })).toBeVisible();
    await expect(page.getByText("That's the name it gives itself.")).toBeVisible();
    const where = page.getByRole("note");
    await expect(where).toContainText("Approving hands it a one-time code on this computer");
    await expect(where).not.toContainText("doesn't know");
    // The choices start from what it asked for, as on the token page.
    await expect(page.getByRole("textbox", { name: "Name it" })).toHaveValue("E2E Code Tool");
    await expect(page.getByRole("combobox", { name: "It can" })).toHaveValue("publish");
    await expect(page.getByRole("combobox", { name: "For how long" }).locator("option")).toHaveText([
      "1 day",
      "7 days",
      "30 days",
      "3 months",
      "6 months",
      "12 months",
    ]);
    const results = await new AxeBuilder({ page }).include("form.connect-ai").withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
    expect(results.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);

    await page.getByRole("textbox", { name: "Name it" }).fill("E2E laptop tool");
    await page.getByRole("combobox", { name: "It can" }).selectOption("read");
    await page.getByRole("combobox", { name: "For how long" }).selectOption({ label: "7 days" });
    await page.getByRole("button", { name: "Approve" }).click();
    await expect(page).toHaveURL(new RegExp(`^http://127\\.0\\.0\\.1:${port}/callback\\?`));
    const back = callback()!;
    expect(back.searchParams.get("state")).toBe(start.state);
    expect(back.searchParams.get("iss")).toBe(server.url);
    await expect(page.getByText(/confirm it's you/i)).toHaveCount(0);

    // The tool swaps the code for its tokens, and MCP works with them.
    const tokens = await tokenFor(clientId, back.searchParams.get("code")!, start.verifier, redirect);
    expect(tokens.scope).toBe("read");
    expect((await listTools(tokens.access_token)).status).toBe(200);

    // Account → AI agents lists it, approved in the browser, and removes it.
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");
    const row = section.getByRole("listitem").filter({ hasText: "E2E laptop tool" });
    await expect(row).toContainText("looks · all apps");
    await expect(row).toContainText("Approved in the browser for E2E Code Tool on your computer");
    await expect(row.getByText("expires in 7 days")).toBeVisible();
    await expect(row.getByRole("button", { name: /Make a replacement/ })).toHaveCount(0);
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
    expect((await listTools(tokens.access_token)).status).toBe(401);
  });

  test("Deny tells the tool no, and nothing is connected", async ({ page }) => {
    const port = 47_000 + Math.floor(Math.random() * 1000);
    const redirect = `http://localhost:${port}/callback`;
    const clientId = await registerTool("E2E Denied Tool", redirect);
    const start = authorizeUrl(clientId, redirect);
    const callback = await listen(page, `http://localhost:${port}`);
    await signIn(page, "maker");
    await page.goto(start.url);
    await page.getByRole("button", { name: "Deny" }).click();
    await expect(page).toHaveURL(new RegExp(`^http://localhost:${port}/callback\\?`));
    const back = callback()!;
    expect(back.searchParams.get("error")).toBe("access_denied");
    expect(back.searchParams.get("state")).toBe(start.state);
    expect(back.searchParams.get("code")).toBeNull();
    await openAccount(page, "agents");
    await expect(page.locator("#ai-agents").getByText("E2E Denied Tool")).toHaveCount(0);
  });

  test("signed out: signing in first comes back to the approval", async ({ page }) => {
    const redirect = "http://127.0.0.1:48123/callback";
    const clientId = await registerTool("E2E Signed-out Tool", redirect);
    await page.goto(authorizeUrl(clientId, redirect).url);
    await expect(page).toHaveURL(new RegExp(`/signin\\?return_to=${encodeURIComponent("/connect-ai/")}[0-9a-f-]+$`));
    const returnTo = new URL(page.url()).searchParams.get("return_to")!;
    await expect(page.getByRole("link", { name: "Continue with GitHub" })).toHaveAttribute(
      "href",
      `/v1/auth/github/start?return_to=${encodeURIComponent(returnTo)}`,
    );
    // Signing in (the test server's GitHub can't, so the session is set) and going on.
    await signIn(page, "maker");
    await page.goto(returnTo);
    await expect(page.getByRole("heading", { name: "Connect E2E Signed-out Tool to SLAPorSLOP?" })).toBeVisible();
  });

  test("a website SLAPorSLOP doesn't know is named and warned about", async ({ page }) => {
    const redirect = "https://tool.example/oauth/callback";
    const clientId = await registerTool("Totally Claude", redirect);
    await page.route("https://tool.example/**", (route) => route.fulfill({ status: 200, body: "tool" }));
    await signIn(page, "maker");
    await page.goto(authorizeUrl(clientId, redirect).url);
    const where = page.getByRole("note");
    await expect(where).toContainText("Approving sends your browser to tool.example.");
    await expect(where).toContainText("SLAPorSLOP doesn't know tool.example.");
    await page.getByRole("button", { name: "Deny" }).click();
    await expect(page).toHaveURL(/^https:\/\/tool\.example\/oauth\/callback\?error=access_denied/);
  });

  test("a tool SLAPorSLOP can't check, or an expired request, says so", async ({ page }) => {
    await signIn(page, "maker");
    await page.goto(authorizeUrl("slopcl_nobody", "http://127.0.0.1:5000/callback").url);
    await expect(page.getByRole("heading", { name: "SLAPorSLOP doesn't know this AI tool" })).toBeVisible();
    await page.goto("/connect-ai/01a00000-0000-7000-8000-000000000000");
    await expect(page.getByRole("heading", { name: "This connection request has expired" })).toBeVisible();
  });

  test("Account → AI agents shows the easy way first, with no token in it", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: server.url });
    await signIn(page, "maker");
    await openAccount(page, "agents");
    const easy = page.getByRole("region", { name: "Connect your AI tool: the easy way" });
    await expect(easy).toBeVisible();
    // Before the token form.
    const easyBox = (await easy.boundingBox())!;
    const formBox = (await page.getByRole("form", { name: "Make a token" }).boundingBox())!;
    expect(easyBox.y).toBeLessThan(formBox.y);
    await expect(easy.getByText(`${server.url}/mcp`, { exact: true })).toBeVisible();
    await easy.getByRole("button", { name: "Copy the command that adds SLAPorSLOP to Claude Code" }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
      `claude mcp add --transport http --scope user slaporslop ${server.url}/mcp`,
    );
    const tools = easy.getByRole("tablist", { name: "Your AI tool" });
    await tools.getByRole("tab", { name: "Codex" }).click();
    await expect(easy.getByText("codex mcp login slaporslop")).toBeVisible();
    await tools.getByRole("tab", { name: "Claude app" }).click();
    await expect(easy.getByText("Add custom connector")).toBeVisible();
    await tools.getByRole("tab", { name: "ChatGPT" }).click();
    await expect(easy.getByText("Add custom MCP server")).toBeVisible();
    expect(await easy.innerText()).not.toMatch(/slopmk_|SLOP_TOKEN|Bearer/);
  });
});
