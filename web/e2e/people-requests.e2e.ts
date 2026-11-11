// SPDX-License-Identifier: AGPL-3.0-or-later
// Open requests on People (/people?view=requests; web/src/pages/PeopleRequests.tsx,
// crates/slop-api/src/people/work_requests.rs): the board, signed out too; Rhea posts; Vera puts
// her name forward and takes it back; Rhea is told, sees Vera's card and opens Vera's hire form
// filled in from the request (never sent here); who can't put their name forward; the cap of
// three; a prepared request from an AI assistant; reporting, hiding and an appeal.
//
// Every test ends with Rhea holding only the seeded request open (afterEach checks it): what a test
// posts has a title of its own (CI retries post again) and is closed in its own `finally`. Nobody
// puts their name forward on the seeded request, no hire request is sent, and nothing here touches
// Cleo, her app, Vera's jobs or Vera's page. Bea only reads (no report is ever hers); the report
// comes from a signed-out visitor. At the end Vera's names are taken back, and the notices these
// requests sent Rhea are removed.
import AxeBuilder from "@axe-core/playwright";
import type { Browser, Locator, Page } from "@playwright/test";

import { openItem } from "./admin-helpers";
import { expect, ids, signIn, test, type Who } from "./fixtures";
import { call, inDays } from "./people-helpers";

test.describe.configure({ mode: "serial" });

/** What this file posted (to take Vera's names back) and its titles (to find the notices). */
const made: string[] = [];
const titles: string[] = [];

/** A title no other run uses. */
const named = (what: string) => `${what} ${Date.now().toString(36)}${Math.floor(Math.random() * 1296).toString(36)}`;
const requestPath = (id: string) => `/people?view=requests&request=${id}`;
const fillHash = (fields: unknown) => `#fill=${encodeURIComponent(JSON.stringify(fields))}`;
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Rhea posts a request through the API, as the form would. */
async function post(title: string, skill: string, text = "Made by the open requests browser tests."): Promise<string> {
  titles.push(title);
  const r = await call<{ id: string }>("requester", "POST", "/v1/work-requests", {
    app_slug: ids.requesterApp,
    skill,
    title,
    description_md: text,
    confirm_adult: true,
  });
  made.push(r.id);
  return r.id;
}
const close = (id: string) => call("requester", "POST", `/v1/work-requests/${id}/close`);
const takeBack = (id: string) => call("creative", "DELETE", `/v1/work-requests/${id}/response`);

interface Mine {
  posted: Array<{ id: string; title: string; owner: { status: string } }>;
}
interface Notice {
  id: string;
  category: string;
  title: string;
  link: string;
}

/** Someone else's browser in the same test (closed with it, fixtures.ts). */
async function person(browser: Browser, who: Who | null): Promise<Page> {
  const page = await browser.newPage();
  if (who) await signIn(page, who);
  return page;
}

const board = (page: Page) => page.getByRole("region", { name: /^Open requests/ });
const yours = (page: Page) => page.getByRole("region", { name: "Your requests" });
/** A request's card in a list, by its title. */
const card = (scope: Locator, title: string) =>
  scope.getByRole("listitem").filter({ has: scope.page().getByRole("link", { name: title, exact: true }) });

async function noSeriousProblems(page: Page) {
  const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  const serious = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
  expect(serious, serious.join("\n")).toEqual([]);
}

test.afterEach(async () => {
  const mine = await call<Mine>("requester", "GET", "/v1/me/work-requests");
  expect(mine.posted.filter((r) => r.owner.status === "open").map((r) => r.id)).toEqual([ids.seededRequest]);
});

test.afterAll(async () => {
  for (const id of made) await takeBack(id).catch(() => {});
  const inbox = await call<{ items: Notice[] }>("requester", "GET", "/v1/me/notifications?limit=100");
  for (const n of inbox.items.filter((n) => titles.some((t) => n.title.includes(t)))) {
    await call("requester", "DELETE", `/v1/me/notifications/${n.id}`).catch(() => {});
  }
  await call("requester", "GET", "/v1/me/decisions").catch(() => {});
});

test("the board, signed out: People's second tab, newest first, a skill filter, and a request with no budget", async ({ page }) => {
  await page.goto("/people");
  const tabs = page.getByRole("tablist", { name: "People and open requests" });
  await expect(tabs.getByRole("tab", { name: "People" })).toHaveAttribute("aria-selected", "true");
  await tabs.getByRole("tab", { name: "Open requests" }).click();
  await expect(page).toHaveURL("/people?view=requests");
  await expect(page.getByRole("heading", { name: "Open requests", level: 1 })).toBeAttached();
  await expect(tabs.getByRole("tab", { name: "Open requests" })).toHaveAttribute("aria-selected", "true");

  const seeded = card(board(page), "Rooftop theme");
  await expect(seeded).toContainText("Music · for Rhea's Rooftops · posted by Rhea Requester on");
  await expect(seeded.getByRole("link", { name: "Rhea's Rooftops" })).toHaveAttribute("href", `/app/${ids.requesterApp}`);
  await expect(board(page).getByText(/^Newest first\. Nothing else changes the order/)).toBeVisible();
  // It's a tab inside People: no link anywhere on the page goes by that name.
  await expect(page.locator("a").filter({ hasText: /^Open requests$/ })).toHaveCount(0);

  const skills = page.getByRole("navigation", { name: "Open requests" }).getByRole("group", { name: "Skill" });
  await skills.getByRole("button", { name: "Music", exact: true }).click();
  await expect(page).toHaveURL(/[?&]skill=music/);
  await expect(page.getByRole("heading", { name: "Open requests: Music", level: 2 })).toBeVisible();
  await expect(seeded).toBeVisible();
  await skills.getByRole("button", { name: "Pixel Art", exact: true }).click();
  await expect(page).toHaveURL(/[?&]skill=pixel-art/);
  await expect(board(page).getByText("No open requests for Pixel Art right now.")).toBeVisible();
  await skills.getByRole("button", { name: "Every skill" }).click();
  await expect(page).toHaveURL("/people?view=requests");

  await seeded.getByRole("link", { name: "Rooftop theme", exact: true }).click();
  await expect(page).toHaveURL(requestPath(ids.seededRequest));
  const request = page.getByRole("article", { name: "Rooftop theme" });
  await expect(request.getByRole("heading", { name: "Rooftop theme", level: 2 })).toBeFocused();
  await expect(request).toContainText("A calm 60-second loop for the rooftops.");
  await expect(request.getByText("There's no budget on a request: the creative sets the price in their quote.")).toBeVisible();
  await expect(request.getByText("Have a People page? Sign in to put your name forward.")).toBeVisible();
  await expect(request.getByRole("link", { name: "Sign in" })).toHaveAttribute(
    "href",
    `/signin?return_to=${encodeURIComponent(requestPath(ids.seededRequest))}`,
  );
  await expect(request.getByRole("button", { name: "Report" })).toBeVisible();
  await expect(request.getByText(/put their name forward/)).toHaveCount(0);

  // The site's own navigation (top bar, store bar, More menu, footer) has no link to the board.
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 }).first()).toBeAttached();
  await expect(page.locator('a[href*="view=requests"]')).toHaveCount(0);
});

test("Rhea posts a request: no budget, public for 30 days, on the board and under Your requests", async ({ page }) => {
  const title = named("Rooftop tiles");
  titles.push(title);
  try {
    await signIn(page, "requester");
    await page.goto("/people?view=requests");
    await yours(page).getByRole("button", { name: "Post a request" }).click();
    const form = page.getByRole("form", { name: "Post a request" });
    await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue(ids.requesterApp);
    // Project, skill, title, text and date: nothing for money.
    const controls = form.locator("input, select, textarea");
    await expect(controls).toHaveCount(5);
    expect(await controls.evaluateAll((els) => els.map((e) => (e as HTMLInputElement).type))).toEqual([
      "select-one",
      "select-one",
      "text",
      "textarea",
      "date",
    ]);
    await expect(form.getByRole("spinbutton")).toHaveCount(0);
    await expect(form.getByText(/budget|price/i)).toHaveText(["There's no budget on a request: the creative sets the price in their quote."]);
    await expect(
      form.getByText("Anyone can read this request, with your display name and your project. It stays up for 30 days unless you close it sooner."),
    ).toBeVisible();
    // Rhea confirmed she's 18 or older long ago.
    await expect(form.getByRole("checkbox", { name: "I'm 18 or older" })).toHaveCount(0);
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);

    await form.getByRole("combobox", { name: "Skill" }).selectOption("pixel-art");
    await form.getByRole("textbox", { name: "Title" }).fill(title);
    await form.getByRole("textbox", { name: /^What do you need\?/ }).fill("Twelve rooftop tiles, 32×32, in the style of the first level.");
    await form.getByRole("button", { name: "Post request" }).click();
    await expect(yours(page).getByRole("status").filter({ hasText: "Your request is on the board." })).toBeFocused();
    await expect(form).toHaveCount(0);
    await expect(card(board(page), title)).toContainText("Pixel Art · for Rhea's Rooftops · posted by Rhea Requester on");
    await expect(card(board(page), title)).toContainText("YOUR REQUEST");
    const mine = card(yours(page), title);
    await expect(mine).toContainText(/Open until /);
    await expect(mine).toContainText("Nobody has put their name forward yet");
  } finally {
    const mine = await call<Mine>("requester", "GET", "/v1/me/work-requests");
    for (const r of mine.posted.filter((r) => r.title === title)) {
      made.push(r.id);
      await close(r.id);
    }
  }
});

test("Vera puts her name forward, naming a package; only she sees it on the board; Bea sees no button and no count", async ({ page, browser }) => {
  const title = named("Rooftop sprites");
  const id = await post(title, "pixel-art", "Four rooftop cats, 32×32, idle and walk.");
  try {
    await signIn(page, "creative");
    await page.goto(requestPath(id));
    const request = page.getByRole("article", { name: title });
    const form = request.getByRole("form", { name: "Put your name forward" });
    await expect(form.getByText("The maker sees a link to your People page, and the package if you name one. Nothing else is sent, and you can take it back.")).toBeVisible();
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);
    await form.getByRole("combobox", { name: "Name one of your packages (optional)" }).selectOption({ label: "Sprite sheet" });
    await form.getByRole("button", { name: "Put my name forward" }).click();
    const said = request
      .getByRole("status")
      .filter({ hasText: /^You put your name forward on .+, naming your package “Sprite sheet”\. The maker sees a link to your People page\.$/ });
    await expect(said).toBeFocused();
    await expect(request.getByRole("button", { name: "Take my name back" })).toBeVisible();
    await expect(form).toHaveCount(0);

    await page.goto("/people?view=requests");
    await expect(card(board(page), title)).toContainText("YOU PUT YOUR NAME FORWARD");
    await expect(yours(page).getByRole("heading", { name: "Requests you put your name forward for", level: 3 })).toBeVisible();
    await expect(card(yours(page), title)).toBeVisible();

    const bea = await person(browser, "bystander");
    await bea.goto(requestPath(id));
    const seen = bea.getByRole("article", { name: title });
    await expect(seen).toContainText("Four rooftop cats, 32×32, idle and walk.");
    await bea.waitForLoadState("networkidle");
    await expect(seen.getByRole("button", { name: "Put my name forward" })).toHaveCount(0);
    await expect(seen.getByText(/name forward|Your People page/)).toHaveCount(0);
    await bea.goto("/people?view=requests");
    await expect(card(board(bea), title)).toBeVisible();
    await expect(card(board(bea), title)).not.toContainText(/name forward|YOUR REQUEST/i);
  } finally {
    await takeBack(id);
    await close(id);
  }
});

test("Rhea is told once; she sees Vera's card and package; Ask Vera for a quote opens the hire form filled in from the request", async ({ page }) => {
  const title = named("Rooftop chimneys");
  const id = await post(title, "pixel-art", "Six chimneys with smoke, 32×32.");
  try {
    const vera = await call<{ services: Array<{ id: string; name: string }> }>("requester", "GET", `/v1/people/${ids.creativeHandle}`);
    const sprites = vera.services.find((s) => s.name === "Sprite sheet")!;
    await call("creative", "PUT", `/v1/work-requests/${id}/response`, { service_id: sprites.id });
    // Again: only the package can change, and nobody is told twice.
    await call("creative", "PUT", `/v1/work-requests/${id}/response`, { service_id: sprites.id });
    const inbox = await call<{ items: Notice[] }>("requester", "GET", "/v1/me/notifications?category=jobs&limit=100");
    const told = inbox.items.filter((n) => n.link === requestPath(id));
    expect(told.map((n) => n.title)).toEqual([`New names put forward for “${title}”`]);

    await signIn(page, "requester");
    await page.goto("/notifications/jobs");
    await page
      .locator(".notifications li")
      .filter({ hasText: told[0].title })
      .getByRole("button", { name: new RegExp(escape(told[0].title)) })
      .click();
    await expect(page).toHaveURL(requestPath(id));
    const names = page.getByRole("region", { name: "Who put their name forward" });
    await expect(names.getByText("In the order they did. SLAPorSLOP doesn't rank or recommend anyone here.")).toBeVisible();
    const veraItem = names.getByRole("listitem").filter({ hasText: "Vera Okafor" });
    await expect(veraItem.getByRole("link", { name: /Vera Okafor/ })).toHaveAttribute("href", `/people/${ids.creativeHandle}`);
    await expect(veraItem).toContainText("Package: Sprite sheet");
    await expect(page.getByRole("article", { name: title }).getByRole("button", { name: "Report" })).toHaveCount(0);
    await page.waitForLoadState("networkidle");
    await noSeriousProblems(page);

    await veraItem.getByRole("button", { name: "Ask Vera for a quote" }).click();
    await expect(page).toHaveURL(`/people/${ids.creativeHandle}`);
    const form = page.getByRole("form", { name: "Hire Vera Okafor" });
    const notice = page.getByRole("status", { name: "Filled in from your open request" });
    await expect(notice).toContainText(
      "Filled in from your open request. Check everything, especially what you're asking for, word for word, then press Send request. Nothing is sent until you do.",
    );
    await expect(page.getByRole("status", { name: "Filled in by your AI assistant" })).toHaveCount(0);
    expect(await page.evaluate(() => location.hash)).toBe("");
    await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue(ids.requesterApp);
    await expect(form.getByRole("combobox", { name: "Package" })).toHaveValue(sprites.id);
    await expect(form.getByRole("combobox", { name: "Skill" })).toHaveValue("pixel-art");
    await expect(form.getByRole("textbox", { name: /^What do you need, exactly\?/ })).toHaveValue(`${title}\n\nSix chimneys with smoke, 32×32.`);
    await expect(form.getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ })).not.toBeChecked();
    // Leaves without sending: no job is made by the board.
  } finally {
    await takeBack(id);
    await close(id);
  }
});

test("who can't put their name forward: Pip can't be hired yet, Vera doesn't list the skill, Tia and Bea see nothing", async ({ browser }) => {
  const birds = named("Rooftop birds");
  const signs = named("Rooftop signs");
  const a = await post(birds, "pixel-art");
  const b = await post(signs, "writing");
  try {
    // Pip: live, but no payouts.
    const pip = await person(browser, "previewer");
    await pip.goto(requestPath(a));
    const r1 = pip.getByRole("article", { name: birds });
    await expect(r1.getByText(/^Your People page can't be hired right now, so you can't put your name forward yet\./)).toBeVisible();
    await expect(r1.getByRole("link", { name: "See what it needs" })).toHaveAttribute("href", "/people/me");
    await expect(r1.getByRole("button", { name: "Put my name forward" })).toHaveCount(0);

    // Vera does pixel art and music, not writing.
    const vera = await person(browser, "creative");
    await vera.goto(requestPath(b));
    const r2 = vera.getByRole("article", { name: signs });
    await expect(r2.getByText(/^Your People page doesn't list Writing\./)).toBeVisible();
    await expect(r2.getByRole("link", { name: "Add it to your page" })).toHaveAttribute("href", "/people/me");
    await expect(r2.getByRole("button", { name: "Put my name forward" })).toHaveCount(0);

    // No People page: they read it, with nothing to explain.
    for (const who of ["teen", "bystander"] as const) {
      const p = await person(browser, who);
      await p.goto(requestPath(a));
      const r = p.getByRole("article", { name: birds });
      await expect(r.getByRole("button", { name: "Report" })).toBeVisible();
      await p.waitForLoadState("networkidle");
      await expect(r.getByRole("button", { name: "Put my name forward" })).toHaveCount(0);
      await expect(r.getByText(/Your People page|Sign in to put your name forward/)).toHaveCount(0);
    }
  } finally {
    await close(a);
    await close(b);
  }
});

test("Vera takes her name back; Rhea closes the request, and it's gone for everyone else", async ({ page, browser }) => {
  const title = named("Rooftop lanterns");
  const id = await post(title, "pixel-art");
  try {
    await call("creative", "PUT", `/v1/work-requests/${id}/response`, { service_id: null });
    const vera = await person(browser, "creative");
    await vera.goto(requestPath(id));
    const request = vera.getByRole("article", { name: title });
    await expect(request.getByText(/^You put your name forward on /)).toBeVisible();
    await request.getByRole("button", { name: "Take my name back" }).click();
    await expect(request.getByRole("status").filter({ hasText: "You took your name back." })).toBeFocused();
    await expect(request.getByRole("form", { name: "Put your name forward" })).toBeVisible();

    await signIn(page, "requester");
    await page.goto(requestPath(id));
    const names = page.getByRole("region", { name: "Who put their name forward" });
    await expect(names.getByText("Nobody yet. Requests stay up for 30 days.")).toBeVisible();
    await expect(names.getByRole("listitem")).toHaveCount(0);
    await page.getByRole("button", { name: "Close request" }).click();
    await expect(page.getByRole("main").getByRole("status").filter({ hasText: "Request closed." })).toBeFocused();
    await expect(page.getByText(/^You closed this request on /)).toBeVisible();
    await expect(page.getByRole("button", { name: "Close request" })).toHaveCount(0);
    await page.goto("/people?view=requests");
    await expect(card(yours(page), title)).toContainText(/Closed on /);
    await expect(card(board(page), "Rooftop theme")).toBeVisible();
    await expect(card(board(page), title)).toHaveCount(0);

    const bea = await person(browser, "bystander");
    await bea.goto(requestPath(id));
    await expect(bea.getByText("This request isn't open any more.")).toBeVisible();
    await expect(bea.getByRole("link", { name: "See open requests" })).toHaveAttribute("href", "/people?view=requests");
  } finally {
    await takeBack(id);
    await close(id);
  }
});

test("three open at most: at three, one sentence instead of the button, and a prepared request waits", async ({ page }) => {
  const made: string[] = [];
  try {
    // With the seeded one, three.
    made.push(await post(named("Rooftop cap one"), "music"));
    made.push(await post(named("Rooftop cap two"), "music"));
    await signIn(page, "requester");
    await page.goto("/people?view=requests");
    await expect(yours(page).getByText("You have 3 open requests, the most at one time. Close one to post another.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Post a request" })).toHaveCount(0);

    await page.goto(`/people?view=requests${fillHash({ title: named("Rooftop cap three"), skill: "music" })}`);
    await expect(yours(page).getByRole("status").filter({ hasText: "Your AI assistant's request is ready." })).toHaveText(
      "Your AI assistant's request is ready. It opens here, filled in, once you have fewer than 3 open requests. It's kept in this browser tab only, for two hours.",
    );
    await expect(page.getByRole("form", { name: "Post a request" })).toHaveCount(0);
  } finally {
    for (const id of made) await close(id);
  }
});

test("a prepared request opens the form filled in, leaves out the budget and the 18+ tick, and waits through signing in", async ({ page, browser }) => {
  const title = named("Rooftop handoff");
  const fields = {
    app_slug: ids.requesterApp,
    skill: "music",
    title,
    description_md: "A short loop for the rooftops at night.",
    wanted_by: inDays(30),
    budget: 500,
    confirm_adult: true,
  };
  await signIn(page, "requester");
  await page.goto(`/people?view=requests${fillHash(fields)}`);
  const form = page.getByRole("form", { name: "Post a request" });
  const notice = form.getByRole("status", { name: "Filled in by your AI assistant" });
  await expect(notice).toContainText(
    "Check everything, especially the title and what you need, since both are public, then press Post request. Nothing is sent until you do.",
  );
  await expect(notice.locator(".handoff-filled")).toHaveText("Filled in: Project, Skill, Title, What you need and Hoping to have it by.");
  await expect(notice.getByRole("listitem")).toHaveText([
    "Budget: there's no budget on a request: the creative sets the price in their quote",
    "I'm 18 or older: only you can say you're 18 or older",
  ]);
  expect(await page.evaluate(() => location.hash)).toBe("");
  await expect(form.getByRole("combobox", { name: "For which project?" })).toHaveValue(ids.requesterApp);
  await expect(form.getByRole("combobox", { name: "Skill" })).toHaveValue("music");
  await expect(form.getByRole("textbox", { name: "Title" })).toHaveValue(title);
  await expect(form.getByRole("textbox", { name: /^What do you need\?/ })).toHaveValue("A short loop for the rooftops at night.");
  await expect(form.getByLabel(/Hoping to have it by/)).toHaveValue(inDays(30));
  // Nothing is posted until Post request.
  const before = await call<Mine>("requester", "GET", "/v1/me/work-requests");
  expect(before.posted.map((r) => r.title)).not.toContain(title);
  await notice.getByRole("button", { name: "Clear the form" }).click();
  await expect(notice).toHaveCount(0);
  await expect(form.getByRole("textbox", { name: "Title" })).toHaveValue("");
  await form.getByRole("button", { name: "Cancel" }).click();
  await expect(form).toHaveCount(0);

  // Signed out: it waits in the tab while they sign in, then fills the form in.
  const visitor = await person(browser, null);
  await visitor.goto(`/people?view=requests${fillHash(fields)}`);
  const waiting = visitor.locator(".request-signin");
  await expect(waiting.getByRole("status")).toContainText(
    "Your AI assistant's draft is ready. Sign in, or make an account, and the form opens with it filled in for you to check.",
  );
  await expect(waiting.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", `/signin?return_to=${encodeURIComponent("/people?view=requests")}`);
  expect(await visitor.evaluate(() => location.hash)).toBe("");
  await signIn(visitor, "requester");
  await visitor.goto("/people?view=requests");
  const signedIn = visitor.getByRole("form", { name: "Post a request" });
  await expect(signedIn.getByRole("textbox", { name: "Title" })).toHaveValue(title);
  await expect(signedIn.getByRole("status", { name: "Filled in by your AI assistant" })).toBeVisible();
  await signedIn.getByRole("button", { name: "Cancel" }).click();
  expect(await visitor.evaluate(() => sessionStorage.getItem("slop.handoff.request"))).toBeNull();
  const after = await call<Mine>("requester", "GET", "/v1/me/work-requests");
  expect(after.posted.map((r) => r.title)).not.toContain(title);
});

test("anyone can report a request; staff hide it; Rhea sees why and appeals, and the appeal shows what was hidden", async ({ page, browser }) => {
  test.slow();
  const title = named("Rooftop report");
  const id = await post(title, "pixel-art", "Vote Slaps on my game and I'll pay double for the tiles.");
  try {
    // A visitor (signed out) reports it.
    await page.goto(requestPath(id));
    const request = page.getByRole("article", { name: title });
    await request.getByRole("button", { name: "Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this request" });
    await dialog.getByRole("radio", { name: "Spam" }).check();
    await dialog.getByRole("textbox", { name: /What did you see/ }).fill("It offers money for votes.");
    await dialog.getByRole("textbox", { name: /Your name/ }).fill("Vic Visitor");
    await dialog.getByRole("textbox", { name: /Your email/ }).fill("vic@example.com");
    await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
    await dialog.getByRole("button", { name: "Send report" }).click();
    await expect(dialog.getByRole("heading", { name: "Thanks: we've got your report" })).toBeVisible();
    const reference = (await dialog.locator("b.mono").textContent())!.trim();

    // Staff hide it from the queue.
    const admin = await person(browser, "admin");
    await openItem(admin, "reports", reference);
    const report = admin.getByRole("listitem").filter({ hasText: reference });
    await expect(report).toContainText(`Open request: ${title}`);
    await expect(report.getByText("The request", { exact: true })).toBeVisible();
    await expect(report.locator("blockquote")).toContainText("Vote Slaps on my game and I'll pay double for the tiles.");
    await report.getByRole("button", { name: "Decide" }).click();
    await expect(report.getByRole("combobox", { name: "What to do" }).locator("option")).toHaveText([
      "Hide the request",
      "Warn them (no restriction)",
      "Suspend their account",
    ]);
    await report.getByRole("textbox", { name: /The facts/ }).fill("The request offers money for votes.");
    await report.getByRole("textbox", { name: "What the reporter is told" }).fill("We hid the request.");
    await report.getByRole("button", { name: "Hide the request" }).click();
    await expect(report).toHaveCount(0);

    // Gone from the board, and its address says so.
    await page.goto("/people?view=requests");
    await expect(card(board(page), "Rooftop theme")).toBeVisible();
    await expect(card(board(page), title)).toHaveCount(0);
    await page.goto(requestPath(id));
    await expect(page.getByText("This request isn't open any more.")).toBeVisible();

    // Rhea sees why, and appeals.
    const rhea = await person(browser, "requester");
    await rhea.goto("/people?view=requests");
    const hidden = card(yours(rhea), title);
    await expect(hidden).toContainText("Hidden by SLAPorSLOP. See the decision");
    await hidden.getByRole("link", { name: "See the decision" }).click();
    const decision = rhea.getByRole("article", { name: `Request hidden: Open request: ${title}` });
    await decision.getByRole("button", { name: "Appeal this decision" }).click();
    await decision.getByRole("textbox", { name: /Why should we look again/ }).fill("It was a joke.");
    await decision.getByRole("button", { name: "Send appeal" }).click();
    await expect(decision.getByText(/You appealed on/)).toBeVisible();

    // The person deciding reads what was hidden (its page is gone for them too), and keeps it.
    await openItem(admin, "appeals", `Open request: ${title}`);
    const appeal = admin
      .getByRole("listitem")
      .filter({ hasText: `Open request: ${title}` })
      .filter({ has: admin.getByRole("button", { name: "Reverse the decision" }) });
    await expect(appeal.getByText("What was hidden", { exact: true })).toBeVisible();
    await expect(appeal.locator("blockquote")).toContainText(title);
    await expect(appeal.locator("blockquote")).toContainText("Vote Slaps on my game and I'll pay double for the tiles.");
    await appeal.getByRole("textbox", { name: /Why \(the person reads this\)/ }).fill("Paying for votes isn't allowed.");
    await appeal.getByRole("button", { name: "Keep it" }).click();
    await expect(appeal).toHaveCount(0);
  } finally {
    await close(id);
  }
});
