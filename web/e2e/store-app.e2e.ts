// SPDX-License-Identifier: AGPL-3.0-or-later
// An app's store page, as players, its maker and visitors use it: voting, reviews and the
// maker's replies, what it does and its accessibility, the library and Interested buttons,
// drafts, Private Slop and its share panel, community builds, and what's shown around them.
// The apps changed here are the test's own (store-helpers.ts); the seeded ones are only read.
import type { Page } from "@playwright/test";
import { expect, ids, server, signIn, test } from "./fixtures";
import { call, delist, draftApp, failApi, listedApp, patchJson, path, played, released, tryCall } from "./store-helpers";

// Remy Pending makes this file's public apps (he already has a passkey deadline, and nothing
// else tests his apps); they leave the store again at the end.
const rated = "rated-rafts";
const soon = "soon-sails";
// Bo's Private Slop: he's Pat's friend and in Night Owls (invite-only) and Open Crew (open).
const secret = "secret-sloop";

test.beforeAll(async () => {
  await listedApp(rated, "Rated Rafts", {
    who: "pendingCreative",
    tags: ["puzzle"],
    min_age: 12,
    content_labels: ["violence"],
    accessibility: ["subtitles", "colorblind"],
    description_md: "# Rated Rafts\n\nBuild rafts. Read the [raft guide](https://example.com/raft-guide) first.",
  });
  await listedApp(soon, "Soon Sails", { who: "pendingCreative", stage: "coming_soon", tags: ["strategy"] });
  await draftApp(secret, "Secret Sloop", { visibility: "private" });
  await released(secret);
  await played("player", rated);
});

test.afterAll(async () => {
  await delist(rated, soon);
});

const ratings = (page: Page) => page.getByRole("region", { name: "Ratings" });

test("a player votes Not Slop, switches to Slop, and takes the vote back", async ({ page }) => {
  await tryCall("player", "POST", `/v1/apps/${rated}/vote`, { vote: null });
  await signIn(page, "player");
  await page.goto(`/app/${rated}`);
  const up = ratings(page).getByRole("button", { name: "👍 Not Slop" });
  const down = ratings(page).getByRole("button", { name: "👎 Slop" });
  await expect(ratings(page).getByText("No votes yet")).toBeVisible();
  await expect(ratings(page).getByText(/Your vote counts \d+% \(more as you play\)/)).toBeVisible();

  await up.click();
  await expect(up).toHaveAttribute("aria-pressed", "true");
  await expect(ratings(page).getByText("1 vote", { exact: true })).toBeVisible();
  await down.click();
  await expect(down).toHaveAttribute("aria-pressed", "true");
  await expect(up).toHaveAttribute("aria-pressed", "false");
  await page.reload();
  await expect(down).toHaveAttribute("aria-pressed", "true");
  // Pressing it again takes it back.
  await down.click();
  await expect(down).toHaveAttribute("aria-pressed", "false");
  await expect(ratings(page).getByText("No votes yet")).toBeVisible();

  // A refused vote says why.
  await failApi(page, path(`/v1/apps/${rated}/vote`), { status: 429, message: "Too many votes. Try again in a minute." });
  await up.click();
  await expect(ratings(page).getByRole("alert")).toHaveText("Too many votes. Try again in a minute.");
  await expect(up).toHaveAttribute("aria-pressed", "false");
});

test("only players who played it can vote; makers, visitors and upcoming apps can't", async ({ browser }) => {
  // Tia never launched it: the server says why.
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${rated}`);
  await expect(ratings(teen).getByText("Launch it from the Slop app first: only players can vote.")).toBeVisible();
  await expect(ratings(teen).getByRole("button", { name: "👍 Not Slop" })).toHaveCount(0);

  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${rated}`);
  await expect(ratings(maker).getByText("Makers can't vote on their own apps.")).toBeVisible();

  const visitor = await browser.newPage();
  await visitor.goto(`/app/${rated}`);
  await expect(ratings(visitor).getByText("Sign in and play it from the Slop app to vote.")).toBeVisible();
  await visitor.goto(`/app/${soon}`);
  await expect(ratings(visitor).getByText("Voting opens when the app is released.")).toBeVisible();
});

test("the rating is at the top of the page: Rate it jumps to the vote, or it says why you can't yet", async ({ page, browser }) => {
  await tryCall("player", "POST", `/v1/apps/${rated}/vote`, { vote: null });
  const head = (p: Page) => p.locator(".rating-head");
  try {
    await signIn(page, "player");
    await page.goto(`/app/${rated}`);
    await expect(head(page).getByText("No ratings yet")).toBeVisible();
    // The vote comes before "What this app does".
    const panel = await ratings(page).boundingBox();
    const label = await page.getByRole("heading", { name: "What this app does" }).boundingBox();
    expect(panel!.y).toBeLessThan(label!.y);
    await head(page).getByRole("button", { name: "Rate it" }).click();
    await expect(ratings(page)).toBeFocused();
    await ratings(page).getByRole("button", { name: "👍 Not Slop" }).click();
    await expect(head(page).getByRole("button", { name: "Your rating: 👍 Not Slop" })).toBeVisible();
    await expect(head(page).getByText("1 vote so far; a rating shows from")).toBeVisible();
  } finally {
    await tryCall("player", "POST", `/v1/apps/${rated}/vote`, { vote: null });
  }

  // A rated app: the rating, its label and the votes.
  await page.goto(`/app/${ids.listedApp}`);
  await expect(head(page).locator(".rating-big")).toHaveText(/^\d+% Not Slop$/);
  await expect(head(page).getByText(/\d+ votes$/)).toBeVisible();

  // Why others can't.
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${rated}`);
  await expect(head(teen).getByText("Play it from the Slop app to rate it.")).toBeVisible();
  await expect(head(teen).getByRole("button")).toHaveCount(0);
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${rated}`);
  await expect(head(maker).getByText("Makers can't rate their own app.")).toBeVisible();
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${rated}`);
  await expect(head(visitor).getByText(/and play it from the Slop app to rate it\./)).toBeVisible();
  await expect(head(visitor).getByRole("link", { name: "Sign in" })).toHaveAttribute("href", `/signin?return_to=${encodeURIComponent(`/app/${rated}`)}`);
  await visitor.goto(`/app/${soon}`);
  await expect(head(visitor).getByText("Voting opens when it's released.")).toBeVisible();
});

test("the rating chart switches between weighted and one account, one vote", async ({ page }) => {
  await page.goto(`/app/${ids.listedApp}`);
  const weighted = ratings(page).getByRole("button", { name: "Weighted by playtime" });
  const flat = ratings(page).getByRole("button", { name: "One account, one vote" });
  await expect(weighted).toHaveAttribute("aria-pressed", "true");
  await flat.click();
  await expect(flat).toHaveAttribute("aria-pressed", "true");
  await expect(weighted).toHaveAttribute("aria-pressed", "false");
  await weighted.click();
  await expect(weighted).toHaveAttribute("aria-pressed", "true");
});

test("a voter writes, edits and deletes their review", async ({ page }) => {
  await call("player", "POST", `/v1/apps/${rated}/vote`, { vote: null });
  await signIn(page, "player");
  await page.goto(`/app/${rated}`);
  // No vote, no review; and with no reviews at all, no Reviews section.
  await expect(ratings(page).getByRole("button", { name: "Add a written review" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Reviews" })).toHaveCount(0);

  await ratings(page).getByRole("button", { name: "👍 Not Slop" }).click();
  await ratings(page).getByRole("button", { name: "Add a written review" }).click();
  const box = ratings(page).getByRole("textbox", { name: "Your review" });
  await box.fill("Sturdy rafts, calm water. A lovely evening.");
  await ratings(page).getByRole("button", { name: "Save review" }).click();
  await expect(ratings(page).getByText("Sturdy rafts, calm water. A lovely evening.")).toBeVisible();
  // The list below picks it up at once.
  const reviews = page.locator("#reviews");
  await expect(reviews.getByRole("heading", { name: "Reviews" })).toBeVisible();
  await expect(reviews.getByText("Sturdy rafts, calm water. A lovely evening.")).toBeVisible();

  await ratings(page).getByRole("button", { name: "Edit", exact: true }).click();
  await box.fill("Sturdy rafts. The storms in chapter two are hard.");
  await ratings(page).getByRole("button", { name: "Save review" }).click();
  await expect(reviews.getByText("Sturdy rafts. The storms in chapter two are hard.")).toBeVisible();

  // Deleting can fail; it says so and keeps the review.
  await failApi(page, path(`/v1/apps/${rated}/review`), { method: "DELETE" });
  await ratings(page).getByRole("button", { name: "Delete", exact: true }).click();
  await expect(ratings(page).getByRole("alert")).toHaveText("Something broke on Slop's side.");
  await expect(ratings(page).getByText("Sturdy rafts. The storms in chapter two are hard.")).toBeVisible();
  await page.unrouteAll();
  await ratings(page).getByRole("button", { name: "Delete", exact: true }).click();
  await expect(ratings(page).getByRole("button", { name: "Add a written review" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Reviews" })).toHaveCount(0);

  // A review that can't be saved says why, and keeps what was typed.
  await failApi(page, path(`/v1/apps/${rated}/review`), { status: 400, message: "Reviews can be up to 4,000 characters.", method: "PUT" });
  await ratings(page).getByRole("button", { name: "Add a written review" }).click();
  await box.fill("Short.");
  await ratings(page).getByRole("button", { name: "Save review" }).click();
  await expect(ratings(page).getByRole("alert")).toHaveText("Reviews can be up to 4,000 characters.");
  await expect(box).toHaveValue("Short.");
});

test("a review hidden by moderation tells its writer where to read why", async ({ page }) => {
  // As the server answers once a moderator hid Pat's review (moderation.rs).
  await patchJson(page, path(`/v1/apps/${rated}/ratings`), (r) => ({
    ...r,
    mine: { ...r.mine, vote: "not_slop", review: { body: "Hidden in this test only.", hidden: true } },
  }));
  await signIn(page, "player");
  await page.goto(`/app/${rated}`);
  await expect(ratings(page).getByText("Hidden in this test only.")).toBeVisible();
  await expect(ratings(page).getByText(/Hidden by a moderation decision/)).toBeVisible();
  await expect(ratings(page).getByRole("link", { name: "account page" })).toHaveAttribute("href", "/account/moderation#decisions");
});

test("reviews filter by vote, hide private names, and page in more", async ({ page }) => {
  // The seed's three (Pat's and Tia's 👍, Kai's 👎), and any other tests wrote.
  const counts = await call<{ not_slop: number; slop: number }>(null, "GET", `/v1/apps/${ids.listedApp}/reviews`);
  await signIn(page, "player");
  await page.goto(`/app/${ids.listedApp}`);
  const reviews = page.locator("#reviews");
  const all = reviews.getByRole("button", { name: `All ${counts.not_slop + counts.slop}` });
  await expect(all).toHaveAttribute("aria-pressed", "true");
  await reviews.getByRole("button", { name: `👎 ${counts.slop}` }).click();
  await expect(reviews.getByText("Crashed twice on level 3 and lost my save.")).toBeVisible();
  await expect(reviews.getByText(/Forty hours in/)).toHaveCount(0);
  await reviews.getByRole("button", { name: `👍 ${counts.not_slop}` }).click();
  await expect(reviews.getByText(/Forty hours in/)).toBeVisible();
  await expect(reviews.getByText("Crashed twice on level 3 and lost my save.")).toHaveCount(0);
  await all.click();
  await expect(reviews.locator("li")).toHaveCount(Math.min(20, counts.not_slop + counts.slop));

  // Tia is in the under-18 view: her profile is private, so her review is "A player", unlinked.
  const tias = reviews.locator("li").filter({ hasText: "Really fun puzzles" });
  await expect(tias.getByText("A player")).toBeVisible();
  await expect(tias.getByRole("link")).toHaveCount(0);
  // Pat's own review can't be reported by Pat; the others can.
  const pats = reviews.locator("li").filter({ hasText: "Forty hours in" });
  await expect(pats.getByRole("link", { name: "Pat Player" })).toBeVisible();
  await expect(pats.getByRole("button", { name: "⚑ Report" })).toHaveCount(0);
  await expect(tias.getByRole("button", { name: "⚑ Report" })).toBeVisible();
  // The maker's reply shows under the review it answers.
  const crash = reviews.locator("li").filter({ hasText: "Crashed twice" });
  await expect(crash.getByText("The maker replied")).toBeVisible();
  await expect(crash.getByText(/1\.0\.1 fixes the level 3 crash/)).toBeVisible();

  // More than a page of reviews: "More reviews" adds the next page, and says when it can't.
  let paged = false;
  await page.route(
    (u) => u.pathname === `/v1/apps/${ids.listedApp}/reviews`,
    async (route) => {
      const offset = new URL(route.request().url()).searchParams.get("offset");
      const res = await route.fetch();
      const body = await res.json();
      if (!offset) return route.fulfill({ response: res, json: { ...body, next_offset: 3 } });
      if (!paged) {
        paged = true;
        return route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy. Try again soon." } } });
      }
      const older = { ...body.reviews[0], id: "00000000-0000-0000-0000-000000000001", body: "An older review, from the next page." };
      return route.fulfill({ json: { ...body, reviews: [older], next_offset: null } });
    },
  );
  await page.reload();
  await reviews.getByRole("button", { name: "More reviews" }).click();
  await expect(reviews.getByRole("alert")).toHaveText("Slop is busy. Try again soon.");
  await reviews.getByRole("button", { name: "More reviews" }).click();
  await expect(reviews.getByText("An older review, from the next page.")).toBeVisible();
  await expect(reviews.getByRole("button", { name: "More reviews" })).toHaveCount(0);
  await expect(reviews.getByRole("alert")).toHaveCount(0);
});

test("the maker replies to a review, changes and deletes the reply; the reviewer can report it", async ({ browser }) => {
  await played("client", rated);
  await call("client", "POST", `/v1/apps/${rated}/vote`, { vote: "slop" });
  await call("client", "PUT", `/v1/apps/${rated}/review`, { body: "The raft sank on the first wave." });
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${rated}`);
  const review = maker.locator("#reviews li").filter({ hasText: "The raft sank on the first wave." });
  await review.getByRole("button", { name: "Reply", exact: true }).click();
  const box = review.getByRole("textbox", { name: "Your reply, shown under the review" });
  await box.fill("Thanks! 1.1 makes the rafts float higher.");
  // A reply that can't be saved says why and stays open.
  await failApi(maker, path(`/v1/apps/${rated}/reviews/${await reviewId("Cleo Client")}/reply`), {
    status: 400,
    message: "Replies can be up to 2,000 characters.",
    method: "PUT",
  });
  await review.getByRole("button", { name: "Post reply" }).click();
  await expect(review.getByRole("alert")).toHaveText("Replies can be up to 2,000 characters.");
  await maker.unrouteAll();
  await review.getByRole("button", { name: "Post reply" }).click();
  await expect(review.getByText("The maker replied")).toBeVisible();
  await expect(review.getByText("Thanks! 1.1 makes the rafts float higher.")).toBeVisible();

  await review.getByRole("button", { name: "Change your reply" }).click();
  await box.fill("Thanks! 1.1 makes the rafts float higher, and adds a bilge pump.");
  await review.getByRole("button", { name: "Post reply" }).click();
  await expect(review.getByText(/adds a bilge pump/)).toBeVisible();

  // Cleo changes the review: the reply now answers an earlier version. She can report the reply.
  const pat = await browser.newPage();
  await signIn(pat, "client");
  await call("client", "PUT", `/v1/apps/${rated}/review`, { body: "The raft sank on the first wave. Still sinks in 1.0." });
  await pat.goto(`/app/${rated}`);
  const mine = pat.locator("#reviews li").filter({ hasText: "Still sinks in 1.0." });
  await expect(mine.getByText(/to an earlier version of this review/)).toBeVisible();
  await expect(mine.getByRole("button", { name: "Reply", exact: true })).toHaveCount(0);
  await mine.getByRole("button", { name: "⚑ Report" }).click();
  await expect(pat.getByRole("dialog", { name: "Report the maker's reply" })).toBeVisible();
  await pat.keyboard.press("Escape");

  // Others see no Reply button.
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${rated}`);
  await expect(teen.locator("#reviews").getByText("Still sinks in 1.0.")).toBeVisible();
  await expect(teen.getByRole("button", { name: /^(Reply|Change your reply)$/ })).toHaveCount(0);

  await maker.reload();
  await review.getByRole("button", { name: "Change your reply" }).click();
  await review.getByRole("button", { name: "Delete reply" }).click();
  await expect(review.getByText("The maker replied")).toHaveCount(0);
  await expect(review.getByRole("button", { name: "Reply", exact: true })).toBeVisible();
  await call("client", "DELETE", `/v1/apps/${rated}/review`);
});

/** The ID of the rated app's review by `author`. */
async function reviewId(author: string): Promise<string> {
  const page = await call<{ reviews: Array<{ id: string; author: string }> }>(null, "GET", `/v1/apps/${rated}/reviews`);
  const r = page.reviews.find((x) => x.author === author);
  if (!r) throw new Error(`no review by ${author}`);
  return r.id;
}

test("players confirm or dispute accessibility features, and take it back", async ({ browser }) => {
  const page = await browser.newPage();
  await signIn(page, "player");
  await page.goto(`/app/${rated}`);
  const label = page.locator("#what-it-does");
  const subtitles = label.getByRole("group", { name: 'Does "Subtitles or captions" work for you?' });
  await expect(label.getByText("Colourblind modes")).toBeVisible();
  await expect(label.getByText(/the maker says · 0 players confirmed/).first()).toBeVisible();
  await subtitles.getByRole("button", { name: "Works for me" }).click();
  await expect(subtitles.getByRole("button", { name: "Works for me" })).toHaveAttribute("aria-pressed", "true");
  await expect(label.locator("li").filter({ hasText: "Subtitles or captions" }).getByText(/1 player confirmed/)).toBeVisible();
  await subtitles.getByRole("button", { name: "Doesn't work" }).click();
  await expect(label.locator("li").filter({ hasText: "Subtitles or captions" }).getByText(/0 players confirmed, 1 say it doesn't work/)).toBeVisible();
  // Pressing it again takes it back.
  await subtitles.getByRole("button", { name: "Doesn't work" }).click();
  await expect(subtitles.getByRole("button", { name: "Doesn't work" })).toHaveAttribute("aria-pressed", "false");
  await expect(label.locator("li").filter({ hasText: "Subtitles or captions" }).getByText(/· 0 players confirmed$/)).toBeVisible();

  // A refusal is shown.
  await failApi(page, (u) => u.pathname.startsWith(`/v1/apps/${rated}/accessibility/`), { status: 403, message: "Play it first." });
  await subtitles.getByRole("button", { name: "Works for me" }).click();
  await expect(label.getByRole("alert")).toHaveText("Play it first.");

  // Before installing: the label's notes, in the install box.
  const box = page.locator(".install-box");
  await expect(box.getByText("Before you install:")).toBeVisible();
  await expect(box.getByText("Files not scanned")).toBeVisible();
  await expect(box.getByRole("link", { name: "What this app does" })).toHaveAttribute("href", "#what-it-does");

  // Only players who played it get the buttons.
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${rated}`);
  await expect(visitor.locator("#what-it-does").getByText("Subtitles or captions")).toBeVisible();
  await expect(visitor.getByRole("button", { name: "Works for me" })).toHaveCount(0);
});

test("adding to the library and taking it out again, and a refusal", async ({ page }) => {
  await tryCall("player", "DELETE", `/v1/me/library/${rated}`);
  await signIn(page, "player");
  await page.goto(`/app/${rated}`);
  const add = page.getByRole("button", { name: "+ Add to library" });
  await add.click();
  const inLibrary = page.getByRole("button", { name: "✓ In your library" });
  await expect(inLibrary).toHaveAttribute("aria-pressed", "true");
  // The page knows after a reload too.
  await page.reload();
  await expect(inLibrary).toBeVisible();
  await inLibrary.click();
  await expect(add).toHaveAttribute("aria-pressed", "false");
  await page.goto("/library");
  await expect(page.getByRole("link", { name: "Rated Rafts" })).toHaveCount(0);

  await page.goto(`/app/${rated}`);
  await failApi(page, path(`/v1/me/library/${rated}`), { status: 403, message: "You can't add that app." });
  await add.click();
  await expect(page.locator(".install-box").getByRole("alert")).toHaveText("You can't add that app.");
  await expect(add).toBeVisible();
});

test("Interested: players mark upcoming apps, the maker sees the count, visitors are asked to sign in", async ({ browser }) => {
  await tryCall("player", "DELETE", `/v1/me/interested/${soon}`);
  const visitor = await browser.newPage();
  await visitor.goto(`/app/${soon}`);
  await expect(visitor.getByText(/0 people are interested/)).toBeVisible();
  await expect(visitor.locator(".interested-count").getByRole("link", { name: "Sign in" })).toHaveAttribute(
    "href",
    `/signin?return_to=${encodeURIComponent(`/app/${soon}`)}`,
  );

  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${soon}`);
  const button = pat.getByRole("button", { name: "☆ I'm interested" });
  await button.click();
  const on = pat.getByRole("button", { name: "★ Interested" });
  await expect(on).toHaveAttribute("aria-pressed", "true");
  await expect(pat.getByText(/1 person is interested\. It's in your library; we'll tell you when it reaches beta or is released\./)).toBeVisible();
  await pat.reload();
  await expect(on).toBeVisible();

  // The maker sees only the count.
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${soon}`);
  await expect(maker.getByText("★ 1 person is interested")).toBeVisible();
  await expect(maker.getByRole("button", { name: /interested/i })).toHaveCount(0);

  await on.click();
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await expect(pat.getByText(/0 people are interested\. Mark it to hear when it's out\./)).toBeVisible();

  await failApi(pat, path(`/v1/me/interested/${soon}`), { status: 409, message: "It's already released." });
  await button.click();
  await expect(pat.locator(".interested").getByRole("alert")).toHaveText("It's already released.");
  await expect(button).toHaveAttribute("aria-pressed", "false");

  // Released apps have no Interested button.
  await pat.goto(`/app/${rated}`);
  await expect(pat.getByRole("button", { name: /interested/i })).toHaveCount(0);
});

test("a coming-soon page has no builds, a beta deadline, and the Upcoming crumb", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(`/app/${soon}`);
  await expect(page.getByRole("heading", { name: "Soon Sails", level: 1 })).toBeVisible();
  await expect(page.locator(".title-row").getByText("COMING SOON")).toBeVisible();
  await expect(page.getByText("No builds yet. This is a Coming soon page.")).toBeVisible();
  await expect(page.locator(".stage-panel").getByText(/^Beta due in \d+ days?$/)).toBeVisible();
  await expect(page.locator(".stage-panel").getByText(/^Coming soon since /)).toBeVisible();
  await expect(page.locator(".crumbs").getByRole("link", { name: "Upcoming" })).toBeVisible();
  // No bug reports or donations before there's a build.
  await expect(page.locator("#bug-reports")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "+ Add to library" })).toHaveCount(0);
});

test("a dead project says so instead of its deadline", async ({ page }) => {
  await patchJson(page, path(`/v1/apps/${soon}`), (app) => ({ ...app, is_dead: true, last_commit_at: new Date(Date.now() - 120 * 864e5).toISOString() }));
  await page.goto(`/app/${soon}`);
  await expect(page.locator(".stage-panel").getByText("Dead project")).toBeVisible();
  await expect(page.locator(".stage-panel").getByText("No code changes in 120 days")).toBeVisible();
  await expect(page.locator(".title-row").getByText("DEAD PROJECT")).toBeVisible();
});

test("an app's page shows its age rating, content labels, tags and source", async ({ browser }) => {
  const page = await browser.newPage();
  await page.goto(`/app/${rated}`);
  await expect(page.locator(".title-row").getByText("12+")).toBeVisible();
  await expect(page.getByLabel("Content labels").getByText("VIOLENCE")).toBeVisible();
  await expect(page.locator(".app-side").getByRole("link", { name: "Puzzle" })).toHaveAttribute("href", "/tag/puzzle");
  await expect(page.locator(".crumbs").getByRole("link", { name: "Games" })).toHaveAttribute("href", "/browse/games");
  await expect(page.locator("dl.kv").getByText("MIT")).toBeVisible();
  // Report is for other people's apps.
  await expect(page.locator(".report-row").getByRole("button", { name: "⚑ Report" })).toBeVisible();

  // A link in the description opens outside Slop, in a new tab.
  await page.context().route("https://example.com/**", (r) => r.fulfill({ body: "<title>Raft guide</title>", contentType: "text/html" }));
  const [guide] = await Promise.all([page.context().waitForEvent("page"), page.locator(".prose").getByRole("link", { name: "raft guide" }).click()]);
  await expect(guide).toHaveURL("https://example.com/raft-guide");
  await expect(page).toHaveURL(new RegExp(`/app/${rated}$`));

  // Its maker sees edit links and no Report button.
  const maker = await browser.newPage();
  await signIn(maker, "pendingCreative");
  await maker.goto(`/app/${rated}`);
  await expect(maker.getByRole("link", { name: "✎ Edit store page & media" })).toHaveAttribute("href", `/publish/${rated}`);
  await expect(maker.getByRole("link", { name: "⬆ Releases" })).toHaveAttribute("href", `/publish/${rated}/releases`);
  await expect(maker.locator(".report-row")).toHaveCount(0);
});

test("a draft is only for its maker: others can't find it", async ({ browser }) => {
  const maker = await browser.newPage();
  await signIn(maker, "maker");
  await maker.goto(`/app/${ids.draftApp}`);
  const notice = maker.locator(".notice").filter({ hasText: "Only you can see this page." });
  await expect(notice.getByText("DRAFT", { exact: true })).toBeVisible();
  await expect(notice.getByText(/It goes in the store once it's published and passes every scan\./)).toBeVisible();
  await expect(notice.getByRole("link", { name: "Edit draft" })).toHaveAttribute("href", `/publish/${ids.draftApp}`);

  for (const who of ["player", null] as const) {
    const other = await browser.newPage();
    if (who) await signIn(other, who);
    await other.goto(`/app/${ids.draftApp}`);
    await expect(other.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
    await expect(other.getByRole("alert").getByRole("button", { name: "Try again" })).toHaveCount(0);
  }
});

test("a page that fails to load offers to try again", async ({ page }) => {
  let fail = true;
  await page.route(path(`/v1/apps/${rated}`), (route) =>
    fail ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy. Try again soon." } } }) : route.fallback(),
  );
  await page.goto(`/app/${rated}`);
  await expect(page.getByRole("alert")).toContainText("Slop is busy. Try again soon.");
  fail = false;
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("heading", { name: "Rated Rafts", level: 1 })).toBeVisible();
});

test("a community build says whose it is and asks players to read its caveats first", async ({ browser }) => {
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${ids.communityBuild}`);
  const notice = pat.locator(".community-notice");
  await expect(notice).toContainText("A port of Lantern Keep, made by Bo Builder, not Mia Maker.");
  await expect(notice.getByRole("link", { name: "Lantern Keep" })).toHaveAttribute("href", `/app/${ids.listedApp}`);
  const box = pat.locator(".install-box");
  await expect(box.getByText("Made by Bo Builder, not Mia Maker.")).toBeVisible();
  await expect(box.getByText("May lag behind Lantern Keep's official updates.")).toBeVisible();
  await expect(box.getByRole("button", { name: "+ Add to library" })).toHaveCount(0);
  await box.getByRole("checkbox", { name: "I understand this is a community build." }).click();
  await expect(box.getByRole("button", { name: /Add to library|In your library/ })).toBeVisible();

  // Its author installs it without the caveats.
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/app/${ids.communityBuild}`);
  await expect(bo.locator(".install-box").getByRole("checkbox")).toHaveCount(0);
  await expect(bo.locator(".install-box").getByRole("button", { name: /Add to library|In your library/ })).toBeVisible();
});

test("the Community area lists mods and builds, with publish links for everyone but the maker", async ({ browser }) => {
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${ids.listedApp}`);
  const area = pat.locator(".community-builds");
  const mod = area.locator("li").filter({ hasText: "More Lanterns" });
  await expect(mod.getByText("MOD", { exact: true })).toBeVisible();
  await expect(mod.getByText("CONTAINS CODE")).toHaveCount(0);
  await expect(mod.getByText(/by Bo Builder · Twelve new puzzles and a harder fog\./)).toBeVisible();
  await expect(mod.getByRole("link", { name: "More Lanterns" })).toHaveAttribute("href", `/app/${ids.mod}`);
  const build = area.locator("li").filter({ hasText: "Lantern Keep for Linux" });
  await expect(build.getByText("COMMUNITY", { exact: true })).toBeVisible();
  await expect(build.getByText(/Port by Bo Builder · adds Linux, \.deb/)).toBeVisible();
  await expect(area.getByRole("link", { name: "Publish a mod →" })).toHaveAttribute("href", `/app/${ids.listedApp}/branch?kind=addon`);
  await expect(area.getByRole("link", { name: "Register a branch from your fork →" })).toBeVisible();

  // A mod with code that goes online says so (as the server marks one from its source check).
  await patchJson(pat, path(`/v1/apps/${ids.listedApp}`), (app) => ({
    ...app,
    community: app.community.map((b: { branch_kind: string }) => (b.branch_kind === "addon" ? { ...b, mod_code: true, mod_online: true } : b)),
  }));
  await pat.reload();
  await expect(mod.getByText("CONTAINS CODE")).toBeVisible();
  await expect(mod.getByText("GOES ONLINE")).toBeVisible();
  await pat.unrouteAll();

  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/app/${ids.listedApp}`);
  await expect(mia.locator(".community-builds").getByText("More Lanterns")).toBeVisible();
  await expect(mia.getByRole("link", { name: "Publish a mod →" })).toHaveCount(0);

  // A mod's own page, on the website, says where it's added.
  await pat.goto(`/app/${ids.mod}`);
  await expect(pat.locator(".install-box")).toContainText(
    "A mod for Lantern Keep. Add it to the game from your library in the Slop app, which checks it against Slop's signature first.",
  );
  await expect(pat.locator(".install-box").getByRole("link", { name: "Lantern Keep" })).toHaveAttribute("href", `/app/${ids.listedApp}`);

  // An app with nothing yet says so.
  await pat.goto(`/app/${rated}`);
  await expect(pat.locator(".community-builds").getByText(/^None yet\. Mods are checked/)).toBeVisible();
  await expect(pat.locator(".community-builds").getByText(/^None yet\. Modded versions/)).toBeVisible();
});

test("similar shelves leave out empty ones and say games for games", async ({ page }) => {
  await page.goto(`/app/${ids.listedApp}`);
  const similar = page.locator(".similar");
  await expect(similar.getByRole("heading", { name: "Discover something new" })).toBeVisible();
  await expect(similar.getByText("Fresh games that haven't been found yet")).toBeVisible();

  const someApp = (await call<{ apps: unknown[] }>(null, "GET", "/v1/apps?limit=1")).apps[0];
  await patchJson(page, path(`/v1/apps/${ids.listedApp}/similar`), () => ({ like_this: [someApp], popular: [], discover: [] }));
  await page.reload();
  await expect(similar.getByRole("heading", { name: "More like this" })).toBeVisible();
  await expect(similar.getByText("Shares the most tags")).toBeVisible();
  await expect(similar.getByRole("heading", { name: "Popular right now" })).toHaveCount(0);
  await expect(similar.getByRole("heading", { name: "Discover something new" })).toHaveCount(0);
});

test("the install box shows a demo, Linux packages, sandbox notes and how the build was made, when there are", async ({ page }) => {
  const ends = new Date(Date.now() + 5 * 864e5).toISOString();
  await patchJson(page, path(`/v1/apps/${rated}`), (app) => ({
    ...app,
    demo: { url: "https://play.example.com/play/rafts/", ends_at: ends },
    sandbox_ready: true,
    sandbox_mismatch: false,
    sandbox_trouble: true,
  }));
  await patchJson(page, path(`/v1/apps/${rated}/releases`), (releases) => {
    const [r] = releases;
    const file = r.files[0];
    return [
      {
        ...r,
        files: [
          {
            ...file,
            provenance: {
              source_repo: "https://github.com/bo/rafts",
              commit: "abcdef1234567890abcdef1234567890abcdef12",
              signer_workflow: "https://github.com/slop/build/.github/workflows/build.yml@refs/tags/v1",
              built_by_slop_build: true,
              runner: "github-hosted",
              run_url: "https://github.com/bo/rafts/actions/runs/1",
            },
            scan_report: { result: "passed", detail: "", engines: { clamav: "1.4" }, findings: [], capabilities: [] },
            defender: { sha256: file.sha256, engine: "1.1", signatures: "1.2", signatures_updated: null, detections: [], signed_by: "Bo Builder" },
          },
          { ...file, platform: "linux-deb", url: "https://github.com/bo/rafts/releases/download/v1.0.0/rafts.deb", provenance: { source_repo: "", commit: "abcdef1", signer_workflow: "x", built_by_slop_build: true, runner: null, run_url: null } },
          { ...file, platform: "linux-rpm", url: "https://github.com/bo/rafts/releases/download/v1.0.0/rafts.rpm", provenance: { source_repo: "", commit: "abcdef1", signer_workflow: "x", built_by_slop_build: true, runner: null, run_url: null } },
        ],
      },
    ];
  });
  await page.goto(`/app/${rated}`);
  const demo = page.getByRole("link", { name: "▶ Play the demo in your browser" });
  await expect(demo).toHaveAttribute("href", "https://play.example.com/play/rafts/");
  await expect(demo).toHaveAttribute("target", "_blank");
  await expect(demo).toHaveAttribute("rel", "noreferrer");
  await expect(page.getByText(/Free to try until/)).toBeVisible();
  const packages = page.locator(".packages");
  await expect(packages.getByRole("link", { name: ".deb" })).toHaveAttribute("href", /rafts\.deb$/);
  await expect(packages.getByRole("link", { name: ".rpm" })).toHaveAttribute("href", /rafts\.rpm$/);
  const trust = page.locator(".trust");
  await expect(trust.getByText("Built by Slop Build from source")).toBeVisible();
  await expect(trust.getByRole("link", { name: "slop/build/.github/workflows/build.yml" })).toHaveAttribute("href", "https://github.com/bo/rafts/actions/runs/1");
  await expect(trust.getByText(/at abcdef1/)).toBeVisible();
  await expect(trust.getByText(/Microsoft Defender found nothing · Windows program signed by Bo Builder/)).toBeVisible();
  // What's known about sandbox mode: the maker's word, and players who say otherwise.
  const sandbox = page.locator(".sandbox-note");
  await expect(sandbox.getByText("WORKS IN SANDBOX MODE")).toBeVisible();
  await expect(sandbox.getByText(/Some players say it doesn't start in sandbox mode\./)).toBeVisible();

  // Without them: no demo, no packages, no provenance.
  await page.unrouteAll();
  await page.reload();
  await expect(page.getByRole("heading", { name: "Rated Rafts", level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: /Play the demo/ })).toHaveCount(0);
  await expect(page.locator(".packages")).toHaveCount(0);
  await expect(page.locator(".trust").getByText(/Built .*from source/)).toHaveCount(0);
  await expect(page.locator(".sandbox-note")).toHaveCount(0);
});

test("the store page's gallery steps with the arrow keys", async ({ page }) => {
  await page.goto(`/app/${ids.listedApp}`);
  const gallery = page.getByRole("region", { name: "Lantern Keep: screenshots and videos" });
  const strip = gallery.getByRole("tablist", { name: "Choose a screenshot or video" });
  await strip.getByRole("tab", { name: "The keep at dusk" }).focus();
  await expect(gallery.getByRole("img", { name: "The keep at dusk" })).toBeVisible();
  await page.keyboard.press("ArrowRight");
  await expect(strip.getByRole("tab", { name: "Lighting the first lantern" })).toHaveAttribute("aria-selected", "true");
  await expect(gallery.getByRole("img", { name: "Lighting the first lantern" })).toBeVisible();
  await page.keyboard.press("ArrowRight");
  await expect(gallery.getByRole("button", { name: /Play video: Video 3 of 3/ })).toBeVisible();
  // It wraps around, both ways.
  await page.keyboard.press("ArrowRight");
  await expect(gallery.getByRole("img", { name: "The keep at dusk" })).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(gallery.getByRole("button", { name: /Play video/ })).toBeVisible();
  await expect(gallery.locator("iframe")).toHaveCount(0);
});

test("Private Slop: its owner shares it with friends, people and invite-only teams, and stops", async ({ browser }) => {
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/app/${secret}`);
  await expect(bo.getByText("PRIVATE SLOP")).toBeVisible();
  await expect(bo.getByText("Only you and the people you share it with can see this. It's never in the store.")).toBeVisible();
  const panel = bo.locator(".share-panel");
  await expect(panel.getByText("Nobody yet.")).toBeVisible();
  // Private Slop has no votes or reviews.
  await expect(bo.getByRole("region", { name: "Ratings" })).toHaveCount(0);

  // A friend, from the list.
  await panel.getByRole("combobox", { name: "One of your friends" }).selectOption({ label: "Pat Player" });
  await panel.getByRole("button", { name: "Share with them" }).click();
  await expect(panel.getByRole("link", { name: "Pat Player" })).toBeVisible();
  // Pat was Bo's only friend: the list is gone.
  await expect(panel.getByRole("combobox", { name: "One of your friends" })).toHaveCount(0);

  // Anyone, by their profile link; a bad link or your own is refused.
  const link = panel.getByRole("textbox", { name: "Their profile link" });
  await link.fill("not a link");
  await panel.getByRole("button", { name: "Share", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("paste their profile link");
  await link.fill(`${server.url}/u/${ids.maker2}`);
  await panel.getByRole("button", { name: "Share", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("it's already yours");
  await link.fill(`${server.url}/u/${ids.teen}`);
  await panel.getByRole("button", { name: "Share", exact: true }).click();
  await expect(panel.getByRole("link", { name: "Tia Teen" })).toBeVisible();
  await expect(panel.getByRole("alert")).toHaveCount(0);
  await expect(link).toHaveValue("");

  // Teams: an open team is refused (anyone could join it); an invite-only one is fine.
  const teams = panel.getByRole("combobox", { name: "One of your teams" });
  await teams.selectOption({ label: "Open Crew" });
  await panel.getByRole("button", { name: "Share with the team" }).click();
  await expect(panel.getByRole("alert")).toContainText("only shared with invite-only teams");
  await teams.selectOption({ label: "Night Owls" });
  await panel.getByRole("button", { name: "Share with the team" }).click();
  await expect(panel.getByRole("link", { name: "👥 Night Owls" })).toBeVisible();
  await expect(teams.locator("option", { hasText: "Night Owls" })).toHaveCount(0);

  // Pat sees it, told who shared it, without the share panel; and in the library.
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${secret}`);
  await expect(pat.getByText("Bo Builder shared this with you. Only the people it's shared with can see it.")).toBeVisible();
  await expect(pat.locator(".share-panel")).toHaveCount(0);
  await pat.goto("/library");
  await expect(pat.getByRole("heading", { name: "Private Slop" })).toBeVisible();
  await pat.getByRole("navigation", { name: "Your apps" }).getByRole("link", { name: "Secret Sloop" }).click();
  const shared = pat.getByRole("region", { name: "Secret Sloop" });
  await expect(shared.getByText("PRIVATE", { exact: true })).toBeVisible();
  await expect(shared.getByText("by Bo Builder")).toBeVisible();

  // Stop sharing with Tia: she can't open it any more.
  await panel.locator("li").filter({ hasText: "Tia Teen" }).getByRole("button", { name: "Stop sharing" }).click();
  await expect(panel.getByRole("link", { name: "Tia Teen" })).toHaveCount(0);
  const teen = await browser.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${secret}`);
  await expect(teen.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
  // The team share goes the same way.
  await panel.locator("li").filter({ hasText: "Night Owls" }).getByRole("button", { name: "Stop sharing" }).click();
  await expect(panel.getByRole("link", { name: "👥 Night Owls" })).toHaveCount(0);
  await expect(teams.locator("option", { hasText: "Night Owls" })).toHaveCount(1);

  // Bo's own library lists it as his.
  await bo.goto(`/library/${secret}`);
  await expect(bo.getByRole("region", { name: "Secret Sloop" }).getByText("by you")).toBeVisible();
});
