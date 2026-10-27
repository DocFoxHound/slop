// SPDX-License-Identifier: AGPL-3.0-or-later
// The line to hand an AI assistant, where people start (the signed-out Publish page, the People
// editor before there's a page), and the guide it points to (public/start.md). Read-only.
import { expect, signIn, test } from "./fixtures";

test("the Publish page offers a line for your AI assistant, pointing at the start guide", async ({ page, baseURL }) => {
  await page.goto("/publish");
  await expect(page.getByRole("link", { name: "Sign in to start" })).toBeVisible();
  const line = page.locator(".ai-prompt code");
  await expect(line).toHaveText(`Read ${new URL(baseURL!).origin}/start.md and help me publish my project on SLAPorSLOP.`);
  await expect(page.getByRole("button", { name: "Copy the line for your AI assistant" })).toBeVisible();
});

test("the People editor offers it before there's a page", async ({ page, baseURL }) => {
  await signIn(page, "player");
  await page.goto("/people/me");
  const intro = page.getByRole("region", { name: "How People pages work" });
  await expect(intro.locator(".ai-prompt code")).toHaveText(
    `Read ${new URL(baseURL!).origin}/start.md and help me set up my People page on SLAPorSLOP.`,
  );
});

test("the start guide is plain Markdown, listed for AI tools", async ({ request }) => {
  const start = await request.get("/start.md");
  expect(start.status()).toBe(200);
  expect(start.headers()["content-type"]).toBe("text/markdown; charset=utf-8");
  const text = await start.text();
  expect(text).toContain("## Starting from nothing (no GitHub yet)");
  expect(text).toContain("gh repo create");
  const llms = await (await request.get("/llms.txt")).text();
  expect(llms).toContain("(/start.md)");
});
