// SPDX-License-Identifier: AGPL-3.0-or-later
// The page bar under the top navigation (web/src/components/BrowserBar.tsx): Back and Forward
// through the history (off when there's nowhere to go), Reload this page without reloading the
// app, and the page's address. It's for the desktop app above all, where there's no browser
// around SLAPorSLOP: there it shows from 601px and answers the browser's shortcuts; on the website only
// on wide screens; never on phones.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

import { asDesktop } from "./desktop-helpers";
import { expect, server, test } from "./fixtures";

const bar = (page: Page) => page.getByRole("group", { name: "Page" });
const host = new URL(server.url).host;

/** Requests for the store page's app, to see a reload ask again. */
function countAppLoads(page: Page): () => number {
  let n = 0;
  page.on("request", (r) => {
    if (r.method() === "GET" && new URL(r.url()).pathname === "/v1/apps/lantern-keep") n++;
  });
  return () => n;
}

/** Set on the page: a full page load would lose it. */
async function mark(page: Page) {
  await page.evaluate(() => ((window as unknown as { __stillHere: boolean }).__stillHere = true));
}
async function stillHere(page: Page): Promise<boolean> {
  return page.evaluate(() => (window as unknown as { __stillHere?: boolean }).__stillHere === true);
}

test("Back and Forward go through the history, and are off when there's nowhere to go", async ({ page }) => {
  await page.goto("/");
  const back = bar(page).getByRole("button", { name: "Back" });
  const forward = bar(page).getByRole("button", { name: "Forward" });
  await expect(bar(page)).toBeVisible();
  await expect(back).toBeDisabled();
  await expect(forward).toBeDisabled();
  await expect(bar(page).getByText(`${host}/`, { exact: true })).toBeVisible();

  await page.locator('a[href="/app/lantern-keep"]').first().click();
  await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();
  await expect(bar(page).getByText(`${host}/app/lantern-keep`, { exact: true })).toBeVisible();
  await expect(back).toBeEnabled();
  await expect(forward).toBeDisabled();
  await mark(page);

  await back.click();
  await expect(page).toHaveURL(`${server.url}/`);
  await expect(back).toBeDisabled();
  await expect(forward).toBeEnabled();
  await forward.click();
  await expect(page).toHaveURL(`${server.url}/app/lantern-keep`);
  await expect(forward).toBeDisabled();
  // A new page drops what was ahead, as in a browser.
  await back.click();
  await expect(forward).toBeEnabled();
  await page.locator('a[href="/library"]').first().click();
  await expect(page).toHaveURL(`${server.url}/library`);
  await expect(forward).toBeDisabled();
  await expect(back).toBeEnabled();
  // All without loading the page again.
  expect(await stillHere(page)).toBe(true);
  // The browser's own Back and Forward agree.
  await page.goBack();
  await expect(forward).toBeEnabled();
  await page.goForward();
  await expect(forward).toBeDisabled();
});

test("Reload this page loads its data again without reloading the app", async ({ page }) => {
  const loads = countAppLoads(page);
  await page.goto("/app/lantern-keep");
  await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();
  await expect.poll(loads).toBeGreaterThan(0);
  const before = loads();
  await mark(page);
  await bar(page).getByRole("button", { name: "Reload this page" }).click();
  await expect.poll(loads).toBeGreaterThan(before);
  await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();
  expect(await stillHere(page)).toBe(true);
});

test("the address shows the page with its search, and Copy link copies it", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: server.url });
  await page.goto("/search?q=castle");
  const address = bar(page).locator(".bb-text");
  await expect(address).toHaveText(`${host}/search?q=castle`);
  await expect(address).toHaveAttribute("title", `${server.url}/search?q=castle`);
  // One click selects all of it, to copy by hand.
  await expect(address).toHaveCSS("user-select", "all");
  await bar(page).getByRole("button", { name: "Copy link" }).click();
  await expect(bar(page).locator(".bb-said")).toHaveText("Link copied");
  // Pages' own status messages stay the only ones.
  await expect(bar(page).getByRole("status")).toHaveCount(0);
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`${server.url}/search?q=castle`);

  // Its buttons are labelled, with tooltips; nothing serious for axe.
  for (const name of ["Back", "Forward", "Reload this page", "Copy link"]) {
    await expect(bar(page).getByRole("button", { name, exact: true })).toHaveAttribute("title", new RegExp(`^${name}`));
  }
  const results = await new AxeBuilder({ page }).include(".browserbar").analyze();
  expect(results.violations.map((v) => v.id)).toEqual([]);
});

test("on the website it shows only on wide screens; never on phones", async ({ page }) => {
  for (const [width, shown] of [
    [1280, true],
    [1100, true],
    [900, false],
    [390, false],
  ] as const) {
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    if (shown) await expect(bar(page), `at ${width}px`).toBeVisible();
    else await expect(bar(page), `at ${width}px`).toBeHidden();
  }
});

test.describe("in the desktop app", () => {
  test.beforeEach(async ({ page }) => {
    await asDesktop(page, { state: "none" });
  });

  test("it shows from 601px, and the window is dragged by its empty space, not its controls", async ({ page }) => {
    for (const [width, shown] of [
      [900, true],
      [620, true],
      [390, false],
    ] as const) {
      await page.setViewportSize({ width, height: 700 });
      await page.goto("/");
      if (shown) await expect(bar(page), `at ${width}px`).toBeVisible();
      else await expect(bar(page), `at ${width}px`).toBeHidden();
    }
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/");
    // Tauri drags by the element pressed, not its parents.
    await expect(page.locator(".browserbar")).toHaveAttribute("data-tauri-drag-region");
    await expect(page.locator(".browserbar .grow")).toHaveAttribute("data-tauri-drag-region");
    for (const control of await bar(page).getByRole("button").all()) await expect(control).not.toHaveAttribute("data-tauri-drag-region");
    await expect(bar(page).locator(".bb-text")).not.toHaveAttribute("data-tauri-drag-region");
    // The top bar still is the title bar.
    await expect(page.getByRole("navigation", { name: "Main" }).first()).toHaveAttribute("data-tauri-drag-region");
    await expect(bar(page).getByRole("button", { name: "Back" })).toHaveAttribute("title", "Back (Alt+Left)");
  });

  test("Alt+Left and Alt+Right go back and forward; F5 and Ctrl+R reload the page, not the app", async ({ page }) => {
    const loads = countAppLoads(page);
    await page.goto("/");
    await page.locator('a[href="/app/lantern-keep"]').first().click();
    await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();
    await mark(page);
    await page.keyboard.press("Alt+ArrowLeft");
    await expect(page).toHaveURL(`${server.url}/`);
    await page.keyboard.press("Alt+ArrowRight");
    await expect(page).toHaveURL(`${server.url}/app/lantern-keep`);
    // At the start there's nowhere further back: SLAPorSLOP stays.
    await page.keyboard.press("Alt+ArrowLeft");
    await page.keyboard.press("Alt+ArrowLeft");
    await expect(page).toHaveURL(`${server.url}/`);
    await page.keyboard.press("Alt+ArrowRight");
    await expect(page.getByRole("heading", { name: "Lantern Keep", level: 1 })).toBeVisible();

    const before = loads();
    await page.keyboard.press("F5");
    await expect.poll(loads).toBeGreaterThan(before);
    const after = loads();
    await page.keyboard.press("Control+r");
    await expect.poll(loads).toBeGreaterThan(after);
    expect(await stillHere(page)).toBe(true);

    // Typing in a field, Alt+Left is left to the field.
    await page.getByRole("searchbox", { name: "Search the store" }).fill("lantern");
    await page.keyboard.press("Alt+ArrowLeft");
    await expect(page).toHaveURL(`${server.url}/app/lantern-keep`);
  });
});
