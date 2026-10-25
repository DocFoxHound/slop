// SPDX-License-Identifier: AGPL-3.0-or-later
// A player's own pages: the library (and its Interested list), the notifications bell, public
// profiles, and the account page's privacy, email and phone-notification switches. Settings a
// test changes on a seeded account are put back by the same test.
import { expect, ids, server, signIn, test } from "./fixtures";
import { call, delist, draftApp, failApi, listedApp, path, released, tryCall } from "./store-helpers";

// Remy's coming-soon game, and Vera's Private Slop (shared with Fin Finance to notify him).
const soon = "kelp-keepers";
const nook = "note-nook";

test.beforeAll(async () => {
  await listedApp(soon, "Kelp Keepers", { who: "pendingCreative", stage: "coming_soon", tags: ["simulation"] });
  await draftApp(nook, "Note Nook", { who: "creative", visibility: "private" });
  await released(nook, "creative");
});

test.afterAll(async () => {
  await tryCall("player", "DELETE", `/v1/me/interested/${soon}`);
  await delist(soon);
});

test("signed out, the library asks you to sign in; an empty library points to the store", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/library");
  await expect(visitor.locator("main").getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/signin?return_to=/library");
  await expect(visitor.getByText(/to keep a library that follows you between devices\./)).toBeVisible();

  // Rae has nothing in hers.
  const rae = await browser.newPage();
  await signIn(rae, "reviewer");
  await rae.goto("/library");
  await expect(rae.getByText("Nothing here yet.")).toBeVisible();
  await rae.getByRole("link", { name: "Find something in the store" }).click();
  await expect(rae).toHaveURL(/\/$/);
});

test("the library lists what you added; taking one out there removes it", async ({ page }) => {
  await call("player", "PUT", "/v1/me/library/neon-drift");
  try {
    await signIn(page, "player");
    await page.goto("/library");
    const row = page.locator(".library-row").filter({ has: page.locator('a[href="/app/neon-drift"]') });
    await expect(row.getByText("Neon Drift: a game for the browser tests.")).toBeVisible();
    await row.getByRole("button", { name: "✓ In your library" }).click();
    await expect(row.getByRole("button", { name: "+ Add to library" })).toBeVisible();
    await page.reload();
    await expect(page.locator('.library-row a[href="/app/neon-drift"]')).toHaveCount(0);
  } finally {
    await tryCall("player", "DELETE", "/v1/me/library/neon-drift");
  }
});

test("the Interested list: what you're waiting for, Not interested, and released ones to add", async ({ page }) => {
  await call("player", "PUT", `/v1/me/interested/${soon}`);
  await signIn(page, "player");
  await page.goto("/library");
  await expect(page.getByRole("heading", { name: "Interested" })).toBeVisible();
  const row = page.locator(".library-row").filter({ has: page.locator(`a[href="/app/${soon}"]`) });
  await expect(row.getByText("COMING SOON")).toBeVisible();
  await expect(row.getByText("Kelp Keepers, made by the store's browser tests.")).toBeVisible();

  // Taking it off can fail: it says so.
  await failApi(page, path(`/v1/me/interested/${soon}`), { method: "DELETE" });
  await row.getByRole("button", { name: "Not interested" }).click();
  await expect(page.getByRole("alert")).toHaveText("Something broke on Slop's side.");
  await expect(row).toBeVisible();
  await page.unrouteAll();
  await row.getByRole("button", { name: "Not interested" }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Interested" })).toHaveCount(0);
  expect(await call<unknown[]>("player", "GET", "/v1/me/interested")).toHaveLength(0);

  // Once it's released, the list offers to add it to the library.
  await call("player", "PUT", `/v1/me/interested/${soon}`);
  await page.route(path("/v1/me/interested"), async (route) => {
    const res = await route.fetch();
    const list = (await res.json()) as Array<{ slug: string; stage: string }>;
    await route.fulfill({ response: res, json: list.map((a) => (a.slug === soon ? { ...a, stage: "released" } : a)) });
  });
  await page.reload();
  await expect(row.getByText("Released: add it to your library to install it.")).toBeVisible();
  await expect(row.getByRole("button", { name: "+ Add to library" })).toBeVisible();
  await expect(row.getByRole("button", { name: "Not interested" })).toHaveCount(0);
});

test("notifications: the bell counts them, opening one goes there, and Mark all as read clears it", async ({ browser }) => {
  // Fin gets two when Vera shares her Private Slop with him, twice. (Other tests may have sent
  // him some too: the counts start from what he has.)
  const fin = `${server.url}/u/${ids.finance}`;
  await call("creative", "DELETE", `/v1/apps/${nook}/shares/${ids.finance}`);
  const before = (await call<{ unread: number }>("finance", "GET", "/v1/me/notifications")).unread;
  await call("creative", "POST", `/v1/apps/${nook}/shares`, { user: fin });
  await call("creative", "DELETE", `/v1/apps/${nook}/shares/${ids.finance}`);
  await call("creative", "POST", `/v1/apps/${nook}/shares`, { user: fin });

  const page = await browser.newPage();
  await signIn(page, "finance");
  await page.goto("/");
  const bell = (n: number) => page.getByRole("link", { name: n > 0 ? `Notifications, ${n} unread` : "Notifications", exact: true });
  await expect(bell(before + 2)).toBeVisible();
  await bell(before + 2).click();
  await expect(page.getByRole("heading", { name: "Notifications", level: 1 })).toBeVisible();
  const shared = page.locator(".notifications li").filter({ hasText: "Vera Okafor shared Note Nook with you" });
  await expect(shared.nth(1)).toBeVisible();
  await expect(shared.first()).toHaveClass(/unread/);
  await expect(shared.first().getByText("It's Private Slop: only the people it's shared with can see it.")).toBeVisible();

  // Opening one marks it read and goes to what it's about.
  await shared.first().getByRole("button", { name: /Vera Okafor shared Note Nook with you/ }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${nook}$`));
  await expect(page.getByRole("heading", { name: "Note Nook", level: 1 })).toBeVisible();
  await expect(bell(before + 1)).toBeVisible();

  await page.goto("/notifications");
  await expect(shared.first()).not.toHaveClass(/unread/);
  await expect(shared.nth(1)).toHaveClass(/unread/);
  // Marking all can fail: it says so and nothing changes.
  await failApi(page, path("/v1/me/notifications/read"), { method: "POST" });
  await page.getByRole("button", { name: "Mark all as read" }).click();
  await expect(page.getByRole("alert")).toHaveText("Something broke on Slop's side.");
  await expect(shared.nth(1)).toHaveClass(/unread/);
  await page.unrouteAll();
  await page.getByRole("button", { name: "Mark all as read" }).click();
  await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
  await expect(page.locator(".notifications li.unread")).toHaveCount(0);
  await expect(bell(0)).toBeVisible();
  await expect(page.getByRole("link", { name: "your account page" })).toHaveAttribute("href", "/account/notifications");

  // Signed out, it asks you to sign in.
  const visitor = await browser.newPage();
  await visitor.goto("/notifications");
  await expect(visitor.locator("main").getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/signin?return_to=/notifications");
});

test("with no notifications, the page says what shows up there", async ({ page }) => {
  await page.route(path("/v1/me/notifications"), (route) => route.fulfill({ json: { unread: 0, items: [] } }));
  await signIn(page, "reviewer");
  await page.goto("/notifications");
  await expect(page.getByText(/^Nothing yet\. Job steps, apps you're Interested in/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark all as read" })).toHaveCount(0);
});

test("public profiles show votes and reviews; private ones only to their owner", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto(`/u/${ids.player}`);
  await expect(visitor.getByRole("heading", { name: "Pat Player", level: 1 })).toBeVisible();
  await expect(visitor.getByText(/^On Slop since /)).toBeVisible();
  const vote = visitor.locator(".profile-votes li").filter({ has: visitor.locator(`a[href="/app/${ids.listedApp}"]`) });
  await expect(vote.getByText("👍 Not Slop")).toBeVisible();
  await expect(vote.getByText(/Forty hours in and still finding secrets/)).toBeVisible();
  await expect(visitor.getByText("This is how others see you.")).toHaveCount(0);

  // Tia is in the under-18 view: private by default.
  await visitor.goto(`/u/${ids.teen}`);
  await expect(visitor.getByRole("heading", { name: "Tia Teen", level: 1 })).toBeVisible();
  await expect(visitor.getByText("Tia Teen keeps their votes and reviews private.")).toBeVisible();
  await expect(visitor.locator(".profile-votes")).toHaveCount(0);
  const tia = await browser.newPage();
  await signIn(tia, "teen");
  await tia.goto(`/u/${ids.teen}`);
  await expect(tia.getByText(/Your votes and reviews are private: others see only your name and apps\./)).toBeVisible();
  await expect(tia.getByRole("link", { name: "Change it" })).toHaveAttribute("href", "/account#privacy");
  await expect(tia.locator(".profile-votes li").filter({ hasText: "Really fun puzzles, the music is great." })).toBeVisible();

  // Makers' apps, and a creative's People page.
  await visitor.goto(`/u/${ids.maker}`);
  await expect(visitor.getByRole("heading", { name: "Apps they make" })).toBeVisible();
  await expect(visitor.locator(`a.tile[href="/app/${ids.listedApp}"]`).first()).toBeVisible();
  await expect(visitor.getByText("No votes yet.")).toBeVisible();
  await visitor.goto(`/u/${ids.creative}`);
  await expect(visitor.getByRole("link", { name: "Hire them on People" })).toHaveAttribute("href", `/people/${ids.creativeHandle}`);

  await visitor.goto("/u/00000000-0000-0000-0000-000000000000");
  await expect(visitor.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});

test("privacy switches save to the account and show where they matter", async ({ browser }) => {
  // Sue's profile and playtime; each is put back.
  const sue = await browser.newPage();
  await signIn(sue, "support");
  try {
    await sue.goto("/account");
    const publicProfile = sue.getByRole("checkbox", { name: /Show my votes and reviews on my profile/ });
    const playtime = sue.getByRole("checkbox", { name: /Share my playtime/ });
    await expect(publicProfile).toBeChecked();
    await publicProfile.click();
    await expect(publicProfile).not.toBeChecked();
    await playtime.click();
    await expect(playtime).not.toBeChecked();
    await sue.reload();
    await expect(publicProfile).not.toBeChecked();
    await expect(playtime).not.toBeChecked();
    expect(await call("support", "GET", "/v1/me")).toMatchObject({ profile_public: false, share_playtime: false });
    await sue.getByRole("link", { name: "See your profile" }).click();
    await expect(sue.getByText(/Your votes and reviews are private/)).toBeVisible();
    await sue.goBack();
    await publicProfile.click();
    await expect(publicProfile).toBeChecked();
    await playtime.click();
    await expect(playtime).toBeChecked();

    // A change that doesn't save says so, and the switch shows what's saved.
    await failApi(sue, path("/v1/me"), { method: "PATCH" });
    await publicProfile.click();
    await expect(sue.locator("#privacy").getByRole("alert")).toHaveText("Something broke on Slop's side.");
    await expect(publicProfile).toBeChecked();
  } finally {
    await tryCall("support", "PATCH", "/v1/me", { profile_public: true, share_playtime: true });
  }

  // Pat's recommendations: off, For you is the same for everyone; on again, it's his.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  try {
    await pat.goto("/account");
    const personal = pat.getByRole("checkbox", { name: /Recommendations based on my activity/ });
    await expect(personal).toBeChecked();
    await personal.click();
    await expect(personal).not.toBeChecked();
    await pat.goto("/");
    await expect(pat.getByText(/^Popular, well rated and new\./)).toBeVisible();
    await pat.goto("/account");
    await personal.click();
    await expect(personal).toBeChecked();
    await pat.goto("/");
    await expect(pat.getByText(/^Based on .+, plus something different every fourth$/)).toBeVisible();
  } finally {
    await tryCall("player", "PATCH", "/v1/me", { personalize: true });
  }
});

test("email choices save per kind of notification; phone notifications say what they can", async ({ page, browser }) => {
  await signIn(page, "support");
  await page.goto("/account#notifications");
  const section = page.locator("#notifications");
  await expect(section.getByText(/you don't have an email on your account, so nothing is emailed/)).toBeVisible();
  const jobs = section.getByRole("checkbox", { name: /Email me about people jobs/ });
  try {
    await expect(jobs).toBeChecked();
    await jobs.click();
    await expect(jobs).not.toBeChecked();
    await page.reload();
    await expect(jobs).not.toBeChecked();
    expect((await call<{ categories: Array<{ category: string; email: boolean }> }>("support", "GET", "/v1/me/email-prefs")).categories).toContainEqual({
      category: "jobs",
      label: expect.any(String),
      email: false,
    });
    await jobs.click();
    await expect(jobs).toBeChecked();

    await failApi(page, path("/v1/me/email-prefs"), { method: "PUT", status: 400, message: "Security emails can't be turned off." });
    await jobs.click();
    await expect(section.getByRole("alert")).toHaveText("Security emails can't be turned off.");
    await expect(jobs).toBeChecked();
    await page.unrouteAll();
  } finally {
    await tryCall("support", "PUT", "/v1/me/email-prefs", { jobs: true });
  }

  // This browser blocks notifications (headless Chromium does, until a site is allowed).
  await expect(section.getByText("📱 Notifications are blocked for Slop in this browser's settings.")).toBeVisible();

  // A browser that hasn't been asked yet gets the switch; this server has no push set up, so
  // switching it on says so. (Headless Chromium always answers "denied": pretend it hasn't.)
  const phone = await browser.newPage();
  await phone.addInitScript(() => Object.defineProperty(Notification, "permission", { get: () => "default" }));
  await signIn(phone, "support");
  await phone.goto("/account#notifications");
  const push = phone.locator("#notifications").getByRole("checkbox", { name: /Notifications on this phone or browser/ });
  await expect(push).not.toBeChecked();
  await push.click();
  await expect(phone.getByText("📱 Notifications on phones aren't switched on on this server yet.")).toBeVisible();
});
