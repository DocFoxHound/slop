// SPDX-License-Identifier: AGPL-3.0-or-later
// App icons (`icon` in .slop/media.json, crates/slop-api/src/media.rs; components/AppIcon.tsx and
// IconSlot.tsx): on store tiles and the app page, the icon, else a square from the thumbnail,
// else the letter; the Edit store page's Icon slot in each state, with its crop helper; and the
// new-app form's pointer to them.
//
// Seeded (examples/e2e_server): Lantern Keep has an approved icon and thumbnail, Neon Drift only
// an icon, Castle Rock nothing. The fake GitHub's media.json names an icon for every repository,
// none when its name has "noicon" in it, and one that isn't square when it has "badicon".
import { expect, signIn, test } from "./fixtures";
import { tweak } from "./accounts-helpers";
import { githubRepo, newApp, ok, tag } from "./admin-helpers";

const tileIcon = (page: import("@playwright/test").Page, slug: string) => page.locator(`a.tile[href="/app/${slug}"] .app-icon`).first();

test("store tiles and app pages show the icon, else a crop of the thumbnail, else the letter", async ({ page }) => {
  await page.goto("/search");
  await expect(tileIcon(page, "lantern-keep")).toHaveAttribute("data-kind", "icon");
  await expect(tileIcon(page, "lantern-keep").locator("img")).toHaveAttribute("src", /\/raw\/e2e\/lantern-keep\/[0-9a-f]+\/\.slop\/icon\.png$/);
  await expect(tileIcon(page, "neon-drift")).toHaveAttribute("data-kind", "icon");
  await expect(tileIcon(page, "castle-rock")).toHaveAttribute("data-kind", "letter");
  await expect(tileIcon(page, "castle-rock")).toHaveText("C");
  // Tiles name the app beside its icon.
  await expect(page.locator('a.tile[href="/app/lantern-keep"] .tile-name')).toHaveText("Lantern Keep");
  // The icon really loads, and is square.
  await tileIcon(page, "lantern-keep").scrollIntoViewIfNeeded();
  await expect
    .poll(() => tileIcon(page, "lantern-keep").locator("img").evaluate((img: HTMLImageElement) => [img.naturalWidth, img.naturalHeight]))
    .toEqual([512, 512]);

  await page.goto("/app/lantern-keep");
  const head = page.locator(".app-head .app-head-icon");
  await expect(head).toHaveAttribute("data-kind", "icon");
  await expect(head).toHaveCSS("width", "48px");
  await page.goto("/app/castle-rock");
  await expect(page.locator(".app-head .app-head-icon")).toHaveAttribute("data-kind", "letter");

  // Without an approved icon, the thumbnail's middle (CSS crops it to a square).
  await tweak(page, "/v1/apps/lantern-keep", (app) => ({ ...app, icon_url: null }));
  await page.goto("/app/lantern-keep");
  await expect(head).toHaveAttribute("data-kind", "crop");
  await expect(head.locator("img")).toHaveAttribute("src", /\.slop\/thumb\.png$/);
  await expect(head.locator("img")).toHaveCSS("object-fit", "cover");
  // A picture that doesn't load falls back too.
  await page.unroute((url) => url.pathname === "/v1/apps/lantern-keep");
  await page.route(/\/raw\/e2e\/lantern-keep\/.*\/icon\.png$/, (route) => route.fulfill({ status: 404 }));
  await page.goto("/app/lantern-keep");
  await expect(head).toHaveAttribute("data-kind", "crop");
});

test("the Edit store page's Icon slot: missing, waiting, problem, in the store, and a crop to download", async ({ page }) => {
  test.setTimeout(120_000);
  await signIn(page, "maker");
  const t = tag();
  const noIcon = { slug: `ico-noicon-${t}`, title: `Plain Pier ${t}` };
  const bad = { slug: `ico-badicon-${t}`, title: `Wonky Wharf ${t}` };
  const good = { slug: `ico-good-${t}`, title: `Bright Buoy ${t}` };
  for (const a of [noIcon, bad, good]) await newApp({ ...a, stage: "coming_soon", repo: githubRepo(a.slug) }, "maker");

  // Nothing read yet: no icon, gently; the line to add and where.
  await page.goto(`/publish/${good.slug}`);
  const slot = page.locator("section.icon-slot");
  await expect(slot.getByRole("heading", { name: "Icon", level: 3 })).toBeVisible();
  await expect(slot.getByText("NO ICON YET", { exact: true })).toBeVisible();
  await expect(slot.getByText(/No icon yet\. That's fine for now: it never holds anything up\./)).toBeVisible();
  await expect(slot.getByText(/players see your app's first letter/)).toBeVisible();
  await expect(slot.locator("pre")).toHaveText('"icon": "icon.png",');
  await expect(slot.getByRole("link", { name: "Pictures and videos" })).toHaveAttribute("href", "/publish/guide#pictures-and-videos");
  await expect(slot.locator(".icon-previews .app-icon")).toHaveCount(3);
  await expect(slot.locator(".icon-previews .app-icon").first()).toHaveAttribute("data-kind", "letter");
  // Nothing to crop from yet.
  await expect(slot.getByRole("button", { name: /Use a crop/ })).toHaveCount(0);

  // Read: the icon waits for review with the other pictures, shown at 32, 64 and 128 pixels.
  await page.locator("section.media-editor").getByRole("button", { name: "Check my repository now" }).click();
  await expect(slot.getByText("IN REVIEW", { exact: true })).toBeVisible({ timeout: 45_000 });
  await expect(slot.getByText(/icon\.png, 512×512, is waiting for review/)).toBeVisible();
  for (const [i, px] of [32, 64, 128].entries()) {
    await expect(slot.locator(".icon-previews .app-icon").nth(i)).toHaveAttribute("data-kind", "icon");
    await expect(slot.locator(".icon-previews .app-icon").nth(i)).toHaveCSS("width", `${px}px`);
  }

  // Approved: in the store, and on the app's page.
  const overview = await ok("maker", "GET", `/v1/apps/${good.slug}/media`);
  await ok("admin", "POST", `/v1/admin/media/${overview.pending.id}/approve`);
  await page.reload();
  await expect(slot.getByText("APPROVED", { exact: true })).toBeVisible();
  await expect(slot.getByText(/in players' libraries, on store tiles and on your app's page/)).toBeVisible();
  await expect(slot.locator("pre")).toHaveCount(0);
  await page.goto(`/app/${good.slug}`);
  await expect(page.locator(".app-head .app-head-icon")).toHaveAttribute("data-kind", "icon");

  // An icon that isn't square: the reason, on the icon.
  await page.goto(`/publish/${bad.slug}`);
  await page.locator("section.media-editor").getByRole("button", { name: "Check my repository now" }).click();
  await expect(slot.getByText("PROBLEM", { exact: true })).toBeVisible({ timeout: 45_000 });
  await expect(slot.getByText(/The icon couldn't be used: icon-wide\.png is 600×300; the icon has to be square/)).toBeVisible();

  // No icon in media.json, but a thumbnail: players see its middle; a crop can be downloaded to commit.
  await page.goto(`/publish/${noIcon.slug}`);
  await page.locator("section.media-editor").getByRole("button", { name: "Check my repository now" }).click();
  await expect(page.getByText("Found new pictures. They're waiting for review.")).toBeVisible({ timeout: 45_000 });
  await expect(slot.getByText("NO ICON YET", { exact: true })).toBeVisible();
  const noIconOverview = await ok("maker", "GET", `/v1/apps/${noIcon.slug}/media`);
  expect(noIconOverview.recommendations[0].kind).toBe("icon:missing");
  await ok("admin", "POST", `/v1/admin/media/${noIconOverview.pending.id}/approve`);
  await page.reload();
  await expect(slot.getByText(/players see a square from the middle of your thumbnail/)).toBeVisible();
  await expect(slot.locator(".icon-previews .app-icon").first()).toHaveAttribute("data-kind", "crop");
  await slot.getByRole("button", { name: "Use a crop of one of my pictures as the icon" }).click();
  await expect(slot.getByText(/It isn't stored on SLAPorSLOP/)).toBeVisible();
  await expect(slot.getByRole("combobox", { name: "Picture" })).toHaveValue("0");
  await expect(slot.getByRole("img", { name: `Crop for ${noIcon.slug}'s icon` })).toBeVisible();
  // The thumbnail is 920×430: a 430-pixel square, slid along it.
  await slot.getByRole("slider", { name: /Where to cut/ }).fill("0");
  const download = page.waitForEvent("download");
  await slot.getByRole("button", { name: "Download icon.png (430×430)" }).click();
  expect((await download).suggestedFilename()).toBe("icon.png");
  // A 1280×720 screenshot gives the best size.
  await slot.getByRole("combobox", { name: "Picture" }).selectOption({ label: "Screenshot 1 (shots/one.png)" });
  await expect(slot.getByRole("button", { name: "Download icon.png (512×512)" })).toBeEnabled();
});

test("the new-app form says where the icon and thumbnail come from", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/publish/new");
  const hint = page.locator(".new-app-pictures");
  await expect(hint).toContainText("your app's icon, thumbnail, screenshots and videos live in your repository");
  await expect(hint.locator("code")).toHaveText(".slop/media.json");
  await expect(hint.getByRole("link", { name: "Pictures and videos" })).toHaveAttribute("href", "/publish/guide#pictures-and-videos");
  await hint.getByRole("link", { name: "Pictures and videos" }).click();
  await expect(page.locator("#pictures-and-videos")).toBeVisible();
});
