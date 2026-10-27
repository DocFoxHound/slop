// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 1: the dashboard, a new store page (errors, ownership, licence, private), editing
// it, its pictures, and a freeze when SLAPorSLOP can't read its repository.
import { expect, server, signIn, test } from "./fixtures";
import { answer, apiError, call, fakeRepo, readNotices, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

// Mia's store page for this file (a GitHub repository, so pictures can be read from it).
const slug = uniq("pub-lamp");
const title = `Lamp Lighter ${slug.slice(-4)}`;

test("signed out, /publish explains publishing and asks to sign in", async ({ page }) => {
  await page.goto("/publish");
  await expect(page.getByRole("heading", { name: "Publish on SLAPorSLOP", level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: "Sign in to start" })).toHaveAttribute("href", "/signin?return_to=%2Fpublish");
  await page.getByRole("link", { name: "How to publish" }).click();
  await expect(page).toHaveURL(/\/publish\/guide$/);
  // The editor asks the same.
  await page.goto("/publish/new");
  await expect(page.getByRole("link", { name: "Sign in to start" })).toHaveAttribute(
    "href",
    "/signin?return_to=%2Fpublish%2Fnew",
  );
});

test("someone who hasn't published sees where to start", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/publish");
  await expect(page.getByRole("heading", { name: "Your apps", level: 1 })).toBeVisible();
  await expect(page.getByText("You haven't made anything yet.")).toBeVisible();
  await page.getByRole("link", { name: "Create your first draft" }).click();
  await expect(page.getByRole("heading", { name: "New app", level: 1 })).toBeVisible();
});

test("the dashboard says where each app is, and checks again while one is moving", async ({ page }) => {
  await page.clock.install();
  await signIn(page, "maker2");
  let loads = 0;
  page.on("request", (r) => {
    if (r.url().endsWith("/v1/me/apps")) loads++;
  });
  await page.goto("/publish");
  const row = (name: string) => page.locator("li.status-row").filter({ hasText: name });
  await expect(row("Delisted Dash")).toContainText("Stuck at: In the store");
  await expect(row("Delisted Dash")).toContainText("Taken out of the store by SLAPorSLOP staff.");
  // Pocket Golf's new release is still being looked at.
  await expect(row("Pocket Golf")).toContainText(/Waiting on: /);
  await expect(row("Pocket Golf")).toContainText("v1.1.0");
  await expect(row("Pocket Golf").getByRole("link", { name: "Manage" })).toHaveAttribute("href", "/publish/pocket-golf/manage");
  await expect(row("Pocket Golf").getByRole("link", { name: "Edit page" })).toHaveAttribute("href", "/publish/pocket-golf");
  await expect(row("Pocket Golf").getByRole("link", { name: "Releases & progress" })).toHaveAttribute(
    "href",
    "/publish/pocket-golf/releases",
  );
  const before = loads;
  expect(before).toBeGreaterThan(0);
  // A review is awaited: every minute.
  await page.clock.fastForward(61_000);
  await expect.poll(() => loads).toBeGreaterThan(before);
});

test("the new app form: the address follows the name, errors show where they belong", async ({ page }) => {
  // Every attempt here is refused, so Fin has the same apps at the end as at the start (the admin
  // tests give Fin some of their own, so that may be none or a few).
  const mine = async () => (await call<Array<{ slug: string }>>("finance", "GET", "/v1/me/apps")).map((a) => a.slug).sort();
  const before = await mine();
  await signIn(page, "finance");
  await page.goto("/publish/new");
  const name = page.getByLabel("Name", { exact: true });
  const address = page.getByRole("textbox", { name: /^Store address/ });
  await name.fill("Night Ferry: Deluxe!");
  await expect(address).toHaveValue("night-ferry-deluxe");
  await address.fill("night-ferry");
  await name.fill("Night Ferry");
  // Typed by hand, the address stays.
  await expect(address).toHaveValue("night-ferry");
  await page.getByLabel("One-line summary").fill("A ferry at night.");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`https://github.com/e2e/${uniq("ferry")}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("__other");
  await page.getByLabel("Other licence (SPDX ID)").fill("MIT; GPL");
  const create = page.getByRole("button", { name: "Create draft" });

  await create.click();
  await expect(page.locator(".license-picker").getByRole("alert")).toHaveText(
    "use an SPDX licence ID, such as MIT or GPL-3.0-only",
  );
  await page.getByLabel("Other licence (SPDX ID)").fill("MIT");

  // No platform ticked.
  await create.click();
  const platforms = page.getByRole("group", { name: "Runs on" });
  await expect(platforms.getByRole("alert")).toHaveText("choose Windows, Linux or both");
  await platforms.getByRole("checkbox", { name: /Linux/ }).check();
  // Linux comes with a package; .deb is ticked for it, and one must stay.
  await expect(platforms.getByRole("checkbox", { name: /\.deb/ })).toBeChecked();
  await platforms.getByRole("checkbox", { name: /\.deb/ }).uncheck();
  await expect(platforms.getByText("Pick .deb, .rpm or both.")).toBeVisible();
  await platforms.getByRole("checkbox", { name: /\.rpm/ }).check();
  await expect(platforms.getByText("Pick .deb, .rpm or both.")).toBeHidden();

  // Servers that aren't server names.
  const online = page.getByRole("group", { name: "Online" });
  await online.getByRole("button", { name: "Connects to the maker's servers or database" }).click();
  await online.getByRole("textbox", { name: /Your servers/ }).fill("not a host!!");
  await create.click();
  await expect(online.getByRole("alert")).toBeVisible();
  await online.getByRole("textbox", { name: /Your servers/ }).fill("play.example.com:7777");

  // Ten tags at most: the rest can't be picked.
  const tags = page.getByRole("group", { name: /^Tags/ });
  for (const tag of ["Card Game", "Idle", "Platformer", "Puzzle", "Racing", "Roguelike", "Simulation", "Strategy", "Tycoon", "Cozy"]) {
    await tags.getByRole("button", { name: tag, exact: true }).click();
  }
  await expect(tags.getByText("(10/10)")).toBeVisible();
  await expect(tags.getByRole("button", { name: "Retro" })).toBeDisabled();
  await tags.getByRole("button", { name: "Cozy" }).click();
  await expect(tags.getByRole("button", { name: "Retro" })).toBeEnabled();

  // An address that's taken: the form says so as it's typed and offers a free one; sending it
  // anyway names the free one too (makers' agents get the same answer).
  const addressLabel = page.locator("label").filter({ hasText: "Store address" });
  await address.fill("lantern-keep");
  await expect(addressLabel.getByRole("status")).toContainText("Taken.");
  const use = addressLabel.getByRole("button", { name: /^Use lantern-keep-\d+$/ });
  await expect(use).toBeVisible();
  await create.click();
  await expect(addressLabel.getByRole("alert")).toHaveText(/^that address is taken; lantern-keep-\d+ is free$/);
  const free = (await use.textContent())!.replace("Use ", "");
  await use.click();
  await expect(address).toHaveValue(free);
  await expect(addressLabel.getByRole("status")).toHaveText(`✓ slop/app/${free} is free.`);

  // A repository that's already on SLAPorSLOP: the form says so before sending, and the API refuses.
  await address.fill(uniq("ferry"));
  await page.getByRole("textbox", { name: /Source repository/ }).fill("https://github.com/e2e/lantern-keep");
  await expect(page.getByRole("status").filter({ hasText: "already on SLAPorSLOP as Lantern Keep" })).toBeVisible();
  await create.click();
  await expect(page.locator("form > p.form-error")).toHaveText(
    "that repository is already on SLAPorSLOP as Lantern Keep. To build on it, fork it and register a branch",
  );

  // Private from a private repository elsewhere: a token the host refuses shows by its
  // field (it used to show nowhere).
  await page.getByRole("radio", { name: /Private/ }).check();
  await page.getByRole("textbox", { name: /Source repository/ }).fill(fakeRepo("fin", "ledger"));
  const token = page.getByRole("group", { name: "Private repository on another host" });
  await token.getByLabel("Read-only token").fill("fin-token");
  await create.click();
  await expect(token.getByRole("alert")).toHaveText("the host can't find the repository with this token");

  // Cancel leaves without saving.
  await page.getByRole("link", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/publish$/);
  await expect(page.getByRole("heading", { name: "Your apps", level: 1 })).toBeVisible();
  await expect(page.locator("li.status-row").filter({ hasText: "Night Ferry" })).toHaveCount(0);
  expect(await mine()).toEqual(before);
  // Checking the address started Fin's passkey deadline (the first time anyone publishes).
  await readNotices("finance", "/account#sign-ins");
});

test("the agreement gate stops the draft until it's accepted, and keeps what was typed", async ({ page }) => {
  await signIn(page, "finance");
  let attempts = 0;
  await page.route("**/v1/apps", (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    attempts++;
    // First the agreement; then (accepted) a refusal the test can see, so nothing is made.
    return attempts === 1
      ? route.fulfill({ status: 409, json: apiError("terms_required", "accept the current Publisher Agreement first", "publisher") })
      : route.fulfill({ status: 400, json: apiError("invalid", "choose Windows, Linux or both", "platforms") });
  });
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill("Gate Keeper");
  await page.getByLabel("One-line summary").fill("Behind the gate.");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`https://github.com/e2e/${uniq("gate")}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("button", { name: "Create draft" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: /Before you continue: the Publisher Agreement/ })).toBeVisible();
  await dialog.getByRole("button", { name: "Not now" }).click();
  await expect(dialog).toBeHidden();
  // Declined, the form says what's missing (it used to show nothing).
  await expect(page.locator("form > p.form-error")).toHaveText("accept the current Publisher Agreement first");
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Gate Keeper");
  expect(attempts).toBe(1);

  attempts = 0;
  await page.getByRole("button", { name: "Create draft" }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole("checkbox", { name: /I've read and accept/ }).check();
  await dialog.getByRole("button", { name: "Accept and continue" }).click();
  // Accepted, the draft is sent again by itself.
  await expect(page.getByRole("group", { name: "Runs on" }).getByRole("alert")).toHaveText("choose Windows, Linux or both");
  expect(attempts).toBe(2);
});

test("the repository: whose it is, how to prove it, and a token for a private one elsewhere", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await signIn(page, "finance");
  await page.goto("/publish/new");
  const repo = page.getByRole("textbox", { name: /Source repository/ });
  await repo.fill(`https://github.com/e2e/${uniq("mine")}`);
  await expect(page.getByText("✓ You own this repository.")).toBeVisible();

  // Hosts SLAPorSLOP can't ask: a code for the README, then Check (this server trusts every account,
  // so these answers stand in for a stricter one's).
  const code = "slop-verify-k7q2m9";
  await answer(page, "/v1/repos/ownership", {
    ok: false,
    method: "code",
    link: null,
    code,
    message: `SLAPorSLOP can't ask this host who owns the repository, so prove it: add ${code} to the README`,
  });
  await repo.fill("https://git.example.org/fin/ledger");
  const panel = page.getByRole("status").filter({ hasText: "prove it" });
  await expect(panel).toContainText("SLAPorSLOP can't ask this host");
  await expect(panel.locator("code")).toHaveText(code);
  await panel.getByRole("button", { name: "Copy" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(code);
  // Not found yet.
  await answer(page, "/v1/repos/ownership/check", {
    ok: false,
    method: "code",
    link: null,
    code,
    message: `SLAPorSLOP didn't find ${code} in the README or slop-verify.txt on the default branch yet.`,
  });
  await panel.getByRole("button", { name: "Check" }).click();
  await expect(page.getByRole("status").filter({ hasText: "didn't find" })).toContainText("SLAPorSLOP didn't find");
  await page.unroute("**/v1/repos/ownership/check");
  await answer(page, "/v1/repos/ownership/check", {
    ok: true,
    method: "code",
    link: null,
    code,
    message: "You proved you can change this repository.",
  });
  await page.getByRole("button", { name: "Check" }).click();
  await expect(page.getByText("✓ You proved you can change this repository.")).toBeVisible();

  // The real server has no code to check for this account: its refusal shows.
  await page.unroute("**/v1/repos/ownership/check");
  await repo.fill("https://git.example.org/fin/ledger-2");
  await page.getByRole("button", { name: "Check" }).click();
  await expect(page.locator("p.form-error.small")).toHaveText("ask for a code first");

  // An account on another host can prove it: a button to link it, coming back here.
  await page.unroute("**/v1/repos/ownership");
  await answer(page, "/v1/repos/ownership", {
    ok: false,
    method: "account",
    link: "gitlab",
    code: null,
    message: "link the GitLab account that owns gitlab.com/fin/ledger",
  });
  await repo.fill("https://gitlab.com/fin/ledger");
  await expect(page.getByRole("status").filter({ hasText: "Link the GitLab account" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Link GitLab" })).toHaveAttribute(
    "href",
    "/v1/auth/gitlab/link?return_to=%2Fpublish%2Fnew",
  );
  await page.unroute("**/v1/repos/ownership");

  // Private outside GitHub takes a read-only token; on GitHub, the GitHub App instead.
  const token = page.getByRole("group", { name: "Private repository on another host" });
  await expect(token).toBeHidden();
  await page.getByRole("radio", { name: /Private/ }).check();
  await expect(token).toBeVisible();
  // Guessed from the address.
  await expect(token.getByRole("combobox", { name: "Host" })).toHaveValue("gitlab");
  await repo.fill("https://bitbucket.org/fin/ledger");
  await expect(token.getByRole("combobox", { name: "Host" })).toHaveValue("bitbucket");
  await expect(token).toContainText("Repository settings → Security → Access tokens");
  await repo.fill(`https://github.com/e2e/${uniq("private")}`);
  await expect(token).toBeHidden();
  await page.getByRole("radio", { name: /Everyone/ }).check();
});

test("Mia creates a store page", async ({ page }) => {
  await signIn(page, "maker");
  await page.goto("/publish");
  await page.getByRole("link", { name: "+ New app" }).click();
  await expect(page).toHaveURL(/\/publish\/new$/);
  await page.getByLabel("Name", { exact: true }).fill(title);
  await page.getByRole("textbox", { name: /^Store address/ }).fill(slug);
  await page.getByLabel("One-line summary").fill("Light the lamps along the canal.");
  await page.getByLabel("Description").fill("# Lamp Lighter\n\nWalk the canal at dusk.");
  await page.getByLabel("Type").selectOption("game");
  await page.getByLabel("Stage").selectOption("beta");
  // The licence is looked up in the repository; when the lookup fails, the form still works.
  let answerLookup!: () => void;
  const lookupAnswered = new Promise<void>((resolve) => (answerLookup = resolve));
  await page.route("**/v1/repos/license?*", async (route) => {
    await lookupAnswered;
    await route.fulfill({ status: 502, json: apiError("upstream", "GitHub didn't answer") });
  });
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`https://github.com/e2e/${slug}`);
  const licenceHint = page.locator(".license-picker .hint");
  await expect(licenceHint).toHaveText(/Checking your repository's licence/);
  answerLookup();
  await expect(licenceHint).toHaveText(/Must be an OSI-approved open-source licence/);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("Apache-2.0");
  await page.getByLabel("Age rating").selectOption("7");
  // Horror is a content label here (it's also a Games tag).
  await page.getByRole("group", { name: "Content labels" }).getByRole("button", { name: "Horror", exact: true }).click();
  await page.getByRole("checkbox", { name: /Works in sandbox mode/ }).check();
  await page.getByRole("checkbox", { name: /Take bug reports/ }).uncheck();
  await page.getByRole("checkbox", { name: /Windows/ }).check();
  await page.getByRole("button", { name: "Puzzle", exact: true }).click();
  await page.getByRole("button", { name: "Relaxing", exact: true }).click();
  await page.getByRole("button", { name: "Create draft" }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${slug}$`));
  await expect(page.getByRole("heading", { name: title, level: 1 })).toBeVisible();
  await expect(page.getByText("Only you can see this page.")).toBeVisible();

  // Saved as typed.
  const app = await call<{ stage: string; license_spdx: string; min_age: number; content_labels: string[]; tags: string[]; bug_reports: boolean; sandbox_ready: boolean }>(
    "maker",
    "GET",
    `/v1/apps/${slug}`,
  );
  expect(app).toMatchObject({
    stage: "beta",
    license_spdx: "Apache-2.0",
    min_age: 7,
    content_labels: ["horror"],
    bug_reports: false,
    sandbox_ready: true,
  });
  expect([...app.tags].sort()).toEqual(["puzzle", "relaxing"]);
  // The dashboard lists it, waiting for a release.
  await page.goto("/publish");
  await expect(page.locator("li.status-row").filter({ hasText: title })).toContainText("Next: submit a release");
});

test("editing the store page: the address is fixed, others can't open it, errors show", async ({ browser }) => {
  const mia = await browser.newPage();
  await signIn(mia, "maker");
  await mia.goto(`/publish/${slug}`);
  await expect(mia.getByRole("heading", { name: "Edit store page", level: 1 })).toBeVisible();
  await expect(mia.getByRole("textbox", { name: /^Store address/ })).toBeDisabled();
  await expect(mia.getByText("The address can't change once created.")).toBeVisible();
  // Nobody has it yet, so the repository can still change here.
  await expect(mia.getByRole("textbox", { name: /Source repository/ })).toBeEditable();
  // A game's page says where its mods go.
  await expect(mia.getByRole("group", { name: "Mods" })).toBeVisible();

  const online = mia.getByRole("group", { name: "Online" });
  await online.getByRole("button", { name: "Players can host their own servers" }).click();
  await online.getByRole("textbox", { name: /Your servers/ }).fill("not a host!!");
  await mia.getByRole("button", { name: "Save changes" }).click();
  await expect(online.getByRole("alert")).toBeVisible();
  await online.getByRole("button", { name: "Players can host their own servers" }).click();
  await mia.getByLabel("One-line summary").fill("Light every lamp along the canal before dawn.");
  await mia.getByRole("button", { name: "Save changes" }).click();
  await expect(mia).toHaveURL(new RegExp(`/app/${slug}$`));
  await expect(mia.getByText("Light every lamp along the canal before dawn.").first()).toBeVisible();

  // Cancel goes back to the app's page.
  await mia.goto(`/publish/${slug}`);
  await mia.getByRole("link", { name: "Cancel" }).click();
  await expect(mia).toHaveURL(new RegExp(`/app/${slug}$`));

  // Players have Lantern Keep, so its repository moves only through Manage.
  await mia.goto(`/publish/${server.ids.listedApp}`);
  await expect(mia.getByRole("textbox", { name: /Source repository/ })).not.toBeEditable();
  await expect(mia.getByRole("link", { name: "Request a move from Manage" })).toHaveAttribute(
    "href",
    `/publish/${server.ids.listedApp}/manage`,
  );

  // Someone else's draft looks like a missing page.
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/publish/${slug}`);
  await expect(bo.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
  // A mod's own page has no mods folder.
  await bo.goto(`/publish/${server.ids.mod}`);
  await expect(bo.getByRole("heading", { name: "Edit store page", level: 1 })).toBeVisible();
  await expect(bo.getByRole("group", { name: "Mods" })).toHaveCount(0);
});

test("pictures come from the repository: checked, ordered, turned down", async ({ page }) => {
  // Reading the pictures (made on the fly by the test server, in a debug build) takes a while.
  test.setTimeout(120_000);
  await signIn(page, "maker");
  await page.goto(`/publish/${slug}`);
  const media = page.locator("section.media-editor");
  await expect(media.getByText("No pictures in the store yet.")).toBeVisible();
  await media.getByRole("button", { name: "Check my repository now" }).click();
  await expect(media.getByText("Found new pictures. They're waiting for review.")).toBeVisible({ timeout: 45_000 });
  await expect(media.getByText("WAITING FOR REVIEW", { exact: true })).toBeVisible();
  await expect(media.getByText(/Last read/)).toBeVisible();
  const items = media.locator("ol.gallery-edit > li");
  await expect(items).toHaveCount(3);
  await expect(items.nth(0)).toContainText("shots/one.png");
  await expect(media.getByRole("button", { name: "Move item 1 earlier" })).toBeDisabled();
  await expect(media.getByRole("button", { name: "Move item 3 later" })).toBeDisabled();
  await media.getByRole("button", { name: "Move item 1 later" }).click();
  await expect(items.nth(0)).toContainText("shots/two.png");
  await expect(items.nth(1)).toContainText("shots/one.png");
  await media.getByRole("button", { name: "Move item 3 earlier" }).click();
  await expect(items.nth(1)).toContainText("YouTube video");
  // The order is kept.
  await page.reload();
  await expect(items.nth(0)).toContainText("shots/two.png");
  await expect(items.nth(1)).toContainText("YouTube video");

  await media.getByRole("button", { name: "Check my repository now" }).click();
  await expect(media.getByText("Nothing new: these pictures are already shown or waiting for review.")).toBeVisible({ timeout: 45_000 });

  // What else a check can find (this server's fake GitHub always has pictures).
  const overview = await call<Record<string, unknown>>("maker", "GET", `/v1/apps/${slug}/media`);
  await answer(page, `/v1/apps/${slug}/media/check`, { ...overview, result: "none" }, 200, "POST");
  await media.getByRole("button", { name: "Check my repository now" }).click();
  await expect(media.getByText("There's no .slop/media.json on your default branch yet.")).toBeVisible();
  await page.unroute(`**/v1/apps/${slug}/media/check`);
  await answer(page, `/v1/apps/${slug}/media/check`, { ...overview, result: null, problem: "shots/one.png is 9 MB; the limit is 8 MB" }, 200, "POST");
  await media.getByRole("button", { name: "Check my repository now" }).click();
  await expect(media.getByRole("alert")).toHaveText("shots/one.png is 9 MB; the limit is 8 MB");
  await page.unroute(`**/v1/apps/${slug}/media/check`);
  await answer(page, `/v1/apps/${slug}/media/check`, apiError("too_fast", "Too many repository lookups. Try again in 2 minutes."), 429, "POST");
  await media.getByRole("button", { name: "Check my repository now" }).click();
  await expect(media.getByRole("alert").filter({ hasText: "Too many repository lookups" })).toHaveText(
    "Too many repository lookups. Try again in 2 minutes.",
  );
  await page.unroute(`**/v1/apps/${slug}/media/check`);

  // Staff turn them down; the maker is told why.
  const pending = (overview.pending as { id: string }).id;
  await call("admin", "POST", `/v1/admin/media/${pending}/reject`, { reason: "The second screenshot shows another game." });
  await page.reload();
  await expect(media.getByText("These pictures were turned down:")).toBeVisible();
  await expect(media).toContainText("The second screenshot shows another game.");
  await readNotices("maker", slug);
});

test("SLAPorSLOP can't read the repository: the app freezes, Check now, and it thaws", async ({ page }) => {
  await signIn(page, "maker");
  // The daily check, run now: the fake GitHub doesn't know this repository.
  await call("maker", "POST", `/v1/apps/${slug}/repo-check`);
  await page.goto("/notifications");
  await page.getByRole("button", { name: new RegExp(`${title} is frozen`) }).click();
  // The notice leads to the app's Manage page.
  await expect(page).toHaveURL(new RegExp(`/publish/${slug}/manage$`));
  const frozen = page.getByRole("status").filter({ has: page.getByRole("heading", { name: "Frozen: SLAPorSLOP can't check the repository" }) });
  await expect(frozen).toContainText("SLAPorSLOP can't read the repository: it was deleted, renamed, or SLAPorSLOP's access was removed");
  await expect(frozen).toContainText(/If it's still frozen on .*, it leaves the store\./);
  await frozen.getByRole("button", { name: "Check now" }).click();
  await expect(frozen.getByText("Still can't read it. Check the repository's address and SLAPorSLOP's access.")).toBeVisible();

  // Moved somewhere SLAPorSLOP can read (this server answers Git's discovery request for any path).
  await page.goto(`/publish/${slug}`);
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`${server.url}/mia/${slug}`);
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${slug}$`));
  await page.goto(`/publish/${slug}/manage`);
  await page.getByRole("button", { name: "Check now" }).click();
  await expect(page.getByRole("heading", { name: "Frozen: SLAPorSLOP can't check the repository" })).toBeHidden();
  await page.goto("/notifications");
  await page.getByRole("button", { name: new RegExp(`${title} isn't frozen any more`) }).click();
  await expect(page).toHaveURL(new RegExp(`/publish/${slug}/manage$`));
  await readNotices("maker", slug);
});

test("frozen by staff: the maker sees why, and no Check now", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto(`/publish/${server.ids.frozenApp}/manage`);
  const frozen = page.getByRole("status").filter({ has: page.getByRole("heading", { name: "Frozen by SLAPorSLOP" }) });
  await expect(frozen).toContainText("Checking a report that the last build mines cryptocurrency.");
  await expect(frozen.getByRole("button", { name: "Check now" })).toHaveCount(0);
});
