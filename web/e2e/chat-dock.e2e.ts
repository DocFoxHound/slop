// SPDX-License-Identifier: AGPL-3.0-or-later
// Friends & Chat (web/src/components/ChatDock.tsx): the bar in the bottom right and its panel,
// on the website (friends, and where chat is) and in a pretend desktop app (e2e/desktop-helpers.ts),
// where chat sets itself up the first time it's opened (pages/Chat.tsx). Read-only on the server.
import type { Page } from "@playwright/test";

import { call } from "./accounts-helpers";
import { asDesktop, chatCalls } from "./desktop-helpers";
import { expect, signIn, test } from "./fixtures";

const chatBar = (page: Page) => page.getByRole("button", { name: /^Friends & Chat/ });
const panel = (page: Page) => page.getByRole("dialog", { name: "Friends & Chat" });

test.describe("on the website", () => {
  test("signed out there's no bar", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("navigation", { name: "Store" })).toBeVisible();
    await expect(chatBar(page)).toHaveCount(0);
  });

  test("the panel has your friends and says chat is in the app", async ({ page }) => {
    const friends = await call<{ friends: Array<{ name: string }>; incoming: Array<{ name: string }> }>("player", "GET", "/v1/me/friends");
    await signIn(page, "player");
    await page.goto("/");
    const bar = chatBar(page);
    await expect(bar).toHaveAttribute("aria-expanded", "false");
    await bar.click();
    await expect(bar).toHaveAttribute("aria-expanded", "true");
    const p = panel(page);
    await expect(p).toBeVisible();
    await expect(p).toBeFocused();
    await expect(p.getByText("Chat is in the Slop app.")).toBeVisible();
    await expect(p.getByRole("link", { name: /Get Slop for/ })).toHaveAttribute("href", "/download");
    await expect(p.getByRole("heading", { name: `Your friends (${friends.friends.length})` })).toBeVisible();
    for (const f of friends.friends) await expect(p.getByRole("link", { name: f.name })).toBeVisible();
    for (const f of friends.incoming) {
      await expect(p.getByRole("listitem").filter({ hasText: f.name }).getByRole("button", { name: "Accept" })).toBeVisible();
    }
    // No messaging on the website.
    await expect(p.getByRole("button", { name: /Message/ })).toHaveCount(0);
    await expect(p.getByRole("link", { name: "Add or manage friends" })).toHaveAttribute("href", "/friends");
    await expect(p.getByRole("link", { name: "Full page ↗" })).toHaveAttribute("href", "/friends");
    // It fits in the window.
    const box = (await p.boundingBox())!;
    const view = page.viewportSize()!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(view.width);
    expect(box.width).toBeLessThanOrEqual(380);
  });

  test("it stays open while you browse, until Escape or the bar closes it", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/");
    await chatBar(page).click();
    await expect(panel(page)).toBeVisible();
    await page.getByRole("navigation", { name: "Store" }).getByRole("link", { name: "Games", exact: true }).click();
    await expect(page).toHaveURL(/\/browse\/games$/);
    await expect(panel(page)).toBeVisible();
    await page.getByRole("link", { name: "LIBRARY" }).click();
    await expect(panel(page)).toBeVisible();

    await panel(page).getByRole("link", { name: "Add or manage friends" }).focus();
    await page.keyboard.press("Escape");
    await expect(panel(page)).toHaveCount(0);
    await expect(chatBar(page)).toBeFocused();
    await expect(chatBar(page)).toHaveAttribute("aria-expanded", "false");

    await page.keyboard.press("Enter");
    await expect(panel(page)).toBeVisible();
    await chatBar(page).click();
    await expect(panel(page)).toHaveCount(0);

    await chatBar(page).click();
    await panel(page).getByRole("button", { name: "Close Friends & Chat" }).click();
    await expect(panel(page)).toHaveCount(0);
    await expect(chatBar(page)).toBeFocused();
  });

  test("the chat and friends pages still work on their own; the chat page has no bar", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/chat");
    await expect(page.getByText("Chat is in the Slop app.")).toBeVisible();
    await expect(chatBar(page)).toHaveCount(0);
    await page.goto("/friends");
    await expect(page.getByRole("heading", { name: "🤝 Friends", level: 1 })).toBeVisible();
    await expect(chatBar(page)).toBeVisible();
  });

  test("the bar doesn't hide the end of a page", async ({ page }) => {
    await signIn(page, "player");
    await page.goto("/");
    await page.locator("main").evaluate((m) => m.scrollTo({ top: m.scrollHeight }));
    const contact = page.getByRole("navigation", { name: "About Slop" }).getByRole("link", { name: "Contact" });
    const link = (await contact.boundingBox())!;
    const bar = (await chatBar(page).boundingBox())!;
    expect(link.y + link.height).toBeLessThanOrEqual(bar.y);
  });
});

test.describe("in the desktop app", () => {
  test("opening chat the first time sets it up, named after the computer, with the recovery code once", async ({ page }) => {
    await asDesktop(page, { state: "none", computerName: "brians-pc" });
    await signIn(page, "player");
    await page.goto("/");
    // Nothing is set up before chat is opened.
    await expect(page.getByRole("link", { name: "Pat Player" })).toBeVisible();
    expect((await chatCalls(page)).filter((a) => a.action === "setup")).toEqual([]);

    await chatBar(page).click();
    const p = panel(page);
    await expect(p.getByRole("tab", { name: "Chats" })).toHaveAttribute("aria-selected", "true");
    await expect(p.getByRole("heading", { name: "Save your recovery code" })).toBeVisible();
    await expect(p.getByLabel("Recovery code", { exact: true })).toHaveText("RCVR-CODE-1234-5678-ABCD-EFGH");
    // No questions: no device name, no chat name.
    await expect(p.getByRole("textbox")).toHaveCount(0);
    await expect(page.getByText("Name this device")).toHaveCount(0);
    const calls = await chatCalls(page);
    expect(calls.filter((a) => a.action === "setup")).toEqual([{ action: "setup" }]);
    expect(calls.filter((a) => a.action === "set_backups")).toEqual([{ action: "set_backups", on: true }]);

    // Closing the panel doesn't lose the code: it's there until it's saved.
    await p.getByRole("button", { name: "Close Friends & Chat" }).click();
    await chatBar(page).click();
    await expect(p.getByLabel("Recovery code", { exact: true })).toBeVisible();
    await expect(p.getByRole("button", { name: "Start chatting" })).toBeDisabled();
    await p.getByRole("checkbox", { name: "I've saved my recovery code" }).check();
    await p.getByRole("button", { name: "Start chatting" }).click();
    await expect(p.getByText("No chats yet.")).toBeVisible();
    await expect(p.getByRole("button", { name: "Devices and backups" })).toBeVisible();
    // Set up once, however often it was opened.
    expect((await chatCalls(page)).filter((a) => a.action === "setup")).toHaveLength(1);

    // Friends in the app can be messaged.
    await p.getByRole("tab", { name: "Friends" }).click();
    await expect(p.getByRole("button", { name: /^Message / }).first()).toBeVisible();
  });

  test("the chat page sets chat up by itself too", async ({ page }) => {
    await asDesktop(page, { state: "none" });
    await signIn(page, "player");
    await page.goto("/chat");
    await expect(page.getByRole("heading", { name: "Save your recovery code", level: 1 })).toBeVisible();
    await expect(page.getByRole("textbox")).toHaveCount(0);
    expect((await chatCalls(page)).filter((a) => a.action === "setup")).toEqual([{ action: "setup" }]);
    // The page is the full version of the panel: no bar on it.
    await expect(chatBar(page)).toHaveCount(0);
  });

  test("another computer waits for approval from one already chatting, with no recovery code", async ({ page }) => {
    await asDesktop(page, { state: "none", setupBecomes: "pending" });
    await signIn(page, "player");
    await page.goto("/");
    await chatBar(page).click();
    const p = panel(page);
    await expect(p.getByRole("heading", { name: "Approve this device" })).toBeVisible();
    await expect(p.getByLabel("This device's code")).toHaveText("ABC-DEF-GHI");
    const calls = await chatCalls(page);
    expect(calls.filter((a) => a.action === "setup")).toHaveLength(1);
    expect(calls.filter((a) => a.action === "set_backups")).toEqual([]);
  });

  test("a removed device is set up again only when asked, in one click", async ({ page }) => {
    await asDesktop(page, { state: "revoked" });
    await signIn(page, "player");
    await page.goto("/");
    await chatBar(page).click();
    const p = panel(page);
    await expect(p.getByText("This device was removed from chat.")).toBeVisible();
    await expect(p.getByRole("textbox")).toHaveCount(0);
    expect((await chatCalls(page)).filter((a) => a.action === "setup")).toEqual([]);
    await p.getByRole("button", { name: "Set up chat again" }).click();
    await expect(p.getByRole("heading", { name: "Save your recovery code" })).toBeVisible();
    expect((await chatCalls(page)).filter((a) => a.action === "setup")).toEqual([{ action: "setup" }]);
  });

  test("a device already chatting just shows the chats", async ({ page }) => {
    await asDesktop(page, { state: "active" });
    await signIn(page, "player");
    await page.goto("/");
    await chatBar(page).click();
    await expect(panel(page).getByText("No chats yet.")).toBeVisible();
    const calls = await chatCalls(page);
    expect(calls.filter((a) => a.action === "setup" || a.action === "set_backups")).toEqual([]);
  });

  test("a chat opens inside the panel, with the way back and the way to the full page", async ({ page }) => {
    const id = "0190a000-0000-7000-8000-0000000000c1";
    await asDesktop(page, { state: "active", conversations: [[id, "Bo Builder"]] });
    await signIn(page, "player");
    await page.goto("/");
    await chatBar(page).click();
    const p = panel(page);
    await p.getByRole("link", { name: /Bo Builder/ }).click();
    // Still the store behind it.
    await expect(page).toHaveURL(/\/$/);
    await expect(p.getByRole("heading", { name: "Bo Builder" })).toBeVisible();
    await expect(p.getByRole("textbox", { name: "Message Bo Builder" })).toBeVisible();
    await expect(p.getByRole("link", { name: "Open full chat ↗" })).toHaveAttribute("href", `/chat/${id}`);
    await expect(p.getByRole("link", { name: "Full page ↗" })).toHaveAttribute("href", `/chat/${id}`);
    // Escape in the message box closes the panel when there's no reply to cancel; the chat is
    // still open when it comes back.
    await p.getByRole("textbox", { name: "Message Bo Builder" }).press("Escape");
    await expect(panel(page)).toHaveCount(0);
    await chatBar(page).click();
    await expect(p.getByRole("textbox", { name: "Message Bo Builder" })).toBeVisible();
    await p.getByRole("button", { name: "‹ Chats" }).click();
    await expect(p.getByRole("link", { name: /Bo Builder/ })).toBeVisible();

    // Messaging a friend opens their chat right there.
    await p.getByRole("tab", { name: "Friends" }).click();
    await p.getByRole("button", { name: /^Message / }).first().click();
    await expect(p.getByRole("tab", { name: "Chats" })).toHaveAttribute("aria-selected", "true");
    await expect(p.getByRole("textbox", { name: "Message Bo Builder" })).toBeVisible();

    // The full chat page takes over from the panel.
    await p.getByRole("link", { name: "Open full chat ↗" }).click();
    await expect(page).toHaveURL(new RegExp(`/chat/${id}$`));
    await expect(panel(page)).toHaveCount(0);
    await expect(page.getByRole("textbox", { name: "Message Bo Builder" })).toBeVisible();
  });
});
