// SPDX-License-Identifier: AGPL-3.0-or-later
import { expect, signIn, test } from "./fixtures";

test("staff read, filter, answer and close what AI agents wrote to the admins", async ({ browser }) => {
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  await admin.getByRole("tab", { name: "AI messages" }).click();

  // Both seeded threads, unread.
  await expect(admin.getByRole("link", { name: "Release upload times out" })).toBeVisible();
  await expect(admin.getByRole("link", { name: "Show scan progress" })).toBeVisible();
  await expect(admin.getByText("UNREAD", { exact: true })).toHaveCount(2);

  // Agents see staff by role unless they choose to show their name.
  const listed = admin.getByRole("checkbox", { name: /Show my name to AI agents/ });
  await expect(listed).not.toBeChecked();
  await listed.click();
  await expect(listed).toBeChecked();
  await listed.click();
  await expect(listed).not.toBeChecked();

  // Filter by category.
  const category = admin.getByRole("combobox", { name: "Category" });
  await category.selectOption({ label: "Suggestions" });
  await expect(admin.getByRole("link", { name: "Show scan progress" })).toBeVisible();
  await expect(admin.getByRole("link", { name: "Release upload times out" })).toHaveCount(0);
  await category.selectOption({ label: "All" });

  // Open one: who sent it, and the message as plain text (no markup, no links).
  await admin.getByRole("link", { name: "Release upload times out" }).click();
  await expect(admin.getByRole("heading", { name: "Release upload times out", level: 2 })).toBeVisible();
  await expect(admin.getByRole("link", { name: "Mia Maker" })).toHaveAttribute("href", /tab=users&user=/);
  await expect(admin.getByText("<b>not bold</b>", { exact: false })).toBeVisible();
  await expect(admin.getByRole("link", { name: /example\.com/ })).toHaveCount(0);
  await expect(admin.getByText('Sent with maker token "Claude Code" (read scope, created 12 days ago)')).toBeVisible();

  // Answer, then close it.
  await admin.getByRole("textbox", { name: /Your reply/ }).fill("Thanks: uploads get 2 minutes from today.");
  await admin.getByRole("button", { name: "Send reply" }).click();
  await expect(admin.getByText("Thanks: uploads get 2 minutes from today.")).toBeVisible();
  await admin.getByRole("button", { name: "Close thread" }).click();
  await expect(admin.getByText("CLOSED", { exact: true })).toBeVisible();

  // Gone from Open (and read), there under Closed.
  await admin.getByRole("link", { name: "← All messages" }).click();
  await expect(admin.getByRole("link", { name: "Show scan progress" })).toBeVisible();
  await expect(admin.getByRole("link", { name: "Release upload times out" })).toHaveCount(0);
  await admin.getByRole("button", { name: "Closed", exact: true }).click();
  await expect(admin.getByRole("link", { name: "Release upload times out" })).toBeVisible();
  await expect(admin.getByText("UNREAD", { exact: true })).toHaveCount(0);

  // Deleting asks first, in the page.
  await admin.getByRole("button", { name: "Open", exact: true }).click();
  await admin.getByRole("link", { name: "Show scan progress" }).click();
  await admin.getByRole("button", { name: "Delete thread" }).click();
  await admin.getByRole("button", { name: "Cancel" }).click();
  await expect(admin.getByRole("heading", { name: "Show scan progress", level: 2 })).toBeVisible();
  await admin.getByRole("button", { name: "Delete thread" }).click();
  await admin.getByRole("button", { name: "Delete for good" }).click();
  await expect(admin.getByText("The thread is deleted.")).toBeVisible();

  // The maker is told about the answer.
  const maker = await browser.newPage();
  await signIn(maker, "maker");
  await maker.goto("/notifications");
  await expect(maker.getByText(/Slop's admins answered your AI agent: Release upload times out/)).toBeVisible();
});
