// SPDX-License-Identifier: AGPL-3.0-or-later
// Friends, teams and the connection buttons on profiles, between Mia (maker), Olly (overdue) and
// Sue (support), who have no friends or teams in the seed; the seed's friendships and teams
// (Pat's) stay as they are. Chat itself is in the desktop app.
import type { Browser, Page } from "@playwright/test";
import { expect, ids, server, signIn, test, type Who } from "./fixtures";
import { call } from "./people-helpers";

async function as(browser: Browser, who: Who, path: string): Promise<Page> {
  const page = await browser.newPage();
  await signIn(page, who);
  await page.goto(path);
  return page;
}

const link = (who: "overdue" | "maker" | "support") => `${server.url}/u/${ids[who]}`;

test("friends: a request by link, cancelled, sent again and accepted; removed; declined; blocked and unblocked", async ({ browser }) => {
  const mia = await as(browser, "maker", "/friends");
  await expect(mia.getByRole("heading", { name: /Friends/, level: 1 })).toBeVisible();
  await expect(mia.getByRole("heading", { name: "Your friends (0)" })).toBeVisible();
  const add = mia.getByRole("textbox", { name: /Add a friend: paste their profile link/ });
  const sendReq = mia.getByRole("button", { name: "Send request" });
  await expect(sendReq).toBeDisabled();

  // Not a profile link, and yourself: refused.
  await add.fill("not a link at all");
  await sendReq.click();
  await expect(mia.getByRole("alert")).toBeVisible();
  await add.fill(link("maker"));
  await sendReq.click();
  await expect(mia.getByRole("alert")).toContainText("that's you");

  // To Olly, then cancelled while it waits.
  await add.fill(link("overdue"));
  await sendReq.click();
  await expect(mia.getByText("Request sent. They'll see it under their 🔔.")).toBeVisible();
  await expect(add).toHaveValue("");
  const waiting = mia.locator("li").filter({ hasText: "Olly Overdue" });
  await expect(mia.getByRole("heading", { name: "Waiting for an answer" })).toBeVisible();
  await waiting.getByRole("button", { name: "Cancel" }).click();
  await expect(mia.getByRole("heading", { name: "Waiting for an answer" })).toHaveCount(0);

  // Again; Olly accepts.
  await add.fill(link("overdue"));
  await sendReq.click();
  const olly = await as(browser, "overdue", "/friends");
  const requests = olly.locator("#requests");
  await expect(requests.getByRole("heading", { name: "Requests (1)" })).toBeVisible();
  await requests.locator("li").filter({ hasText: "Mia Maker" }).getByRole("button", { name: "Accept" }).click();
  await expect(olly.getByText("You and Mia Maker are friends.")).toBeVisible();
  await expect(olly.getByRole("heading", { name: "Your friends (1)" })).toBeVisible();

  // On Olly's profile Mia sees they're friends, and that chat is in the app.
  await mia.goto(`/u/${ids.overdue}`);
  await expect(mia.getByText("✓ Friends")).toBeVisible();
  await expect(mia.getByRole("link", { name: "💬 Message in the app" })).toHaveAttribute("href", "/chat");

  // Mia removes Olly.
  await mia.goto("/friends");
  await mia.locator("li").filter({ hasText: "Olly Overdue" }).getByRole("button", { name: "Remove" }).click();
  await expect(mia.getByRole("heading", { name: "Your friends (0)" })).toBeVisible();

  // From a profile: Mia adds Olly; Olly, on Mia's profile, accepts; then removes again.
  await mia.goto(`/u/${ids.overdue}`);
  await mia.getByRole("button", { name: "＋ Add friend" }).click();
  await expect(mia.getByText("Friend request sent")).toBeVisible();
  await olly.goto(`/u/${ids.maker}`);
  await olly.getByRole("button", { name: "Accept friend request" }).click();
  await expect(olly.getByText("✓ Friends")).toBeVisible();
  await call("overdue", "DELETE", `/v1/me/friends/${ids.maker}`);

  // Olly declines the next request.
  await call("maker", "POST", "/v1/me/friends", { user: ids.overdue });
  await olly.goto("/friends");
  await requests.locator("li").filter({ hasText: "Mia Maker" }).getByRole("button", { name: "Decline" }).click();
  await expect(requests).toHaveCount(0);
  await expect(olly.getByRole("heading", { name: "Your friends (0)" })).toBeVisible();

  // And blocks the one after: Mia can't ask again, and shows under Blocked until unblocked.
  await call("maker", "POST", "/v1/me/friends", { user: ids.overdue });
  await olly.reload();
  await requests.locator("li").filter({ hasText: "Mia Maker" }).getByRole("button", { name: "Block" }).click();
  await expect(olly.getByText("Mia Maker is blocked.")).toBeVisible();
  const blocked = olly.locator("ul").filter({ has: olly.getByRole("button", { name: "Unblock" }) });
  await expect(olly.getByRole("heading", { name: "Blocked" })).toBeVisible();
  await expect(blocked).toContainText("Mia Maker");
  await mia.goto("/friends");
  await add.fill(link("overdue"));
  await sendReq.click();
  await expect(mia.getByRole("alert")).toBeVisible();
  await blocked.getByRole("button", { name: "Unblock" }).click();
  await expect(olly.getByRole("heading", { name: "Blocked" })).toHaveCount(0);

  // Blocking from a profile, and unblocking there.
  await mia.goto(`/u/${ids.overdue}`);
  await mia.getByRole("button", { name: "Block" }).click();
  await mia.getByRole("button", { name: "Unblock Olly Overdue" }).click();
  await expect(mia.getByRole("button", { name: "＋ Add friend" })).toBeVisible();
});

test("who can message you: anyone, or only people you know; the under-18 view can't change it", async ({ browser }) => {
  const olly = await as(browser, "overdue", "/friends#settings");
  const anyone = olly.getByRole("checkbox", { name: /Anyone on SLAPorSLOP/ });
  await expect(anyone).not.toBeChecked();
  await anyone.click();
  await expect(anyone).toBeChecked();
  expect((await call("overdue", "GET", "/v1/me/chat-settings")).from_anyone).toBe(true);
  await olly.reload();
  await expect(anyone).toBeChecked();
  await anyone.click();
  await expect(anyone).not.toBeChecked();
  expect((await call("overdue", "GET", "/v1/me/chat-settings")).from_anyone).toBe(false);

  const tia = await as(browser, "teen", "/friends");
  await expect(tia.getByText(/In the under-18 view nobody else can message you/)).toBeVisible();
  await expect(tia.getByRole("checkbox", { name: /Anyone on SLAPorSLOP/ })).toHaveCount(0);

  const visitor = await browser.newPage();
  await visitor.goto("/friends");
  await expect(visitor.getByRole("link", { name: "Sign in" }).last()).toHaveAttribute("href", "/signin?return_to=/friends");
});

test("a player team: made, members invited, roles, picks, game nights; invitations declined; leaving", async ({ browser }) => {
  test.slow();
  const mia = await as(browser, "maker", "/teams");
  await mia.getByRole("button", { name: "Start a team" }).click();
  await mia.getByRole("textbox", { name: "Name" }).fill("Lantern Lovers");
  await mia.getByRole("textbox", { name: "Address" }).fill("Bad Address!");
  await mia.getByRole("textbox", { name: /About/ }).fill("We light every lantern.");
  await mia.getByRole("button", { name: "Create" }).click();
  await expect(mia.getByRole("alert")).toBeVisible();
  await mia.getByRole("textbox", { name: "Address" }).fill("lantern-lovers");
  await expect(mia.getByText("slop/teams/lantern-lovers")).toBeVisible();
  await mia.getByRole("button", { name: "Create" }).click();
  await expect(mia).toHaveURL("/teams/lantern-lovers");
  await expect(mia.getByRole("heading", { name: "Lantern Lovers", level: 1 })).toBeVisible();
  await expect(mia.getByText(/Invite-only player team · 1 member/)).toBeVisible();
  await expect(mia.getByText("We light every lantern.")).toBeVisible();

  // Invite Olly and Sue by their profile links.
  const invite = mia.getByRole("textbox", { name: "Their profile link" });
  await invite.fill(link("overdue"));
  await mia.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(invite).toHaveValue("");
  await invite.fill(link("support"));
  await mia.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(invite).toHaveValue("");

  // Olly finds the invitation on Teams and accepts; Sue declines hers.
  const olly = await as(browser, "overdue", "/teams");
  await expect(olly.getByRole("heading", { name: "Invitations" })).toBeVisible();
  await olly.getByRole("link", { name: /Lantern Lovers/ }).click();
  await olly.getByRole("button", { name: "Accept the invitation" }).click();
  await expect(olly.getByText(/Invite-only player team · 2 members/)).toBeVisible();
  await expect(olly.getByRole("button", { name: "Leave" })).toBeVisible();
  const sue = await as(browser, "support", "/teams/lantern-lovers");
  await sue.getByRole("button", { name: "Decline" }).click();
  await expect(sue).toHaveURL("/teams");
  await expect(sue.getByRole("heading", { name: "Invitations" })).toHaveCount(0);

  // The owner makes Olly an admin, then a member again.
  await mia.reload();
  const role = mia.getByRole("combobox", { name: "Olly Overdue's role" });
  await role.selectOption("admin");
  await expect(mia.locator(".team-members li").filter({ hasText: "Olly Overdue" })).toContainText("· admin");
  await role.selectOption("member");
  await expect(mia.locator(".team-members li").filter({ hasText: "Olly Overdue" })).not.toContainText("· admin");

  // Picks: an app by its address or link; a wrong one is refused.
  const pickApp = mia.getByRole("textbox", { name: "App link or address" });
  await pickApp.fill("no-such-game");
  await mia.getByRole("button", { name: "Pick it" }).click();
  await expect(mia.locator(".compact-form .form-error").first()).toBeVisible();
  await pickApp.fill(`${server.url}/app/castle-rock`);
  await mia.getByRole("textbox", { name: "Why (optional)" }).fill("Towers!");
  await mia.getByRole("button", { name: "Pick it" }).click();
  const pick = mia.locator(".team-picks li").filter({ hasText: "Castle Rock" });
  await expect(pick).toContainText("picked by Mia Maker");
  await expect(pick).toContainText("“Towers!”");

  // A game night, seen by Olly; Mia cancels it.
  await mia.getByRole("textbox", { name: "Game night title" }).fill("Castle night");
  await expect(mia.getByRole("button", { name: "Plan it" })).toBeDisabled();
  // Only within the coming year.
  await mia.getByLabel("When").fill("2030-05-01T20:00");
  await mia.getByRole("textbox", { name: "App (optional)" }).fill("castle-rock");
  await mia.getByRole("button", { name: "Plan it" }).click();
  await expect(mia.locator(".compact-form .form-error").first()).toContainText("a time in the coming year");
  await mia.getByLabel("When").fill(`${new Date(Date.now() + 30 * 86400_000).toISOString().slice(0, 10)}T20:00`);
  await mia.getByRole("button", { name: "Plan it" }).click();
  const night = mia.locator(".team-events li").filter({ hasText: "Castle night" });
  await expect(night).toContainText("set by Mia Maker");
  await expect(night.getByRole("link", { name: "Castle Rock" })).toBeVisible();
  await olly.reload();
  await expect(olly.locator(".team-events li").filter({ hasText: "Castle night" })).toBeVisible();
  await expect(olly.locator(".team-picks li").filter({ hasText: "Castle Rock" })).toBeVisible();
  // Only admins remove picks; only who set a night cancels it.
  await expect(olly.locator(".team-picks li").getByRole("button", { name: "Remove" })).toHaveCount(0);
  await expect(olly.locator(".team-events li").getByRole("button", { name: "Cancel it" })).toHaveCount(0);
  await night.getByRole("button", { name: "Cancel it" }).click();
  await expect(night).toHaveCount(0);
  await pick.getByRole("button", { name: "Remove" }).click();
  await expect(pick).toHaveCount(0);

  // Olly leaves; Mia removes nobody else and the team is hers alone again.
  await olly.getByRole("button", { name: "Leave" }).click();
  await expect(olly).toHaveURL("/teams");
  await mia.reload();
  await expect(mia.getByText(/Invite-only player team · 1 member/)).toBeVisible();

  // Invited again, Olly joins and the owner removes him.
  await invite.fill(link("overdue"));
  await mia.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(invite).toHaveValue("");
  await call("overdue", "POST", "/v1/teams/lantern-lovers/invites/accept", {});
  await mia.reload();
  await mia.locator(".team-members li").filter({ hasText: "Olly Overdue" }).getByRole("button", { name: "Remove" }).click();
  await expect(mia.locator(".team-members li").filter({ hasText: "Olly Overdue" })).toHaveCount(0);
});

test("an open team anyone can join, and an official studio team's verification", async ({ browser }) => {
  const mia = await as(browser, "maker", "/teams");
  await mia.getByRole("button", { name: "Start a team" }).click();
  await mia.getByRole("textbox", { name: "Name" }).fill("Open Lanterns");
  await mia.getByRole("textbox", { name: "Address" }).fill("open-lanterns");
  await mia.getByRole("checkbox", { name: /Anyone can find and join it/ }).check();
  await mia.getByRole("button", { name: "Create" }).click();
  await expect(mia.getByText(/Open player team · 1 member/)).toBeVisible();

  // Listed for everyone; Olly can report it, join, and leave.
  const olly = await as(browser, "overdue", "/teams");
  await olly.getByRole("link", { name: /Open Lanterns/ }).click();
  await expect(olly.getByRole("button", { name: "⚑ Report" })).toBeVisible();
  await olly.getByRole("button", { name: "Join" }).click();
  await expect(olly.getByText(/Open player team · 2 members/)).toBeVisible();
  // An open team has no team chat.
  await expect(olly.getByRole("link", { name: /Team chat/ })).toHaveCount(0);
  await olly.getByRole("button", { name: "Leave" }).click();
  await expect(olly).toHaveURL("/teams");

  // An official team waits for verification through its GitHub organization.
  await mia.goto("/teams");
  await mia.getByRole("button", { name: "Start a team" }).click();
  await mia.getByRole("radio", { name: /Official studio team/ }).check();
  await mia.getByRole("textbox", { name: "Name" }).fill("Mia Studio");
  await mia.getByRole("textbox", { name: "Address" }).fill("mia-studio");
  await mia.getByRole("textbox", { name: "GitHub organization" }).fill("mia-studio");
  await mia.getByRole("button", { name: "Create" }).click();
  await expect(mia.getByRole("heading", { name: /Mia Studio/, level: 1 })).toContainText("NOT VERIFIED YET");
  await expect(mia.getByText(/Studio · github\.com\/mia-studio/)).toBeVisible();
  const verification = mia.locator("section.verification");
  await expect(verification.getByRole("heading", { name: "Verification" })).toBeVisible();
  // Proving a domain with DNS: the record to add, then a check (which can't pass here).
  await verification.getByText(/Prove one with DNS/).click();
  await verification.getByRole("textbox", { name: "Your studio's domain" }).fill("mia.example");
  await verification.getByRole("button", { name: "Check now" }).click();
  await expect(verification.getByRole("alert").or(verification.locator("ul.checks"))).toBeVisible();
  await expect(verification.getByRole("button", { name: "Check now" })).toBeEnabled();
  await expect(mia.getByRole("heading", { name: /Mia Studio/, level: 1 })).toContainText("NOT VERIFIED YET");
  // Not verified: not listed for others.
  await olly.goto("/teams");
  await expect(olly.getByRole("link", { name: /Mia Studio/ })).toHaveCount(0);
});

test("the seed's teams: the invited player sees the invitation; members see the team chat link; signed out sees teams", async ({
  browser,
}) => {
  const tia = await as(browser, "teen", `/teams/${ids.inviteTeam}`);
  await expect(tia.getByRole("button", { name: "Accept the invitation" })).toBeVisible();
  await expect(tia.getByRole("button", { name: "Decline" })).toBeVisible();
  const pat = await as(browser, "player", `/teams/${ids.inviteTeam}`);
  await expect(pat.getByRole("link", { name: "💬 Team chat in the app" })).toHaveAttribute("href", "/chat");
  const visitor = await browser.newPage();
  await visitor.goto("/teams");
  await expect(visitor.getByRole("button", { name: "Start a team" })).toHaveCount(0);
  await expect(visitor.getByRole("link", { name: /Open Crew/ })).toBeVisible();
});

test("chat on the website says it's in the app", async ({ browser }) => {
  const pat = await as(browser, "player", "/chat");
  await expect(pat.getByText("Chat is in the SLAPorSLOP app.")).toBeVisible();
  await expect(pat.getByRole("link", { name: /Get SLAPorSLOP for/ })).toHaveAttribute("href", "/download");
  await pat.goto(`/chat/${ids.directChat}`);
  await expect(pat.getByText("Chat is in the SLAPorSLOP app.")).toBeVisible();
  await pat.getByRole("link", { name: "Your friends" }).click();
  await expect(pat).toHaveURL("/friends");
});
