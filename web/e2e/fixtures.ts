// SPDX-License-Identifier: AGPL-3.0-or-later
// What the tests share: the test server's address, its seeded accounts, and the IDs of what's
// seeded. The seed is crates/slop-api/examples/e2e_server/seed.rs; read it for the details.
//
// Accounts (sign in with `signIn(page, "<key>")`; all US unless said, agreements accepted):
//   maker           Mia Maker: Mia's First Game (mias-first, listed), Lantern Keep (lantern-keep:
//                   the busy store page), Starship Draft (starship-draft, a draft). Her AI agent
//                   wrote the admins two messages ("Release upload times out", "Show scan progress").
//   player          Pat Player: played, rated and reviewed Lantern Keep, donated to it, opened and
//                   pledged to the "Controller support" bounty, reported a bug, friends with Bo and
//                   Tia, owns the Night Owls team, chats with Bo. Cleo's friend request waits.
//   admin           Ada Admin: staff, owner role (everything).
//   overdue         Olly Overdue: past the passkey deadline with no passkey (publishing asks).
//   teen            Tia Teen (GB): the under-18 view. Friends with Pat, invited to Night Owls.
//   maker2          Bo Builder: Pocket Golf (pocket-golf: release 1.1.0 and pictures waiting for
//                   review, a repository move waiting, a re-scan flag), Frozen Fjord (frozen-fjord,
//                   frozen by staff), Delisted Dash (delisted-dash, taken out of the store; Bo
//                   appealed), the Linux build (lantern-keep-linux) and mod
//                   (lantern-keep-more-lanterns) of Lantern Keep; claimed the "A Linux build"
//                   bounty. Owns the open team Open Crew. Has a Stripe account (bounty payouts).
//   creative        Vera Okafor: People page vera-pixels (live, portfolio approved, taking work,
//                   Stripe active). Five jobs from Cleo (see the ids).
//   client          Cleo Client: owns Cleo's Tavern (cleos-tavern), hired Vera; pays a monthly
//                   server donation to Lantern Keep; opened the "A Linux build" bounty; in Night
//                   Owls and Open Crew.
//   pendingCreative Remy Pending: People page remy-sounds, portfolio waiting for review.
//   moderator, reviewer, support, finance, security
//                   Mo Moderator, Rae Reviewer, Sue Support, Fin Finance, Sec Security: staff with
//                   that one role (crates/slop-api/src/staff.rs).
//   suspended       Sid Suspended: suspended by staff for harassment; appealed.
// Without a sign-in: Sam Seed (castle-rock; neon-drift, which plays online), and players Kai
// Kestrel, Lu Lark and Noor Nightjar (votes, a review and bug reports on Lantern Keep, bounty
// pledges, reports).
//
// Objects (server.ids):
//   listedApp "lantern-keep" (listedAppId): signed release 1.0.0, store pictures, 7 votes,
//     3 reviews (Mia answered crashReview), 2 open bug reports (bugReport has a "me too") and a
//     fixed one (fixedBugReport), donations on (server donations too): Pat's paid donation
//     (donation) and Cleo's monthly server donation (serverDonation); bounties openBounty (open,
//     Pat pledged) and claimedBounty (Cleo's, claimed by Bo's Linux build, waiting for Mia).
//   communityBuild "lantern-keep-linux", mod "lantern-keep-more-lanterns", draftApp
//     "starship-draft", frozenApp "frozen-fjord", delistedApp "delisted-dash", maker2App
//     "pocket-golf", releaseInReview (on releaseInReviewApp, "pocket-golf"), clientApp
//     "cleos-tavern".
//   People: creativeHandle "vera-pixels", pendingCreativeHandle "remy-sounds". Jobs, all Cleo
//     hiring Vera: deliveredJob (talked over in its conversation; delivered, Cleo to pay),
//     workingJob (paid up front through Stripe, in progress), requestedJob (just asked),
//     problemJob (in progress; Cleo opened a problem), completedJob (paid and done; Cleo's review
//     is disputed by Vera).
//   Teams: inviteTeam "night-owls" (Pat owns it; Bo and Cleo are members, Tia is invited; a pick
//     and a game night), openTeam "open-crew" (Bo's; Cleo joined). Chat is in the desktop app
//     only, end-to-end encrypted: directChat (Pat and Bo), teamChat (Night Owls). The seed's chat
//     devices are removed afterwards, so a test's device is the account's first.
//   Admin queues: 3 open reports (report is one's reference), 2 appeals (suspensionDecision,
//     delistDecision), staffAction (an AI agent asked to make Sam support staff: it waits for a
//     person), a server alert, a possible-abuse signal, and the release, pictures, repository
//     move, portfolio, People problem and review dispute above. No donation pauses and no
//     shelves (admin.e2e.ts expects none), and no AI messages beyond Mia's two
//     (ai-messages.e2e.ts counts them).
//   Unread notifications: most accounts have some.
//
// Payments are Stripe, faked (e2e_server/fake_stripe.rs): checkout links lead to its "Fake
// Stripe Checkout" page, whose Pay button completes the payment, sends Slop the webhooks and
// returns to the site (donations open it in a new tab). Stripe sign-up links finish at once.
import { test as base, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { STATE } from "./setup";

export type Who =
  | "maker"
  | "player"
  | "admin"
  | "overdue"
  | "teen"
  | "maker2"
  | "creative"
  | "client"
  | "pendingCreative"
  | "moderator"
  | "reviewer"
  | "support"
  | "finance"
  | "security"
  | "suspended";

export type Id =
  | Who
  | "sam"
  | "listedApp"
  | "listedAppId"
  | "crashReview"
  | "bugReport"
  | "fixedBugReport"
  | "donation"
  | "serverDonation"
  | "maker2App"
  | "releaseInReview"
  | "releaseInReviewApp"
  | "communityBuild"
  | "mod"
  | "draftApp"
  | "frozenApp"
  | "delistedApp"
  | "openBounty"
  | "claimedBounty"
  | "creativeHandle"
  | "pendingCreativeHandle"
  | "clientApp"
  | "deliveredJob"
  | "workingJob"
  | "requestedJob"
  | "problemJob"
  | "completedJob"
  | "inviteTeam"
  | "openTeam"
  | "directChat"
  | "teamChat"
  | "report"
  | "suspensionDecision"
  | "delistDecision"
  | "staffAction";

export interface Server {
  url: string;
  /** The fake GitHub (release files). */
  fake: string;
  /** The fake Stripe (its checkout pages). */
  stripe: string;
  cookie_name: string;
  cookies: Record<Who, string>;
  ids: Record<Id, string>;
}

export const server: Server = JSON.parse(readFileSync(STATE, "utf8"));
export const ids = server.ids;

/** Signs the page's browser in as one of the seeded accounts. */
export async function signIn(page: Page, who: Who) {
  await page.context().addCookies([
    { name: server.cookie_name, value: server.cookies[who], url: server.url, httpOnly: true, sameSite: "Lax" },
  ]);
}

export const test = base.extend<{ closeOwnWindows: void }>({
  baseURL: async ({}, use) => use(server.url),
  // Tests open windows of their own for other people (browser.newPage()). Each is closed when its
  // test ends: left open, hundreds pile up over a run and the browser starts dropping page loads.
  closeOwnWindows: [
    async ({ browser }, use) => {
      const before = new Set(browser.contexts());
      await use();
      await Promise.all(browser.contexts().filter((c) => !before.has(c)).map((c) => c.close().catch(() => {})));
    },
    { auto: true },
  ],
});
export { expect } from "@playwright/test";
