// SPDX-License-Identifier: AGPL-3.0-or-later
// Account → AI agents, beyond agents.e2e.ts: a read-only token for one app with a short expiry,
// the passkey hint for publishing tokens, and the refusals (a recent sign-in, ten tokens).
// Bo's tokens; each test removes what it made. Making one needs a recent sign-in.
import { call, ok, tag } from "./admin-helpers";
import { expect, signIn, test } from "./fixtures";

const removeAll = async () => {
  for (const t of await ok("maker2", "GET", "/v1/me/maker-tokens")) await ok("maker2", "DELETE", `/v1/me/maker-tokens/${t.id}`);
};

test("a read-only token for one app, for a day", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto("/account#ai-agents");
  const section = page.locator("#ai-agents");
  const name = `Reader ${tag()}`;
  await section.getByRole("textbox", { name: "Name" }).fill(name);
  await section.getByRole("combobox", { name: "Can" }).selectOption({ label: "Look: apps, releases, checks, reviews" });
  const all = section.getByRole("radio", { name: /All my apps, and new ones/ });
  await expect(all).toBeChecked();
  await section.getByRole("checkbox", { name: "Only Pocket Golf" }).check();
  await expect(all).not.toBeChecked();
  await section.getByRole("combobox", { name: "Expires after" }).selectOption({ label: "1 day" });
  await section.getByRole("button", { name: "Make token" }).click();

  await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
  await expect(section.getByText("Never paste this token into a chat with Claude, ChatGPT, Codex, Grok or any AI.")).toBeVisible();
  await expect(section.getByText(/claude mcp add .*slopmk_/)).toHaveCount(0);
  // Looking needs no passkey.
  await expect(section.getByText(/Confirm with a passkey before making a token/)).toHaveCount(0);
  await section.getByRole("button", { name: "Done" }).click();

  const row = section.getByRole("listitem").filter({ hasText: name });
  await expect(row.getByText("looks · pocket-golf")).toBeVisible();
  const tomorrow = new Date(Date.now() + 86_400_000).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
  await expect(row.getByText("expires within a day")).toHaveAttribute("title", tomorrow);
  // It really is limited to Pocket Golf.
  const [made] = (await ok("maker2", "GET", "/v1/me/maker-tokens")).filter((t: { name: string }) => t.name === name);
  expect(made.apps).toEqual(["pocket-golf"]);
  expect(made.scope).toBe("read");
  await row.getByRole("button", { name: "Remove" }).click();
  await expect(row).toHaveCount(0);
});

test("a publishing token without a passkey says what to do, whatever the form says next", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto("/account#ai-agents");
  const section = page.locator("#ai-agents");
  await section.getByRole("textbox", { name: "Name" }).fill(`Publisher ${tag()}`);
  await section.getByRole("button", { name: "Make token" }).click();
  const hint = section.getByText(/Confirm with a passkey before making a token if you want it to publish/);
  await expect(hint).toBeVisible();
  // The hint is about the token just made, not the next one.
  await section.getByRole("combobox", { name: "Can" }).selectOption({ label: "Look: apps, releases, checks, reviews" });
  await expect(hint).toBeVisible();
  await section.getByRole("button", { name: "Done" }).click();
  await expect(section.getByText(/publishes · all apps/)).toBeVisible();
  await removeAll();
});

test("the refusals: ten tokens at most, and a recent sign-in", async ({ page }) => {
  await removeAll();
  for (let i = 0; i < 10; i++) await ok("maker2", "POST", "/v1/me/maker-tokens", { name: `Agent ${i}`, scope: "read", apps: [], people: false, days: 1 });
  await signIn(page, "maker2");
  await page.goto("/account#ai-agents");
  const section = page.locator("#ai-agents");
  await section.getByRole("textbox", { name: "Name" }).fill("The eleventh");
  await section.getByRole("button", { name: "Make token" }).click();
  await expect(section.getByRole("alert")).toHaveText("you have 10 agent tokens; remove one first");
  await expect(section.getByText("Copy it now")).toHaveCount(0);
  await removeAll();

  // An old sign-in is asked to sign in again (the seeded one is still recent: the answer is faked).
  await page.route("**/v1/me/maker-tokens", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 403,
          json: { error: { code: "step_up_required", message: "confirm it's you first: sign in again, or use a passkey" } },
        })
      : route.fallback(),
  );
  await section.getByRole("button", { name: "Make token" }).click();
  await expect(section.getByRole("alert")).toHaveText("confirm it's you first: sign in again, or use a passkey");
  expect((await call("maker2", "GET", "/v1/me/maker-tokens")).json).toEqual([]);
});
