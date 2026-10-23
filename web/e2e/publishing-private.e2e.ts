// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 4: Private Slop. Bo makes a private app from a private repository on another
// host (with a read-only token), manages the token, asks to make it public, moves the repository,
// shares it with friends, a profile link and a team, and triages the bug reports players send.
// Pat and Cleo play it while it's shared with them; at the end it's shared with nobody again.
import { expect, ids, server, signIn, test } from "./fixtures";
import { answer, apiError, approve, call, fakeRepo, readNotices, submitRelease, uniq } from "./publishing-helpers";

test.describe.configure({ mode: "serial" });

const slug = uniq("pub-den");
const title = `Secret Den ${slug.slice(-4)}`;
// This server answers any path, so Slop's checks of a "repository" on it pass: the token check
// (the host's API), and whether the source is public (Git's discovery request).
const readable = `${server.url}/bo/${slug}`;
const fake = fakeRepo("bo", slug);
const manage = `/publish/${slug}/manage`;

test.afterAll(async () => {
  for (const who of ["maker2", "player", "client"] as const) await readNotices(who, slug);
});

test("Bo makes a Private Slop app from a private repository elsewhere, with a token", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill(title);
  await page.getByRole("textbox", { name: /^Store address/ }).fill(slug);
  await page.getByLabel("One-line summary").fill("Only for friends, for now.");
  await page.getByRole("radio", { name: /Private Slop/ }).check();
  await page.getByLabel("Stage").selectOption("released");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(readable);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("checkbox", { name: /Windows/ }).check();
  const token = page.getByRole("group", { name: "Private repository on another host" });
  await expect(token.getByRole("combobox", { name: "Host" })).toHaveValue("gitea");
  await token.getByLabel("Read-only token").fill("den-token-1");
  await page.getByRole("button", { name: "Create draft" }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${slug}$`));
  await expect(page.getByText("PRIVATE SLOP")).toBeVisible();
  await expect(page.getByText("Only you and the people you share it with can see this. It's never in the store.")).toBeVisible();
  // Never in the store, not even for its owner.
  expect(JSON.stringify(await call("maker2", "GET", `/v1/apps?q=${encodeURIComponent(title)}`))).not.toContain(slug);
});

test("the repository token on Manage: replace, refuse, remove, save", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto(manage);
  const panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "Private repository access" }) });
  await expect(panel).toContainText(/A Gitea, Forgejo or Codeberg token is saved \(since .*\)\. Replace it before it expires\./);
  const field = panel.getByLabel("Read-only token");
  await expect(panel.getByRole("button", { name: "Replace token" })).toBeDisabled();

  await panel.getByRole("combobox", { name: "Host" }).selectOption("gitlab");
  await expect(panel).toContainText("Role Reporter, scopes read_api and read_repository.");
  await field.fill("glpat-den-2");
  await panel.getByRole("button", { name: "Replace token" }).click();
  await expect(panel).toContainText(/A GitLab token is saved/);
  // Never shown again.
  await expect(field).toHaveValue("");

  await field.fill("two words");
  await panel.getByRole("button", { name: "Replace token" }).click();
  await expect(panel.getByRole("alert")).toHaveText("paste the token on its own");

  await panel.getByRole("button", { name: "Remove token" }).click();
  await expect(panel).toContainText("Slop needs a read-only token to fetch releases from this private repository and share them.");
  await expect(panel.getByRole("button", { name: "Remove token" })).toHaveCount(0);
  await panel.getByRole("combobox", { name: "Host" }).selectOption("gitea");
  await field.fill("den-token-3");
  await panel.getByRole("button", { name: "Save token" }).click();
  await expect(panel).toContainText(/A Gitea, Forgejo or Codeberg token is saved/);
  expect(await call("maker2", "GET", `/v1/apps/${slug}/repo-token`)).toMatchObject({ saved: true, kind: "gitea" });

  // A server with no key for tokens says so; a GitHub repository uses the GitHub App instead.
  await answer(page, `/v1/apps/${slug}/repo-token`, { saved: false, kind: null, saved_at: null, last_used_at: null, available: false, github: false });
  await page.reload();
  await expect(panel.getByText("This server can't keep repository tokens yet.")).toBeVisible();
  await page.unroute(`**/v1/apps/${slug}/repo-token`);
  await answer(page, `/v1/apps/${slug}/repo-token`, { saved: false, kind: null, saved_at: null, last_used_at: null, available: true, github: true });
  await page.reload();
  await expect(page.getByRole("heading", { name: "Make it public" })).toBeVisible();
  await expect(panel).toHaveCount(0);
});

test("asking to make it public, and keeping it private after all", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto(manage);
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "Make it public" }) });
  // Private apps have no donations or ratings.
  await expect(page.getByRole("heading", { name: "Ratings" })).toHaveCount(0);
  await section.getByRole("button", { name: "Ask to make it public" }).click();
  await expect(section.getByText(/^Waiting for a reviewer\./)).toBeVisible();
  await section.getByRole("button", { name: "Keep it private" }).click();
  await expect(section.getByRole("button", { name: "Ask to make it public" })).toBeVisible();
});

test("nobody has it yet, so the repository moves straight away; then it can't go public", async ({ page }) => {
  await signIn(page, "maker2");
  await page.goto(manage);
  const repo = page.locator("section").filter({ has: page.getByRole("heading", { name: "Repository", exact: true }) });
  await expect(repo.getByText(/No one has this app yet, so you can change the repository/)).toBeVisible();
  await expect(repo.getByRole("link", { name: "store page" })).toHaveAttribute("href", `/publish/${slug}`);
  // No reason needed when nobody has it.
  await expect(repo.getByRole("textbox", { name: /Why/ })).toHaveCount(0);
  await repo.getByRole("textbox", { name: "Move to" }).fill(fake);
  await repo.getByRole("button", { name: "Move", exact: true }).click();
  await expect(repo.getByText(/^Done\./)).toBeVisible();
  await expect(repo.getByRole("link", { name: fake })).toBeVisible();
  await expect(page.locator("ul.release-list > li").filter({ hasText: `Move to ${fake}` })).toContainText("DONE");

  // Its source can't be read without signing in now.
  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: "Make it public" }) });
  await section.getByRole("button", { name: "Ask to make it public" }).click();
  await expect(section.getByRole("alert")).toHaveText(
    "make the repository public first: the repository can't be read without signing in: it was deleted, made private, or moved",
  );
});

test("sharing: with a friend, a profile link and an invite-only team, not an open one", async ({ page }) => {
  await approve(await submitRelease("maker2", slug, fake, "1.0.0"));
  await signIn(page, "maker2");
  await page.goto(`/app/${slug}`);
  const panel = page.locator(".share-panel");
  await expect(panel.getByText("Nobody yet.")).toBeVisible();
  await panel.getByRole("combobox", { name: "One of your friends" }).selectOption({ label: "Pat Player" });
  await panel.getByRole("button", { name: "Share with them" }).click();
  await expect(panel.getByRole("link", { name: "Pat Player" })).toHaveAttribute("href", `/u/${ids.player}`);
  // Pat is shared with now, so no longer offered.
  await expect(panel.getByRole("combobox", { name: "One of your friends" })).toHaveCount(0);

  await panel.getByRole("textbox", { name: "Their profile link" }).fill("not a profile");
  await panel.getByRole("button", { name: "Share", exact: true }).click();
  await expect(panel.getByRole("alert")).toBeVisible();
  await panel.getByRole("textbox", { name: "Their profile link" }).fill(`${server.url}/u/${ids.client}`);
  await panel.getByRole("button", { name: "Share", exact: true }).click();
  await expect(panel.getByRole("link", { name: "Cleo Client" })).toBeVisible();
  await expect(panel.getByRole("textbox", { name: "Their profile link" })).toHaveValue("");

  const teams = panel.getByRole("combobox", { name: "One of your teams" });
  await teams.selectOption({ label: "Open Crew" });
  await panel.getByRole("button", { name: "Share with the team" }).click();
  await expect(panel.getByRole("alert")).toHaveText("Private Slop is only shared with invite-only teams");
  await teams.selectOption({ label: "Night Owls" });
  await panel.getByRole("button", { name: "Share with the team" }).click();
  await expect(panel.getByRole("link", { name: "👥 Night Owls" })).toHaveAttribute("href", `/teams/${ids.inviteTeam}`);
  await panel.getByRole("listitem").filter({ hasText: "Night Owls" }).getByRole("button", { name: "Stop sharing" }).click();
  await expect(panel.getByRole("link", { name: "👥 Night Owls" })).toHaveCount(0);
  await expect(panel.getByRole("listitem")).toHaveCount(2);

  // Someone it isn't shared with can't see it at all.
  const teen = await page.context().browser()!.newPage();
  await signIn(teen, "teen");
  await teen.goto(`/app/${slug}`);
  await expect(teen.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});

test("players it's shared with, once they've played it, report bugs; others say they have it too", async ({ browser }) => {
  // Shared with, but not played: no bug reports.
  const cleo = await browser.newPage();
  await signIn(cleo, "client");
  await cleo.goto(`/app/${slug}`);
  await expect(cleo.getByText("Bo Builder shared this with you. Only the people it's shared with can see it.")).toBeVisible();
  await expect(cleo.getByRole("heading", { name: title, level: 1 })).toBeVisible();
  await expect(cleo.locator("#bug-reports")).toHaveCount(0);

  // Pat plays it (the Slop app tells Slop on Play), then reports.
  await call("player", "POST", `/v1/apps/${slug}/launch?platform=windows`);
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${slug}`);
  const section = pat.locator("#bug-reports");
  await section.getByRole("button", { name: "Report a bug" }).click();
  const send = section.getByRole("button", { name: "Send to the maker" });
  await expect(send).toBeDisabled();
  await section.getByRole("textbox", { name: "What's wrong, in a line" }).fill("The den door won't open");
  await section.getByRole("textbox", { name: /What happened/ }).fill("Press E at the door: nothing happens.");
  // Refused (too many reports at once): the reason shows and the form stays.
  await answer(pat, `/v1/apps/${slug}/bug-reports`, apiError("rate_limited", "too many bug reports for now: try again in a while"), 429, "POST");
  await send.click();
  await expect(section.getByRole("alert")).toHaveText("too many bug reports for now: try again in a while");
  await pat.unroute(`**/v1/apps/${slug}/bug-reports`);
  await send.click();
  await expect(section.getByRole("status")).toHaveText("Sent to the maker. You'll be told when they answer.");
  const mine = section.getByRole("listitem").filter({ hasText: "The den door won't open" });
  await expect(mine).toContainText("OPEN");
  // Your own report has no "me too".
  await expect(mine.getByRole("button", { name: "I have this too" })).toHaveCount(0);

  // Cleo plays it too: she can say she has it too, and take it back.
  await call("client", "POST", `/v1/apps/${slug}/launch?platform=windows`);
  await cleo.reload();
  const theirs = cleo.locator("#bug-reports").getByRole("listitem").filter({ hasText: "The den door won't open" });
  const meToo = theirs.getByRole("button", { name: "I have this too" });
  await expect(meToo).toHaveAttribute("aria-pressed", "false");
  await meToo.click();
  await expect(meToo).toHaveAttribute("aria-pressed", "true");
  await expect(theirs).toContainText("2 players");
  await meToo.click();
  await expect(meToo).toHaveAttribute("aria-pressed", "false");
  await expect(theirs).not.toContainText("2 players");
  await meToo.click();
  await expect(theirs).toContainText("2 players");

  // The maker is pointed to Manage instead of a form.
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/app/${slug}`);
  await expect(bo.locator("#bug-reports").getByRole("link", { name: "Manage page" })).toHaveAttribute("href", `${manage}#bug-reports`);
  await expect(bo.locator("#bug-reports").getByRole("button", { name: "Report a bug" })).toHaveCount(0);
});

test("after a crash, the report starts with the game's last output, which the player can edit or leave out", async ({ page }) => {
  await signIn(page, "player");
  await page.goto("/library");
  // What the Slop app's crash prompt does on "Report it" (CrashPrompt in BugReports.tsx): open the
  // app's page with the crash. The prompt itself is only in the desktop app.
  await page.evaluate((path) => {
    const crash = { slug: path.split("/")[2], code: 3, log: "panic: door.rs:12 index out of range\nC:/Users/~/den.log" };
    history.pushState({ usr: { crash }, key: "crash", idx: 1 }, "", path);
    dispatchEvent(new PopStateEvent("popstate", { state: history.state }));
  }, `/app/${slug}#bug-reports`);
  const section = page.locator("#bug-reports");
  // The form is already open, with the output to send.
  const include = section.getByRole("checkbox", { name: /Include the game's last output lines/ });
  await expect(include).toBeChecked();
  const output = section.getByRole("textbox", { name: "Output to send" });
  await expect(output).toHaveValue("panic: door.rs:12 index out of range\nC:/Users/~/den.log");
  await include.uncheck();
  await expect(output).toBeHidden();
  await include.check();
  await output.fill("panic: door.rs:12 index out of range");
  await section.getByRole("textbox", { name: "What's wrong, in a line" }).fill("Crash when the den door opens");
  await section.getByRole("textbox", { name: /What happened/ }).fill("It closed with an error.");
  await section.getByRole("button", { name: "Send to the maker" }).click();
  await expect(section.getByRole("status")).toHaveText(/Sent to the maker\./);
  const reports = await call<{ reports: Array<{ title: string; log?: string | null }> }>("maker2", "GET", `/v1/apps/${slug}/bug-reports?status=open`);
  expect(reports.reports.find((r) => r.title === "Crash when the den door opens")?.log).toBe("panic: door.rs:12 index out of range");
});

test("the maker triages: fixed with a note the player sees, duplicates, open again", async ({ browser }) => {
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(manage);
  const bugs = bo.getByRole("region", { name: "Bug reports" });
  await expect(bugs.getByText("You've turned bug reports off on the store page.")).toHaveCount(0);
  const door = bugs.getByRole("listitem").filter({ hasText: "The den door won't open" });
  await expect(door).toContainText("Pat Player");
  await expect(door).toContainText("1 more with it");
  // Not on GitHub: no issue link.
  await expect(door.getByRole("link", { name: "Open as a GitHub issue" })).toHaveCount(0);
  const crash = bugs.getByRole("listitem").filter({ hasText: "Crash when the den door opens" });
  await crash.getByText("Last output the player sent").click();
  await expect(crash.locator("pre")).toHaveText("panic: door.rs:12 index out of range");

  await door.getByRole("textbox", { name: "Note for the player" }).fill("Fixed in 1.0.1: the door had no hinge.");
  await door.getByRole("button", { name: "Mark fixed" }).click();
  await expect(door).toHaveCount(0);
  await crash.getByRole("button", { name: "Mark duplicate" }).click();
  await expect(crash).toHaveCount(0);
  await expect(bugs.getByText("None.")).toBeVisible();
  await bugs.getByLabel("Show").selectOption("duplicate");
  await expect(crash).toContainText("DUPLICATE");
  await bugs.getByLabel("Show").selectOption("fixed");
  await expect(door).toContainText("FIXED");

  // Pat sees the answer on his report, closed as it is (the page used to list only open ones).
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${slug}`);
  const mine = pat.locator("#bug-reports").getByRole("listitem").filter({ hasText: "The den door won't open" });
  await expect(mine).toContainText("FIXED");
  await expect(mine).toContainText("The maker: Fixed in 1.0.1: the door had no hinge.");

  // Not fixed after all.
  await door.getByRole("button", { name: "Open again" }).click();
  await expect(door).toHaveCount(0);
  await bugs.getByLabel("Show").selectOption("open");
  await expect(door).toContainText("OPEN");
  await pat.reload();
  await expect(mine).toContainText("OPEN");
});

test("Bo stops sharing it; Pat can't see it any more", async ({ browser }) => {
  const bo = await browser.newPage();
  await signIn(bo, "maker2");
  await bo.goto(`/app/${slug}`);
  const panel = bo.locator(".share-panel");
  for (const name of ["Pat Player", "Cleo Client"]) {
    await panel.getByRole("listitem").filter({ hasText: name }).getByRole("button", { name: "Stop sharing" }).click();
    await expect(panel.getByRole("link", { name })).toHaveCount(0);
  }
  await expect(panel.getByText("Nobody yet.")).toBeVisible();
  // Friends are offered again.
  await expect(panel.getByRole("combobox", { name: "One of your friends" })).toBeVisible();
  const pat = await browser.newPage();
  await signIn(pat, "player");
  await pat.goto(`/app/${slug}`);
  await expect(pat.getByRole("alert").getByText("We couldn't find that.")).toBeVisible();
});
