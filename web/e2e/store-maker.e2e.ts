// SPDX-License-Identifier: AGPL-3.0-or-later
// What makers do for their store pages: pictures from the repository (and staff approving or
// turning them down), the browser demo panel, and registering a branch or mod of someone
// else's app.
import { expect, ids, signIn, test } from "./fixtures";
import { draftApp, failApi, path, tryCall } from "./store-helpers";

// Remy's drafts on GitHub (the test server reads their .slop/media.json): one whose pictures
// staff approve, one whose pictures they turn down.
const pond = "pictures-pond";
const puddle = "pictures-puddle";

test.beforeAll(async () => {
  await draftApp(pond, "Pictures Pond", { who: "pendingCreative", github: true });
  await draftApp(puddle, "Pictures Puddle", { who: "pendingCreative", github: true });
});

test("a maker reads pictures from the repository, orders them, and staff approve them", async ({ browser }) => {
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/publish/${pond}`);
  const editor = maker.locator(".media-editor");
  await expect(editor.getByRole("heading", { name: "Pictures & videos" })).toBeVisible();
  await expect(editor.getByText("No pictures in the store yet.")).toBeVisible();
  await editor.getByRole("button", { name: "Check my repository now" }).click();
  await expect(editor.getByText("Found new pictures. They're waiting for review.")).toBeVisible({ timeout: 20_000 });
  const waiting = editor.locator(".media-set").filter({ hasText: "WAITING FOR REVIEW" });
  const items = waiting.locator(".gallery-edit li b");
  await expect(items).toHaveText(["shots/one.png", "shots/two.png", "YouTube video"]);
  await expect(waiting.getByRole("img", { name: "Thumbnail" })).toBeVisible();

  // Order: the video first. It's saved straight away (and isn't reviewed).
  await expect(waiting.getByRole("button", { name: "Move item 1 earlier" })).toBeDisabled();
  await expect(waiting.getByRole("button", { name: "Move item 3 later" })).toBeDisabled();
  await waiting.getByRole("button", { name: "Move item 3 earlier" }).click();
  await expect(items).toHaveText(["shots/one.png", "YouTube video", "shots/two.png"]);
  await waiting.getByRole("button", { name: "Move item 2 earlier" }).click();
  await expect(items).toHaveText(["YouTube video", "shots/one.png", "shots/two.png"]);
  await maker.reload();
  await expect(items).toHaveText(["YouTube video", "shots/one.png", "shots/two.png"]);

  // Nothing new in the repository: it says so.
  await editor.getByRole("button", { name: "Check my repository now" }).click();
  await expect(editor.getByText("Nothing new: these pictures are already shown or waiting for review.")).toBeVisible({ timeout: 20_000 });

  // Staff look at them in the queue and approve.
  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  await admin.getByRole("link", { name: /Pictures to review$/ }).click();
  const row = admin.locator("li").filter({ has: admin.getByRole("link", { name: "Pictures Pond" }) });
  await expect(row.getByText("FIRST PICTURES")).toBeVisible();
  await expect(row.getByRole("img", { name: "Thumbnail" })).toBeVisible();
  await expect(row.getByRole("img", { name: "Screenshot 2" })).toBeVisible();
  await row.getByRole("button", { name: "Approve pictures" }).click();
  await expect(row).toHaveCount(0);
  // Bo's seeded pictures still wait.
  await expect(admin.locator("li").filter({ has: admin.getByRole("link", { name: "Pocket Golf" }) }).getByRole("button", { name: "Approve pictures" })).toBeVisible();

  await maker.reload();
  const live = editor.locator(".media-set").filter({ hasText: "IN THE STORE" });
  await expect(live.locator(".gallery-edit li b")).toHaveText(["YouTube video", "shots/one.png", "shots/two.png"]);
  // The shown set can be put in order too.
  await live.getByRole("button", { name: "Move item 1 later" }).click();
  await expect(live.locator(".gallery-edit li b")).toHaveText(["shots/one.png", "YouTube video", "shots/two.png"]);
  await maker.goto(`/app/${pond}`);
  await expect(maker.getByRole("region", { name: "Pictures Pond: screenshots and videos" })).toBeVisible();
});

test("staff turn pictures down: the maker reads why, and the same pictures are refused again", async ({ browser }) => {
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/publish/${puddle}`);
  const editor = maker.locator(".media-editor");
  await editor.getByRole("button", { name: "Check my repository now" }).click();
  await expect(editor.getByText("Found new pictures. They're waiting for review.")).toBeVisible({ timeout: 20_000 });

  const admin = await browser.newPage();
  await signIn(admin, "admin");
  await admin.goto("/admin");
  await admin.getByRole("link", { name: /Pictures to review$/ }).click();
  const row = admin.locator("li").filter({ has: admin.getByRole("link", { name: "Pictures Puddle" }) });
  await row.getByRole("button", { name: "Turn down" }).click();
  await row.getByRole("textbox", { name: "Turn down: reason" }).fill("The thumbnail is another game's logo.");
  await row.getByRole("button", { name: "Turn down" }).click();
  await expect(row).toHaveCount(0);

  await maker.reload();
  await expect(editor.locator(".notice").filter({ hasText: "These pictures were turned down:" })).toContainText("The thumbnail is another game's logo.");
  await editor.getByRole("button", { name: "Check my repository now" }).click();
  await expect(editor.getByRole("alert")).toContainText("these are the pictures that were turned down", { timeout: 20_000 });
});

test("pictures need a GitHub repository; a failed check says why", async ({ page }) => {
  await signIn(page, "pendingCreative");
  await page.route(path(`/v1/apps/${puddle}/media`), async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), supported: false } });
  });
  await page.goto(`/publish/${puddle}`);
  const editor = page.locator(".media-editor");
  await expect(editor.getByText("Pictures from the repository only work for GitHub repositories so far.")).toBeVisible();
  await expect(editor.getByRole("button", { name: "Check my repository now" })).toBeDisabled();

  await page.unrouteAll();
  await failApi(page, path(`/v1/apps/${puddle}/media/check`), { status: 503, message: "GitHub didn't answer. Try again in a minute." });
  await page.reload();
  await editor.getByRole("button", { name: "Check my repository now" }).click();
  await expect(editor.getByRole("alert").filter({ hasText: "GitHub didn't answer" })).toBeVisible();
});

test("the browser demo panel: hidden on this server, and hosting, checking, live and ended where it's on", async ({ page }) => {
  await signIn(page, "pendingCreative");
  // This server hosts no demos (no play address): no panel.
  await page.goto(`/publish/${pond}/manage`);
  await expect(page.getByRole("heading", { name: "Pictures Pond" }).first()).toBeVisible();
  await expect(page.getByRole("heading", { name: "Browser demo" })).toHaveCount(0);

  // Where demos are on, as demos.rs answers.
  const until = new Date(Date.now() + 10 * 864e5).toISOString();
  const ends = new Date(Date.now() + 9 * 864e5).toISOString();
  let state: object = { available: true, eligible: true, eligible_until: until, demo: null, play_url: null, days: 14 };
  const started: string[] = [];
  let ended = false;
  await page.route(path(`/v1/apps/${pond}/demo`), async (route) => {
    const req = route.request();
    if (req.method() === "POST") {
      const url = (req.postDataJSON() as { url: string }).url;
      if (!url.endsWith(".zip")) {
        return route.fulfill({ status: 400, json: { error: { code: "invalid", field: "url", message: "a web build is a .zip with index.html" } } });
      }
      started.push(url);
      state = { ...state, demo: { status: "fetching", detail: null, scan_result: null, size_bytes: null, plays: 0, ends_at: ends } };
      return route.fulfill({ status: 204 });
    }
    if (req.method() === "DELETE") {
      ended = true;
      state = { ...state, eligible: true, demo: null, play_url: null };
      return route.fulfill({ status: 204 });
    }
    return route.fulfill({ json: state });
  });
  await page.reload();
  const panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "Browser demo" }) });
  await expect(panel.getByText(/For its first 14 days, a new game can be played in the browser/)).toBeVisible();
  const address = panel.getByRole("textbox", { name: "The web build's address on your release page" });
  await address.fill("https://github.com/remy/pond/releases/download/v1.0.0/web.tar");
  await panel.getByRole("button", { name: "Host the demo" }).click();
  await expect(panel.getByRole("alert")).toHaveText("a web build is a .zip with index.html");
  await address.fill("https://github.com/remy/pond/releases/download/v1.0.0/web.zip");
  await panel.getByRole("button", { name: "Host the demo" }).click();
  await expect(panel.getByText("FETCHING")).toBeVisible();
  await expect(panel.getByText("Downloading and scanning…")).toBeVisible();
  expect(started).toEqual(["https://github.com/remy/pond/releases/download/v1.0.0/web.zip"]);

  // The scan turned it down: the maker reads why and can try another build.
  state = { ...state, demo: { status: "failed", detail: "index.html is missing from the zip.", scan_result: null, size_bytes: null, plays: 0, ends_at: ends } };
  await page.reload();
  await expect(panel.getByText("FAILED")).toBeVisible();
  await expect(panel.getByText("index.html is missing from the zip.")).toBeVisible();
  await expect(panel.getByRole("button", { name: "Host the demo" })).toBeVisible();

  // Live: where to play it, how often it's been played, and ending it.
  state = {
    ...state,
    demo: { status: "live", detail: null, scan_result: "passed", size_bytes: 1000, plays: 42, ends_at: ends },
    play_url: "https://play.example.com/play/pictures-pond/",
  };
  await page.reload();
  await expect(panel.getByText("LIVE")).toBeVisible();
  await expect(panel.getByRole("link", { name: "Open it" })).toHaveAttribute("href", "https://play.example.com/play/pictures-pond/");
  await expect(panel.getByText(/42 plays · until /)).toBeVisible();
  await expect(panel.getByRole("button", { name: "Host the demo" })).toHaveCount(0);
  await panel.getByRole("button", { name: "End the demo now" }).click();
  await expect(panel.getByText("LIVE")).toHaveCount(0);
  expect(ended).toBe(true);
  await expect(panel.getByRole("button", { name: "Host the demo" })).toBeVisible();

  // Past its first days with no demo: the panel goes.
  state = { ...state, eligible: false, eligible_until: null, demo: null };
  await page.reload();
  await expect(page.getByRole("heading", { name: "Browser demo" })).toHaveCount(0);
});

test("registering a branch: the kinds, the original's age rating as the least, and errors by field", async ({ browser }) => {
  // Signed out, it asks you to sign in first.
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${ids.listedApp}/branch`);
  await expect(visitor.locator("main").getByRole("link", { name: "Sign in" })).toHaveAttribute("href", `/signin?return_to=/app/${ids.listedApp}/branch`);

  const page = await browser.newPage();
  await signIn(page, "creative");
  await page.goto(`/app/${ids.listedApp}/branch`);
  await expect(page.getByRole("heading", { name: "Register a branch of Lantern Keep", level: 1 })).toBeVisible();
  await expect(page.getByText("Lantern Keep by Mia Maker · rated 12+ · runs on windows")).toBeVisible();
  // A port fills a gap: Lantern Keep doesn't run on Linux.
  await expect(page.getByText(/Lantern Keep doesn't run on linux yet: a port fills that gap\./)).toBeVisible();
  const kinds = page.getByRole("group", { name: "What is it?" }).getByRole("radio");
  await expect(kinds).toHaveCount(6);
  await expect(page.getByRole("radio", { name: /^Port/ })).toBeChecked();
  for (const kind of ["Modded version", "Fix", "Translation", "Other"]) {
    await page.getByRole("radio", { name: new RegExp(`^${kind}`) }).check();
    await expect(page.getByRole("heading", { name: "Register a branch of Lantern Keep" })).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Your fork" })).toBeVisible();
  }
  // A mod is its own thing: its own repository, no platforms.
  await page.getByRole("radio", { name: /^Mod —/ }).check();
  await expect(page.getByRole("heading", { name: "Publish a mod for Lantern Keep" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Your mod's repository" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Publish the mod" })).toBeVisible();
  await page.getByRole("radio", { name: /^Translation/ }).check();

  // It can't be rated below the original.
  const age = page.locator("label").filter({ hasText: "Age rating" }).getByRole("combobox");
  await expect(age).toHaveValue("12");
  await expect(age.locator("option")).not.toContainText(["Everyone"]);

  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Lantern Keep auf Deutsch");
  await page.getByRole("textbox", { name: /^Address/ }).fill("lantern-keep");
  await page.getByRole("textbox", { name: "One-line summary" }).fill("Lantern Keep, in German.");
  await page.getByRole("textbox", { name: /What it changes from Lantern Keep/ }).fill("Every line translated.");
  await page.getByRole("textbox", { name: "Your fork" }).fill("https://github.com/vera/lantern-keep-de");
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("checkbox", { name: /Windows/ }).check();
  // The address is taken: the error shows under it.
  await page.getByRole("button", { name: "Register the branch" }).click();
  const addressField = page.locator("label").filter({ hasText: /^Address/ });
  await expect(addressField.getByRole("alert")).toBeVisible();
  await page.getByRole("textbox", { name: /^Address/ }).fill("lantern-keep-de");
  await page.getByRole("button", { name: "Register the branch" }).click();
  await expect(page).toHaveURL(/\/publish\/lantern-keep-de\/releases$/);

  // It's a draft until a release is approved: not in Lantern Keep's Community area yet.
  await page.goto(`/app/${ids.listedApp}`);
  await expect(page.locator(".community-builds").getByText("Lantern Keep auf Deutsch")).toHaveCount(0);
  await page.goto("/app/lantern-keep-de");
  await expect(page.locator(".community-notice")).toContainText("A translation of Lantern Keep, made by Vera Okafor, not Mia Maker.");
});

test("registering a branch asks for an agreement first when one changed", async ({ browser }) => {
  // (It never asks for a passkey: passkeys are optional since 2026-10-25, passkeys.e2e.ts.)
  // An agreement that changed since it was accepted (as the server answers then): it's shown
  // to accept; "Not now" goes back to the form as it was, with nothing registered.
  const vera = await browser.newPage();
  await signIn(vera, "creative");
  await failApi(vera, path(`/v1/apps/${ids.listedApp}/branches`), {
    status: 403,
    code: "terms_required",
    field: "publisher",
    message: "accept the Publisher Agreement first",
  });
  await vera.goto(`/app/${ids.listedApp}/branch?kind=addon`);
  await fillMod(vera, "lantern-keep-vera-mod");
  await vera.getByRole("button", { name: "Publish the mod" }).click();
  const terms = vera.getByRole("dialog", { name: "Publisher Agreement" });
  await expect(terms.getByRole("heading", { name: "Before you continue: the Publisher Agreement" })).toBeVisible();
  await terms.getByRole("button", { name: "Not now" }).click();
  await expect(terms).toBeHidden();
  await expect(vera.getByRole("button", { name: "Publish the mod" })).toBeEnabled();
  await expect(vera.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Olly's lanterns");
  expect((await tryCall("creative", "GET", "/v1/apps/lantern-keep-vera-mod")).status).toBe(404);
});

async function fillMod(page: import("@playwright/test").Page, slug: string) {
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Olly's lanterns");
  await page.getByRole("textbox", { name: /^Address/ }).fill(slug);
  await page.getByRole("textbox", { name: "One-line summary" }).fill("More lanterns.");
  await page.getByRole("textbox", { name: /What it changes from Lantern Keep/ }).fill("Adds lanterns.");
  await page.getByRole("textbox", { name: "Your mod's repository" }).fill(`https://github.com/e2e/${slug}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
}
