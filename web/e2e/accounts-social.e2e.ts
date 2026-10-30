// SPDX-License-Identifier: AGPL-3.0-or-later
// Profiles, friends and teams on the website (pages/Profile.tsx, Friends.tsx, Teams.tsx, Chat.tsx).
// Sue Support and Sec Security befriend, block and team up here, and leave things as they were.
import { apiError, call, fake, tryCall, unique } from "./accounts-helpers";
import { expect, ids, server, signIn, test } from "./fixtures";

const profile = (id: string) => `${server.url}/u/${id}`;

/** Sue and Sec are strangers again. */
async function strangers() {
  for (const [a, b] of [
    ["support", "security"],
    ["security", "support"],
  ] as const) {
    await tryCall(a, "DELETE", `/v1/me/friends/${ids[b]}`);
    await tryCall(a, "DELETE", `/v1/me/blocks/${ids[b]}`);
  }
}

test.describe("profiles", () => {
  test("a public profile shows votes and reviews, and the apps someone makes", async ({ page }) => {
    await page.goto(`/u/${ids.player}`);
    await expect(page.getByRole("heading", { name: "Pat Player", level: 1 })).toBeVisible();
    await expect(page.getByText(/On SLAPorSLOP since \w+ \d{4}/)).toBeVisible();
    // (A tile's link is named by the app's summary, not its title: see the report.)
    const vote = page.locator(".profile-votes").getByRole("listitem").filter({ has: page.locator('a[href="/app/lantern-keep"]') });
    await expect(vote).toContainText("Slaps");
    await expect(vote).toContainText("The fog timer makes every level tense.");
    // Signed out: no friend buttons.
    await expect(page.getByRole("button", { name: /Add friend|Block/ })).toHaveCount(0);

    await page.goto(`/u/${ids.maker}`);
    await expect(page.getByRole("heading", { name: "Apps they make" })).toBeVisible();
    await expect(page.locator('a[href="/app/lantern-keep"]')).toBeVisible();
  });

  test("a creative's profile links to their People page", async ({ page }) => {
    await page.goto(`/u/${ids.creative}`);
    await page.getByRole("link", { name: "Hire them on People" }).click();
    await expect(page).toHaveURL(new RegExp(`/people/${ids.creativeHandle}$`));
  });

  test("a private profile hides votes from others, and tells its owner", async ({ page }) => {
    // Under-18 profiles are private by default (reviews.rs).
    await signIn(page, "player");
    await page.goto(`/u/${ids.teen}`);
    await expect(page.getByText("Tia Teen keeps their votes and reviews private.")).toBeVisible();
    await signIn(page, "teen");
    await page.reload();
    await expect(page.getByText("This is how others see you.")).toContainText(
      "Your votes and reviews are private: others see only your name and apps.",
    );
    await expect(page.getByRole("link", { name: "Change it" })).toHaveAttribute("href", "/account#privacy");
    await expect(page.getByText("keeps their votes and reviews private")).toHaveCount(0);
  });

  test("your own public profile says so, with no buttons for yourself", async ({ page }) => {
    await signIn(page, "player");
    await page.goto(`/u/${ids.player}`);
    await expect(page.getByText("This is how others see you.")).toContainText("Your votes and reviews are public.");
    await expect(page.getByRole("button", { name: /Add friend|Block/ })).toHaveCount(0);
  });

  test("an unknown profile isn't found", async ({ page }) => {
    await page.goto("/u/01a00000-0000-7000-8000-000000000000");
    await expect(page.getByText("We couldn't find that.")).toBeVisible();
  });

  test("a friend's profile says so, with a way to message in the app", async ({ page }) => {
    await signIn(page, "player");
    await page.goto(`/u/${ids.maker2}`);
    await expect(page.getByText("✓ Friends")).toBeVisible();
    await expect(page.getByRole("link", { name: "Message in the app", exact: true })).toHaveAttribute("href", "/chat");
    await expect(page.getByRole("button", { name: "Block" })).toBeVisible();
  });

  test("from a profile: ask, accept, block and unblock", async ({ browser }) => {
    await strangers();
    const sue = await browser.newPage();
    const sec = await browser.newPage();
    try {
      await signIn(sue, "support");
      await signIn(sec, "security");
      await sue.goto(profile(ids.security));
      await expect(sue.getByRole("link", { name: /Message in the app/ })).toHaveCount(0);
      await sue.getByRole("button", { name: "＋ Add friend" }).click();
      await expect(sue.getByText("Friend request sent")).toBeVisible();

      await sec.goto(profile(ids.support));
      await sec.getByRole("button", { name: "Accept friend request" }).click();
      await expect(sec.getByText("✓ Friends")).toBeVisible();
      await expect(sec.getByRole("link", { name: "Message in the app", exact: true })).toBeVisible();

      await sec.getByRole("button", { name: "Block" }).click();
      await expect(sec.getByText("✓ Friends")).toHaveCount(0);
      await sec.getByRole("button", { name: "Unblock Sue Support" }).click();
      await expect(sec.getByRole("button", { name: "＋ Add friend" })).toBeVisible();

      // Blocked people can't ask.
      await sec.getByRole("button", { name: "Block" }).click();
      await sue.reload();
      await expect(sue.getByRole("heading", { name: "Sec Security" })).toBeVisible();
      await sue.getByRole("button", { name: "＋ Add friend" }).click();
      await expect(sue.getByRole("alert")).toHaveText("not found");
    } finally {
      await strangers();
      await sue.close();
      await sec.close();
    }
  });
});

test.describe("the friends page", () => {
  test("signed out, it asks you to sign in", async ({ page }) => {
    await page.goto("/friends");
    await expect(page.getByText("to see your friends.")).toBeVisible();
    await expect(page.getByRole("main").getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/signin?return_to=/friends");
  });

  test("asking by profile link, and every answer to a request", async ({ browser }) => {
    await strangers();
    const sue = await browser.newPage();
    const sec = await browser.newPage();
    try {
      await signIn(sue, "support");
      await signIn(sec, "security");
      await sue.goto("/friends");
      await expect(sue.getByText("No friends yet.")).toBeVisible();
      await expect(sue.getByRole("link", { name: "About chat" })).toHaveAttribute("href", "/chat");
      const link = sue.getByRole("textbox", { name: "Add a friend: paste their profile link" });
      const ask = sue.getByRole("button", { name: "Send request" });
      await expect(ask).toBeDisabled();
      await link.fill("not a link");
      await ask.click();
      await expect(sue.getByRole("alert")).toHaveText("paste their profile link");
      await link.fill(profile(ids.support));
      await ask.click();
      await expect(sue.getByRole("alert")).toHaveText("that's you");

      // Ask, then take it back.
      await link.fill(profile(ids.security));
      await ask.click();
      await expect(sue.getByText("Request sent. They'll see it in their notifications.")).toBeVisible();
      await expect(link).toHaveValue("");
      const waiting = sue.getByRole("listitem").filter({ hasText: "Sec Security" });
      await expect(sue.getByRole("heading", { name: "Waiting for an answer" })).toBeVisible();
      await waiting.getByRole("button", { name: "Cancel" }).click();
      await expect(waiting).toHaveCount(0);

      // Ask; declined.
      await link.fill(profile(ids.security));
      await ask.click();
      await sec.goto("/friends");
      const request = sec.locator("#requests").getByRole("listitem").filter({ hasText: "Sue Support" });
      await expect(sec.getByRole("heading", { name: "Requests (1)" })).toBeVisible();
      await request.getByRole("button", { name: "Decline" }).click();
      await expect(request).toHaveCount(0);

      // Ask; accepted, then removed.
      await link.fill(profile(ids.security));
      await ask.click();
      await sec.reload();
      await request.getByRole("button", { name: "Accept" }).click();
      await expect(sec.getByText("You and Sue Support are friends.")).toBeVisible();
      const friend = sec.getByRole("listitem").filter({ hasText: "Sue Support" });
      await expect(sec.getByRole("heading", { name: "Your friends (1)" })).toBeVisible();
      await expect(friend).toContainText(/since \w{3} \d+, \d{4}/);
      await expect(friend.getByRole("button", { name: /Message/ })).toHaveCount(0);
      await friend.getByRole("button", { name: "Remove" }).click();
      await expect(sec.getByRole("heading", { name: "Your friends (0)" })).toBeVisible();

      // Ask; blocked, then unblocked.
      await link.fill(profile(ids.security));
      await ask.click();
      await sec.reload();
      await request.getByRole("button", { name: "Block" }).click();
      await expect(sec.getByText("Sue Support is blocked.")).toBeVisible();
      const blocked = sec.getByRole("listitem").filter({ hasText: "Sue Support" });
      await expect(sec.getByRole("heading", { name: "Blocked" })).toBeVisible();
      await blocked.getByRole("button", { name: "Unblock" }).click();
      await expect(sec.getByRole("heading", { name: "Blocked" })).toHaveCount(0);

      // Asking someone who already asked you makes you friends at once.
      await call("security", "POST", "/v1/me/friends", { user: ids.support });
      await link.fill(profile(ids.security));
      await ask.click();
      await expect(sue.getByText("You're friends now.")).toBeVisible();
      await expect(sue.getByRole("heading", { name: "Your friends (1)" })).toBeVisible();
    } finally {
      await strangers();
      await sue.close();
      await sec.close();
    }
  });

  test("who can message you is saved; a failed save says so", async ({ page }) => {
    await signIn(page, "support");
    try {
      await page.goto("/friends#settings");
      const anyone = page.getByRole("checkbox", { name: /Anyone on SLAPorSLOP/ });
      await expect(anyone).not.toBeChecked();
      await anyone.click();
      await expect(anyone).toBeChecked();
      expect((await call("support", "GET", "/v1/me/chat-settings")).from_anyone).toBe(true);
      await page.reload();
      await expect(anyone).toBeChecked();
      await fake(page, "PUT", "/v1/me/chat-settings", { status: 503, body: apiError("unavailable", "SLAPorSLOP is busy: try again in a minute") });
      await anyone.click();
      await expect(page.getByRole("alert")).toHaveText("SLAPorSLOP is busy: try again in a minute");
      await expect(anyone).toBeChecked();
    } finally {
      await call("support", "PUT", "/v1/me/chat-settings", { from_anyone: false });
    }
  });

  test("in the under-18 view, only friends, teammates and job partners can message", async ({ page }) => {
    await signIn(page, "teen");
    await page.goto("/friends");
    await expect(page.getByText("In the under-18 view nobody else can message you.")).toBeVisible();
    await expect(page.getByRole("checkbox", { name: /Anyone on SLAPorSLOP/ })).toHaveCount(0);
    await expect(page.getByRole("listitem").filter({ hasText: "Pat Player" })).toBeVisible();
  });
});

test.describe("chat on the website", () => {
  test("points to the app for this computer, and to friends", async ({ page }) => {
    await page.goto("/chat");
    await expect(page.getByRole("heading", { name: "Chat", exact: true })).toBeVisible();
    await expect(page.getByText("Chat is in the SLAPorSLOP app.")).toBeVisible();
    // The browser here says it's Chrome on Windows.
    await expect(page.getByRole("link", { name: "Get SLAPorSLOP for Windows", exact: true })).toHaveAttribute("href", "/download");
    await page.getByRole("link", { name: "Your friends" }).click();
    await expect(page).toHaveURL(/\/friends$/);
  });
});

test.describe("teams", () => {
  test("the list shows open teams to anyone, and invitations to the invited", async ({ page }) => {
    await page.goto("/teams");
    await expect(page.getByRole("button", { name: "Start a team" })).toHaveCount(0);
    const open = page.getByRole("link", { name: /Open Crew/ });
    await expect(open).toContainText("OPEN");
    await expect(page.getByRole("link", { name: /Night Owls/ })).toHaveCount(0);
    await signIn(page, "teen");
    await page.reload();
    await expect(page.getByRole("heading", { name: "Invitations" })).toBeVisible();
    await expect(page.getByRole("link", { name: /Night Owls/ })).toContainText("INVITE ONLY");
  });

  test("starting a player team, with the server's complaints by each field", async ({ page }) => {
    const slug = unique("acc-crew");
    await signIn(page, "security");
    await page.goto("/teams");
    await page.getByRole("button", { name: "Start a team" }).click();
    await page.getByRole("textbox", { name: "Name" }).fill("Accounts Crew");
    const address = page.getByRole("textbox", { name: "Address" });
    await address.fill("A");
    await expect(address).toHaveValue("a");
    await expect(page.getByText("slop/teams/a")).toBeVisible();
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("alert")).toHaveText("3 to 40 lowercase letters, numbers and single dashes");
    await address.fill(slug);
    await page.getByRole("textbox", { name: /About/ }).fill("Testing teams.");
    await page.getByRole("checkbox", { name: /Anyone can find and join it/ }).check();
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page).toHaveURL(new RegExp(`/teams/${slug}$`));
    await expect(page.getByRole("heading", { name: "Accounts Crew" })).toBeVisible();
    await expect(page.getByText("Open player team · 1 member")).toBeVisible();
    await expect(page.getByText("Testing teams.")).toBeVisible();
    // Address taken.
    await page.goto("/teams");
    await page.getByRole("button", { name: "Start a team" }).click();
    await page.getByRole("textbox", { name: "Name" }).fill("Copy");
    await page.getByRole("textbox", { name: "Address" }).fill(slug);
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("alert")).toHaveText("that address or organization already has a team");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("textbox", { name: "Address" })).toHaveCount(0);
  });

  test("an official team: its organization, the checks, and proving a domain", async ({ page }) => {
    const slug = unique("acc-studio");
    const org = unique("acc-org");
    await signIn(page, "security");
    await page.goto("/teams");
    await page.getByRole("button", { name: "Start a team" }).click();
    await page.getByRole("radio", { name: /Official studio team/ }).check();
    await expect(page.getByRole("checkbox", { name: /Anyone can find and join it/ })).toHaveCount(0);
    await page.getByRole("textbox", { name: "Name" }).fill("Accounts Studio");
    await page.getByRole("textbox", { name: "Address" }).fill(slug);
    const github = page.getByRole("textbox", { name: /GitHub organization/ });
    await github.fill("not an org!");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("alert")).toHaveText("your GitHub organization's name, as in github.com/<name>");
    await github.fill(org);
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("heading", { name: /Accounts Studio/ })).toContainText("NOT VERIFIED YET");
    await expect(page.getByText(`Studio · github.com/${org}`)).toBeVisible();

    const verification = page.locator("section.verification");
    await expect(verification).toContainText(`SLAPorSLOP checks these through its GitHub App on github.com/${org}`);
    await verification.getByText("No verified domain on GitHub? Prove one with DNS").click();
    await verification.getByRole("textbox", { name: "Your studio's domain" }).fill("studio.example");
    await expect(verification.getByText("_slop-team.studio.example")).toBeVisible();
    await verification.getByRole("button", { name: "Check now" }).click();
    // The test server has no GitHub App: the checks say what's missing.
    await expect(verification.getByRole("listitem").filter({ hasText: "SLAPorSLOP's GitHub App is installed" })).toContainText("Not yet:");
    await expect(page.getByRole("heading", { name: /Accounts Studio/ })).toContainText("NOT VERIFIED YET");
    await expect(verification).toContainText(/Last checked \w{3} \d+, \d{4}\./);
  });

  test("a check that fails says why; a verified team shows the badge", async ({ page }) => {
    const slug = unique("acc-studio");
    await call("security", "POST", "/v1/teams", { slug, name: "Badge Studio", kind: "official", github_org: unique("acc-org") });
    await fake(page, "POST", `/v1/teams/${slug}/verify`, { status: 503, body: apiError("unavailable", "GitHub didn't answer: try again later") });
    await signIn(page, "security");
    await page.goto(`/teams/${slug}`);
    await page.getByRole("button", { name: "Check now" }).click();
    await expect(page.locator("section.verification").getByRole("alert")).toHaveText("GitHub didn't answer: try again later");
    await page.unrouteAll();
    // The reload can cancel a request the old page still had open; only the new page's matters.
    await page.route(`**/v1/teams/${slug}`, async (route) => {
      try {
        const res = await route.fetch();
        await route.fulfill({ response: res, json: { ...(await res.json()), verified: true } });
      } catch {
        // Cancelled by the reload.
      }
    });
    await page.reload();
    await expect(page.getByRole("heading", { name: /Badge Studio/ }).getByTitle("Verified through its GitHub organization")).toHaveText("✓ OFFICIAL");
  });

  test("an open team: join and leave; a report button only for outsiders", async ({ browser }) => {
    const slug = unique("acc-open");
    await call("security", "POST", "/v1/teams", { slug, name: "Open Accounts", kind: "player", join_policy: "open" });
    const page = await browser.newPage();
    await signIn(page, "support");
    await page.goto(`/teams/${slug}`);
    await expect(page.getByRole("button", { name: "Report", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Leave" })).toHaveCount(0);
    await page.getByRole("button", { name: "Join" }).click();
    await expect(page.getByText("Open player team · 2 members")).toBeVisible();
    await expect(page.getByRole("button", { name: "Report", exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Join" })).toHaveCount(0);
    // Open teams have no team chat.
    await expect(page.getByRole("link", { name: /Team chat/ })).toHaveCount(0);
    await page.getByRole("button", { name: "Leave" }).click();
    await expect(page).toHaveURL(/\/teams$/);
    await page.close();
  });

  test("an invite-only team: invitations, roles, removing, and what plain members can't do", async ({ browser }) => {
    const slug = unique("acc-owls");
    await call("security", "POST", "/v1/teams", { slug, name: "Accounts Owls", kind: "player" });
    const owner = await browser.newPage();
    const sue = await browser.newPage();
    await signIn(owner, "security");
    await signIn(sue, "support");

    // Outsiders don't see an invite-only team at all.
    await sue.goto(`/teams/${slug}`);
    await expect(sue.getByText("We couldn't find that.")).toBeVisible();

    await owner.goto(`/teams/${slug}`);
    const members = owner.locator(".team-members");
    const invite = owner.getByRole("textbox", { name: "Their profile link" });
    await invite.fill(profile(ids.support));
    await owner.getByRole("button", { name: "Invite" }).click();
    await expect(invite).toHaveValue("");

    // Declined, then invited again and accepted.
    await sue.reload();
    await sue.getByRole("button", { name: "Decline" }).click();
    await expect(sue).toHaveURL(/\/teams$/);
    await invite.fill(profile(ids.support));
    await owner.getByRole("button", { name: "Invite" }).click();
    await sue.goto(`/teams/${slug}`);
    await sue.getByRole("button", { name: "Accept the invitation" }).click();
    await expect(sue.getByText("Invite-only player team · 2 members")).toBeVisible();
    await expect(sue.getByRole("link", { name: "Team chat in the app", exact: true })).toHaveAttribute("href", "/chat");

    // A plain member manages nothing, and the server refuses if asked.
    await expect(sue.getByRole("combobox", { name: /role/ })).toHaveCount(0);
    await expect(sue.getByRole("textbox", { name: "Their profile link" })).toHaveCount(0);
    await expect(sue.locator(".team-members").getByRole("button", { name: "Remove" })).toHaveCount(0);
    const refused = await tryCall("support", "POST", `/v1/teams/${slug}/invites`, { user: `/u/${ids.finance}` });
    expect(refused).toEqual({ status: 409, body: apiError("conflict", "only the team's owner and admins can do that") });

    // The owner makes Sue an admin, then removes her.
    await owner.reload();
    await owner.getByRole("combobox", { name: "Sue Support's role" }).selectOption("admin");
    await expect(members.getByRole("listitem").filter({ hasText: "Sue Support" })).toContainText("· admin");
    await sue.reload();
    await expect(sue.getByRole("textbox", { name: "Their profile link" })).toBeVisible();
    // Admins can't change roles; only the owner can.
    await expect(sue.getByRole("combobox", { name: /role/ })).toHaveCount(0);
    await members.getByRole("listitem").filter({ hasText: "Sue Support" }).getByRole("button", { name: "Remove" }).click();
    await expect(members.getByRole("listitem").filter({ hasText: "Sue Support" })).toHaveCount(0);
    await sue.reload();
    await expect(sue.getByText("We couldn't find that.")).toBeVisible();
    await owner.close();
    await sue.close();
  });

  test("picks and game nights: added, removed, and errors said", async ({ page }) => {
    const slug = unique("acc-picks");
    await call("security", "POST", "/v1/teams", { slug, name: "Accounts Picks", kind: "player" });
    await signIn(page, "security");
    await page.goto(`/teams/${slug}`);
    await expect(page.getByText("No picks yet. Share an app the team should try.")).toBeVisible();
    await expect(page.getByText("Nothing planned.")).toBeVisible();

    const app = page.getByRole("textbox", { name: "App link or address" });
    await app.fill(`${server.url}/app/castle-rock/`);
    await page.getByRole("textbox", { name: "Why (optional)" }).fill("Short and sweet.");
    await page.getByRole("button", { name: "Pick it" }).click();
    const castle = page.locator(".team-picks").getByRole("listitem").filter({ hasText: "Castle Rock" });
    await expect(castle).toContainText("picked by Sec Security");
    await expect(castle).toContainText("“Short and sweet.”");
    await app.fill("neon-drift");
    await page.getByRole("button", { name: "Pick it" }).click();
    await expect(page.locator(".team-picks").getByRole("listitem").filter({ hasText: "Neon Drift" })).toBeVisible();
    await app.fill("no-such-app-here");
    await page.getByRole("button", { name: "Pick it" }).click();
    await expect(page.locator("form").filter({ has: app }).getByRole("alert")).toBeVisible();
    await castle.getByRole("button", { name: "Remove" }).click();
    await expect(castle).toHaveCount(0);
    // A remove that fails says so (it used to fail silently).
    await fake(page, "DELETE", new RegExp(`^/v1/teams/${slug}/picks/`), {
      status: 503,
      body: apiError("unavailable", "SLAPorSLOP is busy: try again in a minute"),
    });
    await page.locator(".team-picks").getByRole("listitem").filter({ hasText: "Neon Drift" }).getByRole("button", { name: "Remove" }).click();
    await expect(page.locator("form").filter({ has: app }).getByRole("alert")).toHaveText("SLAPorSLOP is busy: try again in a minute");
    await expect(page.locator(".team-picks").getByRole("listitem").filter({ hasText: "Neon Drift" })).toBeVisible();

    const title = page.getByRole("textbox", { name: "Game night title" });
    const plan = page.getByRole("button", { name: "Plan it" });
    await title.fill("Accounts night");
    await expect(plan).toBeDisabled();
    const when = new Date(Date.now() + 2 * 86_400_000);
    const local = new Date(when.getTime() - when.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
    await page.getByRole("textbox", { name: "When" }).fill(local);
    await page.getByRole("textbox", { name: "App (optional)" }).fill("castle-rock");
    await plan.click();
    const night = page.locator(".team-events").getByRole("listitem").filter({ hasText: "Accounts night" });
    await expect(night).toContainText("set by Sec Security");
    await expect(night.getByRole("link", { name: "Castle Rock" })).toHaveAttribute("href", "/app/castle-rock");
    // Too far ahead.
    await title.fill("Far future");
    await page.getByRole("textbox", { name: "When" }).fill("2099-01-01T20:00");
    await plan.click();
    await expect(page.locator("form").filter({ has: title }).getByRole("alert")).toHaveText("a time in the coming year");
    await fake(page, "DELETE", new RegExp(`^/v1/teams/${slug}/events/`), {
      status: 503,
      body: apiError("unavailable", "SLAPorSLOP is busy: try again in a minute"),
    }, { times: 1 });
    await night.getByRole("button", { name: "Cancel it" }).click();
    await expect(page.locator("form").filter({ has: title }).getByRole("alert")).toHaveText("SLAPorSLOP is busy: try again in a minute");
    await night.getByRole("button", { name: "Cancel it" }).click();
    await expect(night).toHaveCount(0);
  });

  test("an invited teen sees the invitation's buttons", async ({ page }) => {
    await signIn(page, "teen");
    await page.goto(`/teams/${ids.inviteTeam}`);
    await expect(page.getByRole("button", { name: "Accept the invitation" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Decline" })).toBeVisible();
  });
});
