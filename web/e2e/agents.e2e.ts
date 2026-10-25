// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("a maker makes a token for their AI agent and can remove it", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/account#ai-agents");
  const section = page.locator("#ai-agents");
  await section.getByRole("textbox", { name: "Name" }).fill("Claude on my laptop");
  await section.getByRole("checkbox", { name: /People jobs too/ }).check();
  await section.getByRole("button", { name: "Make token" }).click();
  await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
  await expect(section.getByText("Never paste this token into a chat with Claude, ChatGPT, Codex, Grok or any AI.")).toBeVisible();
  await expect(section.getByText(/^claude mcp add --transport http --scope user slop .*'Authorization: Bearer \$\{SLOP_TOKEN\}'$/)).toBeVisible();
  await section.getByRole("button", { name: "Done" }).click();
  await expect(section.getByText("Claude on my laptop")).toBeVisible();
  await expect(section.getByText(/People jobs/).last()).toBeVisible();
  await section.getByRole("button", { name: "Remove" }).click();
  await expect(section.getByText("Claude on my laptop")).toHaveCount(0);
});
