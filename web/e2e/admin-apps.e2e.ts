// SPDX-License-Identifier: AGPL-3.0-or-later
// Apps in the admin panel: the Apps tab (one line per app with what needs attention, finding,
// filtering, and the ⋯ menu: stage, taking out and putting back, withdrawing a release), an app's record (fixing the store page, freezes, history) and
// donation pauses with the Money tab. Every app here is made for the test (by Fin Finance: see
// admin-helpers.ts), and is out of the store again at the end.
import { closePages, githubRepo, listedApp, MAKER, MAKER_NAME, newApp, newPage, newRelease, ok, seeDecisions, tag } from "./admin-helpers";
import { expect, signIn, test } from "./fixtures";
import { fake } from "./accounts-helpers";

test.afterEach(closePages);

test("the Apps tab: one line per app with what needs attention, filters, and the ⋯ menu's actions", async ({ page, browser }) => {
  const t = tag();
  const listed = { slug: `adm-apps-${t}`, title: `Harbour Lights ${t}` };
  await listedApp(listed);
  // Two drafts that can't go in yet: one with no release, one whose release waits for review.
  await newApp({ slug: `adm-norel-${t}`, title: `No Release ${t}` });
  await newApp({ slug: `adm-waiting-${t}`, title: `Waiting Release ${t}` });
  await newRelease(`adm-waiting-${t}`, "0.9.0");
  // And one staff froze.
  const frozen = { slug: `adm-iced-${t}`, title: `Iced Over ${t}` };
  await newApp({ ...frozen, stage: "coming_soon" });
  await ok("admin", "POST", `/v1/admin/apps/${frozen.slug}/freeze`, { reason: "Waiting for the maker to answer." });

  await signIn(page, "admin");
  await page.goto("/admin?tab=apps");
  await page.getByRole("textbox", { name: "Find", exact: true }).fill(t);
  await page.getByRole("button", { name: "Search" }).click();
  const list = page.getByRole("list", { name: "Apps" });
  const row = (title: string) => list.getByRole("listitem").filter({ has: page.getByRole("link", { name: title }) });
  await expect(list.getByRole("listitem")).toHaveCount(4);
  // One line each: title and address, maker, stage and status, and what needs attention.
  await expect(row(listed.title).getByText("LISTED", { exact: true })).toBeVisible();
  await expect(row(listed.title).getByText(`by ${MAKER_NAME}`)).toBeVisible();
  await expect(row(listed.title).getByText(listed.slug, { exact: true })).toBeVisible();
  await expect(row(listed.title).getByText("Released", { exact: true })).toBeVisible();
  await expect(row(listed.title).getByText("Players get 1.0.0")).toBeVisible();
  await expect(row(`Waiting Release ${t}`).getByText("A release waiting for review")).toBeVisible();
  await expect(row(frozen.title).getByText("Frozen: Waiting for the maker to answer.")).toBeVisible();
  await expect(row(`No Release ${t}`).getByText("No release yet")).toBeVisible();

  // Only the ones needing attention, most urgent first: frozen, then the waiting release.
  const show = page.getByRole("combobox", { name: "Show" });
  await show.selectOption("Needs attention");
  await expect(page.getByRole("combobox", { name: "Sort" })).toHaveValue("urgent");
  await expect(list.getByRole("listitem")).toHaveCount(2);
  await expect(list.getByRole("listitem").first()).toContainText(frozen.title);
  await expect(list.getByRole("listitem").nth(1)).toContainText(`Waiting Release ${t}`);
  await expect(page).toHaveURL(/show=attention/);
  // The status filter.
  await show.selectOption("Listed");
  await expect(row(listed.title)).toBeVisible();
  await expect(row(`No Release ${t}`)).toHaveCount(0);
  await show.selectOption("Draft");
  await expect(row(`No Release ${t}`)).toBeVisible();
  await expect(row(listed.title)).toHaveCount(0);
  await show.selectOption("All apps");
  await expect(list.getByRole("listitem")).toHaveCount(4);

  // The menu: a real menu button, opened from the keyboard, closed with Escape.
  const menuButton = (title: string) => row(title).getByRole("button", { name: `Actions for ${title}` });
  const menu = (title: string) => row(title).getByRole("menu", { name: `Actions for ${title}` });
  const choose = async (title: string, item: string, role: "menuitem" | "menuitemradio" = "menuitem") => {
    await menuButton(title).click();
    await menu(title).getByRole(role, { name: item }).click();
  };
  await expect(menuButton(listed.title)).toHaveAttribute("aria-haspopup", "menu");
  await expect(menuButton(listed.title)).toHaveAttribute("aria-expanded", "false");
  await menuButton(listed.title).focus();
  await page.keyboard.press("Enter");
  await expect(menuButton(listed.title)).toHaveAttribute("aria-expanded", "true");
  await expect(menu(listed.title).getByRole("menuitem", { name: "Open its record" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu(listed.title).getByRole("menuitem", { name: "Store page" })).toBeFocused();
  await page.keyboard.press("End");
  await expect(menu(listed.title).getByRole("menuitem", { name: "Withdraw 1.0.0…" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu(listed.title)).toHaveCount(0);
  await expect(menuButton(listed.title)).toBeFocused();
  await expect(menu(listed.title).getByRole("menuitemradio")).toHaveCount(0);

  // A draft without a release can't go in yet: the menu says why.
  await menuButton(`No Release ${t}`).click();
  const put = menu(`No Release ${t}`).getByRole("menuitem", { name: "Put in the store" });
  await expect(put).toHaveAttribute("aria-disabled", "true");
  await expect(put).toHaveAccessibleDescription(/the maker hasn't submitted a release\. Set the stage to Coming soon/);
  await page.keyboard.press("Escape");
  await menuButton(`Waiting Release ${t}`).click();
  await expect(menu(`Waiting Release ${t}`).getByText(/approve its release under Review queue → New releases first/)).toBeVisible();
  await page.keyboard.press("Escape");

  // A stage that needs an approved release is refused for a draft without one, on its line.
  await choose(`No Release ${t}`, "Beta", "menuitemradio");
  await expect(row(`No Release ${t}`).getByRole("alert")).toHaveText("approve a release first: Beta and Released apps need an approved release");
  // A coming-soon page can be listed without one.
  await choose(`No Release ${t}`, "Coming soon", "menuitemradio");
  await expect(row(`No Release ${t}`).getByText("Coming soon", { exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await menuButton(`No Release ${t}`).click();
  await expect(menu(`No Release ${t}`).getByRole("menuitemradio", { name: "Coming soon" })).toHaveAttribute("aria-checked", "true");
  await expect(menu(`No Release ${t}`).getByRole("menuitem", { name: "Put in the store" })).not.toHaveAttribute("aria-disabled");
  await page.keyboard.press("Escape");

  // The listed app: a new stage, then out of the store, asked "Are you sure?" with a rule and the facts.
  await choose(listed.title, "Beta", "menuitemradio");
  await expect(row(listed.title).getByText("Beta", { exact: true })).toBeVisible();
  await choose(listed.title, "Take out of the store…");
  await expect(row(listed.title).getByText(/Are you sure\? Take “Harbour Lights .*” out of the store/)).toBeVisible();
  await expect(row(listed.title).getByRole("combobox", { name: "The rule it breaks" })).toBeFocused();
  const out = row(listed.title).getByRole("button", { name: "Take out of the store" });
  await expect(out).toBeDisabled();
  // Cancel goes back to the menu button.
  await row(listed.title).getByRole("button", { name: "Cancel" }).click();
  await expect(out).toHaveCount(0);
  await expect(menuButton(listed.title)).toBeFocused();
  await choose(listed.title, "Take out of the store…");
  await row(listed.title).getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Misleading pages" });
  await row(listed.title).getByRole("textbox", { name: /The facts, as the maker will read them/ }).fill("The trailer is from another game.");
  await out.click();
  await expect(row(listed.title).getByText("DELISTED", { exact: true })).toBeVisible();
  await expect(out).toHaveCount(0);
  // Back in, then its release withdrawn.
  await choose(listed.title, "Put in the store");
  await expect(row(listed.title).getByText("LISTED", { exact: true })).toBeVisible();
  await choose(listed.title, "Withdraw 1.0.0…");
  await expect(row(listed.title).getByText(/Are you sure\? Withdraw 1\.0\.0 of/)).toBeVisible();
  await expect(row(listed.title).getByRole("button", { name: "Withdraw 1.0.0" })).toBeDisabled();
  await row(listed.title).getByRole("textbox", { name: "Withdraw 1.0.0: reason" }).fill("The installer drops an unknown service.");
  await row(listed.title).getByRole("button", { name: "Withdraw 1.0.0" }).click();
  await expect(row(listed.title).getByRole("button", { name: "Withdraw 1.0.0" })).toHaveCount(0);
  await menuButton(listed.title).click();
  await expect(menu(listed.title).getByRole("menuitem", { name: /Withdraw/ })).toHaveCount(0);
  // The record, from the menu.
  await menu(listed.title).getByRole("menuitem", { name: "Open its record" }).click();
  await expect(page.getByRole("heading", { name: listed.title, level: 2 })).toBeVisible();
  const releases = await ok(MAKER, "GET", `/v1/apps/${listed.slug}/releases`);
  expect(releases[0].status).toBe("revoked");

  // The maker was told why it was taken out.
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  await maker.goto("/account#decisions");
  await expect(maker.getByRole("heading", { name: `Taken out of the store: App: ${listed.title}` })).toBeVisible();
  await expect(maker.getByText("The trailer is from another game.")).toBeVisible();

  // Out of the store again, and the freeze lifted.
  await ok("admin", "POST", `/v1/admin/apps/${listed.slug}/delist`, { rule: "misleading", reason: "Taken out after the admin browser test." });
  await ok("admin", "POST", `/v1/admin/apps/${frozen.slug}/unfreeze`, { note: "Taken off after the admin browser test." });
  await seeDecisions(MAKER);
});

test("an app's record: fixing the store page, stage, out and back in, withdrawing, and its history", async ({ browser }) => {
  const t = tag();
  const app = { slug: `adm-rec-${t}`, title: `Moss Garden ${t}` };
  await listedApp(app);
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=apps&app=${app.slug}`);
  await expect(admin.getByRole("heading", { name: app.title, level: 2 })).toBeVisible();
  await expect(admin.getByRole("link", { name: MAKER_NAME })).toHaveAttribute("href", /tab=users&user=/);

  // Fixing the store page: only what changed is sent, with a reason the maker reads.
  await admin.getByRole("button", { name: "Fix the store page" }).click();
  const form = admin.locator("form").filter({ has: admin.getByRole("button", { name: "Save and tell the maker" }) });
  await form.getByRole("textbox", { name: "Title" }).fill(`Moss Garden Deluxe ${t}`);
  await form.getByRole("textbox", { name: "Summary" }).fill("A calm garden game with a little peril.");
  await form.getByRole("combobox", { name: "Age rating" }).selectOption("16+");
  await form.getByRole("textbox", { name: /Content labels/ }).fill("violence, gore");
  await expect(form.getByRole("button", { name: "Save and tell the maker" })).toBeDisabled();
  await form.getByRole("textbox", { name: /Why, as the maker will read it/ }).fill("The trailer shows fights the page didn't mention.");
  await form.getByRole("button", { name: "Save and tell the maker" }).click();
  await expect(form.getByRole("alert")).toHaveText("use romance, suggestive, violence, horror, strong_language or gambling_themes");
  await form.getByRole("textbox", { name: /Content labels/ }).fill("violence, horror");
  await form.getByRole("button", { name: "Save and tell the maker" }).click();
  await expect(admin.getByRole("status").filter({ hasText: "Changed:" })).toHaveText(
    "Changed: title, summary, age rating, content labels. The maker has been told.",
  );
  await expect(admin.getByRole("heading", { name: `Moss Garden Deluxe ${t}`, level: 2 })).toBeVisible();
  await expect(admin.getByText("Age 16+ · labels: violence, horror · platforms: windows")).toBeVisible();
  const maker = await newPage(browser);
  await signIn(maker, MAKER);
  await maker.goto("/notifications");
  await expect(maker.getByText(`Slop changed Moss Garden Deluxe ${t}'s store page`)).toBeVisible();

  // The stage, out of the store and back, and a release withdrawn.
  const store = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "In the store" }) });
  await store.getByRole("combobox", { name: "Stage" }).selectOption("Beta");
  await expect(store.getByRole("combobox", { name: "Stage" })).toHaveValue("beta");
  await store.getByRole("button", { name: "Take out of the store" }).click();
  await store.getByRole("combobox", { name: "The rule it breaks" }).selectOption({ label: "Misleading pages" });
  await store.getByRole("textbox", { name: /The facts/ }).fill("The page promised online play.");
  await store.getByRole("button", { name: "Take out of the store" }).click();
  await expect(admin.getByText("DELISTED", { exact: true })).toBeVisible();
  await store.getByRole("button", { name: "Put in the store" }).click();
  await expect(admin.getByText("LISTED", { exact: true })).toBeVisible();
  await store.getByRole("button", { name: "Withdraw 1.0.0" }).click();
  await store.getByRole("textbox", { name: "Withdraw 1.0.0: reason" }).fill("Crashes on start for everyone.");
  await store.getByRole("button", { name: "Withdraw 1.0.0" }).click();
  const releases = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "Releases" }) });
  await expect(releases.getByRole("row", { name: /1\.0\.0 revoked/ })).toBeVisible();

  // The decision and everything staff did are on the record.
  const decisions = admin.locator("section").filter({ has: admin.getByRole("heading", { name: /^Decisions \(1\)/ }) });
  await expect(decisions.getByText(/delist_app under misleading by Ada Admin/)).toBeVisible();
  const history = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "History" }) });
  for (const action of ["app.listing_edited", "app.stage", "app.delist", "app.list", "release.revoke"]) {
    await expect(history.getByText(action, { exact: true }).first()).toBeVisible();
  }

  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/delist`, { rule: "misleading", reason: "Taken out after the admin browser test." });
  await seeDecisions(MAKER);
});

test("freezes: an app SLAPorSLOP can't read is checked again on request; staff freeze and lift", async ({ page }) => {
  const t = tag();
  const app = { slug: `adm-frz-${t}`, title: `Glacier ${t}` };
  // A github.com repository the (fake) GitHub doesn't have: checking it freezes the app.
  await newApp({ ...app, stage: "coming_soon", repo: githubRepo(app.slug) });
  await ok(MAKER, "POST", `/v1/apps/${app.slug}/repo-check`);

  await signIn(page, "admin");
  await page.goto(`/admin?tab=apps&app=${app.slug}`);
  await expect(page.getByText("FROZEN", { exact: true })).toBeVisible();
  await expect(page.getByText(/frozen since .* \(SLAPorSLOP lost access to its repository\)/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Lift the freeze" })).toHaveCount(0);
  // Checking again: the repository still can't be read, so it stays frozen.
  await page.getByRole("button", { name: "Check the repository now" }).click();
  const note = page.getByRole("textbox", { name: "Check the repository now: reason" });
  await expect(page.getByRole("button", { name: "Check the repository now" })).toBeDisabled();
  await note.fill("   ");
  await expect(page.getByRole("button", { name: "Check the repository now" })).toBeDisabled();
  await note.fill("The maker says the repository is public again.");
  await page.getByRole("button", { name: "Check the repository now" }).click();
  const history = page.locator("section").filter({ has: page.getByRole("heading", { name: "History" }) });
  await expect(history.getByText("app.repo_rechecked", { exact: true })).toBeVisible();
  await expect(page.getByText("FROZEN", { exact: true })).toBeVisible();

  // Staff freeze it on top: theirs to lift, which ends the freeze.
  await page.getByRole("button", { name: "Freeze" }).click();
  await expect(page.getByRole("button", { name: "Freeze" })).toBeDisabled();
  await page.getByRole("textbox", { name: "Freeze: reason" }).fill("Waiting for the maker to answer a report.");
  await page.getByRole("button", { name: "Freeze" }).click();
  await expect(page.getByText(/by staff \(Waiting for the maker to answer a report\.\)/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Check the repository now" })).toHaveCount(0);
  await page.getByRole("button", { name: "Lift the freeze" }).click();
  await page.getByRole("textbox", { name: "Lift the freeze: reason" }).fill("They answered.");
  await page.getByRole("button", { name: "Lift the freeze" }).click();
  await expect(page.getByText("FROZEN", { exact: true })).toHaveCount(0);
  await expect(history.getByText("app.frozen", { exact: true })).toBeVisible();
  await expect(history.getByText("app.unfrozen", { exact: true })).toBeVisible();
});

test("donation pauses: turned off with a ground, the maker's request kept, lifted, and the Money tab", async ({ browser }) => {
  const t = tag();
  const app = { slug: `adm-don-${t}`, title: `Tidepool ${t}` };
  await newApp({ ...app, stage: "coming_soon" });
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto(`/admin?tab=apps&app=${app.slug}`);

  // Turned off until fixed, with a ground.
  await admin.getByRole("button", { name: "Pause donations" }).click();
  await admin.getByRole("radio", { name: /Turn off until fixed/ }).check();
  await admin.getByRole("textbox", { name: /What's wrong and what would fix it/ }).fill("The page asks for donations to unlock levels.");
  await admin.getByRole("textbox", { name: /Ground \(optional\)/ }).fill("Content Policy: Pay-to-win");
  await admin.getByRole("button", { name: "Turn off donations" }).click();
  await expect(admin.getByText("TURNED OFF UNTIL FIXED")).toBeVisible();
  await expect(admin.getByText("DONATIONS PAUSED")).toBeVisible();
  await expect(admin.getByText("Ground: Content Policy: Pay-to-win", { exact: true })).toBeVisible();
  // Making it a mere pause would let one person lift it: refused.
  await admin.getByRole("button", { name: "Change the pause" }).click();
  await admin.getByRole("radio", { name: /Pause for review/ }).check();
  await admin.getByRole("button", { name: "Save" }).click();
  await expect(admin.getByRole("alert")).toHaveText("donations to this app are turned off: lifting that is a sensitive action");

  // The maker asks; Home says so, and that some are paused.
  await ok(MAKER, "POST", `/v1/apps/${app.slug}/donations/hold/request`, { message: "Levels no longer need donations." });
  await admin.goto("/admin");
  await expect(admin.getByText("1 apps have donations paused.")).toBeVisible();
  // Staff keep it for now, from the review queue.
  await admin.getByRole("link", { name: /^1 Donation pause requests$/ }).click();
  const request = admin.getByRole("list", { name: "Donation pause requests" }).getByRole("listitem").filter({ hasText: app.title });
  await expect(request.getByText("TURNED OFF")).toBeVisible();
  await request.getByRole("link").click();
  const card = admin.locator(".panel").filter({ has: admin.getByRole("link", { name: app.title }) });
  await expect(card.getByText("MAKER ASKS TO LIFT")).toBeVisible();
  await card.getByRole("textbox", { name: "Your answer to the maker" }).fill("Level 3 still asks for one.");
  await card.getByRole("button", { name: "Not yet: keep it" }).click();
  // Answered, so it leaves the queue; the pause is under Money.
  await expect(admin.getByText("Nothing waiting here.")).toBeVisible();
  await admin.goto("/admin?tab=money");
  await expect(card.getByText("Kept: Level 3 still asks for one.")).toBeVisible();
  await expect(card.getByText("MAKER ASKS TO LIFT")).toHaveCount(0);
  // Then lifted directly, with a note.
  await card.getByRole("button", { name: "Lift the pause" }).click();
  await expect(card.getByRole("button", { name: "Lift it: donations back on" })).toBeDisabled();
  await card.getByRole("textbox", { name: "Why, as the maker will read it" }).fill("Fixed in 1.0.1.");
  await card.getByRole("button", { name: "Lift it: donations back on" }).click();
  await expect(card).toHaveCount(0);
  await expect(admin.getByText("No donation pauses.")).toBeVisible();
  // Lifted ones, on request.
  await admin.getByRole("checkbox", { name: "Show lifted ones too" }).check();
  await expect(card.getByText("LIFTED", { exact: true })).toBeVisible();
  await expect(card.getByText(/Lifted by Ada Admin .* on .*: Fixed in 1\.0\.1\./)).toBeVisible();
  await expect(admin.getByRole("heading", { name: /Donation pauses \(0 in force\)/ })).toBeVisible();

  // The payments overview.
  await expect(
    admin.getByText(
      /Payments are live through Stripe · \d+ Stripe events in the last day · \d+ monthly server donations active · 0 donation pauses in force \(0 waiting for an answer\)/,
    ),
  ).toBeVisible();
  const months = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "The last 12 months" }) });
  await expect(months.getByRole("row").nth(1)).toBeVisible();
  const recent = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "Newest payments" }) });
  await expect(recent.getByRole("cell", { name: /^donation/ }).first()).toBeVisible();
});

test("a sensitive action asks for no recent sign-in and offers no step-up", async ({ page }) => {
  // The page's own session is trusted (SECURITY 4.3): the action goes straight through, so it's
  // played here with an answer and nothing changes on the server.
  const withdrawn = await fake(page, "POST", /^\/v1\/admin\/releases\/[^/]+\/revoke$/, { status: 200, body: {} });
  await signIn(page, "admin");
  await page.goto("/admin?tab=apps&app=lantern-keep");
  const store = page.locator("section").filter({ has: page.getByRole("heading", { name: "In the store" }) });
  await store.getByRole("button", { name: "Withdraw 1.0.0" }).click();
  await store.getByRole("textbox", { name: "Withdraw 1.0.0: reason" }).fill("Testing that no sign-in is asked.");
  await store.getByRole("button", { name: "Withdraw 1.0.0" }).click();
  await expect.poll(() => withdrawn.calls.length).toBe(1);
  await expect(store.getByRole("button", { name: "Confirm with a passkey" })).toHaveCount(0);
  await expect(store.getByText(/needs a sign-in from the last few minutes/)).toHaveCount(0);
  await expect(page.getByText("LISTED", { exact: true })).toBeVisible();
});
