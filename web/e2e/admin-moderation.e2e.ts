// SPDX-License-Identifier: AGPL-3.0-or-later
// Reports, decisions and appeals end to end: someone reports, staff act or dismiss, the person
// reads the statement of reasons on their account page and appeals, staff keep or reverse it,
// and the reporter reads the outcome. The things reported are made here (Olly's teams and
// bounty, an app of Sec's); the seeded reports and appeals stay as they are.
import { call, closePages, listedApp, newPage, newTeam, ok, report, seeDecisions, tag } from "./admin-helpers";
import { expect, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("a copyright report acted on, a counter-notice appeal, and the decision kept", async ({ browser }) => {
  const t = tag();
  const team = `adm-mod-${t}`;
  const name = `Pirate Copies ${t}`;
  await newTeam("overdue", team, name);

  // Reported from the team's page, signed in: no name or email asked.
  const reporter = await newPage(browser);
  await signIn(reporter, "finance");
  await reporter.goto(`/teams/${team}`);
  await reporter.getByRole("button", { name: "⚑ Report" }).click();
  const dialog = reporter.getByRole("dialog", { name: "Report this team" });
  await expect(dialog.getByRole("textbox", { name: /Your name/ })).toHaveCount(0);
  await expect(dialog.getByRole("textbox", { name: /Your email/ })).toHaveCount(0);
  await dialog.getByRole("radio", { name: "Copyright or trademark infringement" }).check();
  await dialog.getByRole("textbox", { name: /What did you see/ }).fill("The team's banner is our studio's logo.");
  await dialog.getByRole("textbox", { name: /The work that's being copied/ }).fill("The Lantern Studio logo, registered 2024.");
  await dialog.getByRole("textbox", { name: /Who owns it/ }).fill("Lantern Studio Ltd");
  await dialog.getByRole("textbox", { name: /Your postal address/ }).fill("1 Harbour Road, Bristol");
  await dialog.getByRole("textbox", { name: /Your phone number/ }).fill("+44 117 000 0000");
  await dialog.getByRole("checkbox", { name: /good faith that this use isn't authorised/ }).check();
  await dialog.getByRole("checkbox", { name: /under penalty of perjury, I'm the owner/ }).check();
  await dialog.getByRole("textbox", { name: /as your signature/ }).fill("Fin Finance");
  await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("heading", { name: "Thanks: we've got your report" })).toBeVisible();
  const reference = (await dialog.locator("b.mono").innerText()).trim();
  await expect(dialog.getByRole("link", { name: "Your reports" })).toBeVisible();
  await dialog.getByRole("button", { name: "Close" }).click();

  // Staff decide: hide the team, under the copyright rule and the law.
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue");
  const card = admin.getByRole("listitem").filter({ hasText: reference });
  await expect(card.getByText("SERIOUS", { exact: true })).toBeVisible();
  await expect(card.getByText("Copyright or trademark infringement")).toBeVisible();
  await expect(card.getByText(/Work: The Lantern Studio logo, registered 2024\. · owner: Lantern Studio Ltd/)).toBeVisible();
  await expect(card.getByText(/Olly Overdue · \d+ earlier decision/)).toBeVisible();
  await card.getByRole("button", { name: "Decide" }).click();
  await expect(card.getByRole("combobox", { name: "What to do" })).toHaveValue("hide_team");
  await expect(card.getByRole("combobox", { name: "The rule it breaks" })).toHaveValue("copyright");
  const act = card.getByRole("button", { name: "Hide the team" });
  await expect(act).toBeDisabled();
  await card.getByRole("textbox", { name: /The law/ }).fill("17 U.S.C. §512");
  await card.getByRole("textbox", { name: /The facts, as Olly Overdue will read them/ }).fill("The banner is Lantern Studio's registered logo.");
  await card.getByRole("textbox", { name: "What the reporter is told" }).fill("We hid the team's page.");
  await act.click();
  await expect(card).toHaveCount(0);

  // The reporter reads the outcome.
  await reporter.goto("/account#reports");
  const mine = reporter.locator("#reports").getByRole("listitem").filter({ hasText: reference });
  await expect(mine.getByText("ACTED ON")).toBeVisible();
  await expect(mine.getByText("We hid the team's page.")).toBeVisible();

  // Olly is told on every page, reads the statement of reasons, and appeals with a counter-notice.
  const olly = await newPage(browser);
  await signIn(olly, "overdue");
  await olly.goto("/");
  await olly.getByRole("link", { name: "See what, why, and how to appeal" }).click();
  const decision = olly.getByRole("article", { name: `Team hidden: Team: ${name}` });
  await expect(decision.getByText("NEW", { exact: true })).toBeVisible();
  await expect(decision.getByText("decided by a person, not automated tools")).toBeVisible();
  await expect(decision.getByText("The banner is Lantern Studio's registered logo.")).toBeVisible();
  await expect(decision.getByRole("link", { name: "Content Policy: Copyright and trademarks" })).toHaveAttribute(
    "href",
    "/legal/content#copyright-and-trademarks",
  );
  await expect(decision.getByText("17 U.S.C. §512")).toBeVisible();
  await decision.getByRole("button", { name: "Appeal this decision" }).click();
  await decision.getByRole("textbox", { name: /Why should we look again/ }).fill("I drew the banner myself in 2023.");
  await decision.getByRole("textbox", { name: "Your full name", exact: true }).fill("Olly Overdue");
  await decision.getByRole("textbox", { name: "Your postal address" }).fill("2 Mill Lane, Leeds");
  await decision.getByRole("textbox", { name: "Your phone number" }).fill("+44 113 000 0000");
  await decision.getByRole("textbox", { name: "Your full name, as your signature" }).fill("Olly Overdue");
  // Both statements are needed.
  await decision.getByRole("button", { name: "Send appeal" }).click();
  await expect(decision.getByRole("alert")).toHaveText("a counter-notice needs both statements");
  await decision.getByRole("checkbox", { name: /removed by mistake or misidentification/ }).check();
  await decision.getByRole("checkbox", { name: /I consent to the jurisdiction/ }).check();
  await decision.getByRole("button", { name: "Send appeal" }).click();
  await expect(decision.getByText(/You appealed on .*\. A person will look at it again and tell you here\./)).toBeVisible();
  await expect(decision.getByRole("button", { name: "Appeal this decision" })).toHaveCount(0);
  // The notice on every page is gone once read.
  await expect(olly.getByRole("link", { name: "See what, why, and how to appeal" })).toHaveCount(0);

  // Staff see the counter-notice and keep the decision.
  await admin.goto("/admin?tab=queue#appeals");
  // The same address as before but for the part: load the queue again.
  await admin.reload();
  const appeal = admin.getByRole("listitem").filter({ hasText: `Team: ${name}` });
  await expect(appeal.getByText("I drew the banner myself in 2023.")).toBeVisible();
  await expect(appeal.getByText(/Olly Overdue, 2 Mill Lane, Leeds, \+44 113 000 0000 · both statements made\./)).toBeVisible();
  await expect(appeal.getByText(/Send these details to the person who reported it/)).toBeVisible();
  await expect(appeal.getByRole("button", { name: "Keep it" })).toBeDisabled();
  await appeal.getByRole("textbox", { name: /Why \(the person reads this\)/ }).fill("The banner matches the studio's 2019 filing.");
  await appeal.getByRole("button", { name: "Keep it" }).click();
  await expect(appeal).toHaveCount(0);

  await olly.reload();
  await expect(decision.getByText(/Your appeal was looked at, and the decision stands\. The banner matches the studio's 2019 filing\./)).toBeVisible();
  await expect(decision.getByText("REVERSED")).toHaveCount(0);
  await seeDecisions("overdue");
});

test("a threat report closed with no action, passed on to the police; the reporter is told", async ({ browser }) => {
  const t = tag();
  const team = `adm-thr-${t}`;
  await newTeam("overdue", team, `Night Watch ${t}`);
  const reference = await report("client", "team", team, "threat", "The team page says they'll find where I live.");

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue");
  const card = admin.getByRole("listitem").filter({ hasText: reference });
  await expect(card.getByText("URGENT", { exact: true })).toBeVisible();
  await expect(card.getByText(/^R-.* · due .*/)).toBeVisible();
  await card.getByRole("button", { name: "Decide" }).click();
  // Threats can be passed on, with a reference.
  const passed = card.getByRole("textbox", { name: /Passed on to \(NCMEC, NCA, police\) and reference/ });
  await expect(passed).toBeVisible();
  await passed.fill("West Yorkshire Police, ref 13/123456/26");
  const none = card.getByRole("button", { name: "No action" });
  await expect(none).toBeDisabled();
  await card.getByRole("textbox", { name: "What the reporter is told" }).fill("The page quotes a game's dialogue; we passed your report to the police too.");
  await none.click();
  await expect(card).toHaveCount(0);
  const audit = await ok("admin", "GET", "/v1/admin/audit?q=report.dismiss");
  expect(audit.length).toBeGreaterThan(0);

  const reporter = await newPage(browser);
  await signIn(reporter, "client");
  await reporter.goto("/notifications");
  await expect(reporter.getByText(`Your report ${reference} was answered`)).toBeVisible();
  await reporter.goto("/account#reports");
  const mine = reporter.locator("#reports").getByRole("listitem").filter({ hasText: reference });
  await expect(mine.getByText("NO ACTION")).toBeVisible();
  await expect(mine.getByText(/we passed your report to the police too/)).toBeVisible();
});

test("a bounty: its opener can't report it; staff can hide it", async ({ browser }) => {
  const t = tag();
  const bounty = await ok("overdue", "POST", "/v1/apps/mias-first/bounties", {
    title: `Free coins ${t}`,
    body: "Pledge here and get coins in another game.",
    currency: "usd",
  });

  // Reporting your own says why it can't be sent.
  const olly = await newPage(browser);
  await signIn(olly, "overdue");
  await olly.goto(`/bounties/${bounty.id}`);
  await olly.getByRole("button", { name: "⚑ Report" }).click();
  const dialog = olly.getByRole("dialog", { name: "Report this bounty" });
  await dialog.getByRole("radio", { name: "Spam" }).check();
  await dialog.getByRole("textbox", { name: /What did you see/ }).fill("Testing.");
  await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("alert")).toHaveText("you can't report your own");

  const reference = await report("support", "bounty", bounty.id, "scam", "It promises coins in another game for pledges.");
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue");
  const card = admin.getByRole("listitem").filter({ hasText: reference });
  await card.getByRole("button", { name: "Decide" }).click();
  const what = card.getByRole("combobox", { name: "What to do" });
  await expect(what.locator("option")).toHaveText(["Hide the bounty", "Warn them (no restriction)", "Suspend their account"]);
  await expect(card.getByRole("combobox", { name: "The rule it breaks" })).toHaveValue("scam");
  await card.getByRole("textbox", { name: /The facts/ }).fill("The bounty asks for pledges in exchange for coins elsewhere.");
  await card.getByRole("textbox", { name: "What the reporter is told" }).fill("We hid the bounty.");
  await card.getByRole("button", { name: "Hide the bounty" }).click();
  await expect(card).toHaveCount(0);
  // Hidden from everyone, its opener too.
  expect((await call("overdue", "GET", `/v1/bounties/${bounty.id}`)).status).toBe(404);
  await olly.reload();
  await expect(olly.getByText("We couldn't find that.")).toBeVisible();
  await seeDecisions("overdue");
});

test("an appeal reversed while the app is frozen: it stays out, and staff and maker are told why", async ({ browser }) => {
  const t = tag();
  const app = { slug: `adm-apl-${t}`, title: `Comet ${t}` };
  await listedApp(app, "security");
  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/delist`, { rule: "misleading", reason: "The page shows a level that isn't there." });

  // The maker appeals from their account page.
  const maker = await newPage(browser);
  await signIn(maker, "security");
  await maker.goto("/account#decisions");
  const decision = maker.getByRole("article", { name: `Taken out of the store: App: ${app.title}` });
  await expect(decision.getByText(/until .*/)).toBeVisible();
  await decision.getByRole("button", { name: "Appeal this decision" }).click();
  // Not a copyright decision: no counter-notice.
  await expect(decision.getByText("Counter-notice")).toHaveCount(0);
  await decision.getByRole("button", { name: "Cancel" }).click();
  await decision.getByRole("button", { name: "Appeal this decision" }).click();
  await decision.getByRole("textbox", { name: /Why should we look again/ }).fill("The level is in 1.0.0: here's a video.");
  await decision.getByRole("button", { name: "Send appeal" }).click();
  await expect(decision.getByText(/You appealed on/)).toBeVisible();

  // Meanwhile staff froze it: reversing can't put it back yet.
  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/freeze`, { reason: "Checking its new release." });
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=queue");
  const appeal = admin
    .getByRole("listitem")
    .filter({ hasText: `App: ${app.title}` })
    .filter({ has: admin.getByRole("button", { name: "Reverse the decision" }) });
  await expect(appeal.getByText("Taken out of the store", { exact: true })).toBeVisible();
  await expect(appeal.getByText(/Misleading pages: The page shows a level that isn't there\./)).toBeVisible();
  await appeal.getByRole("textbox", { name: /Why \(the person reads this\)/ }).fill("The video shows the level.");
  await appeal.getByRole("button", { name: "Reverse the decision" }).click();
  await expect(appeal).toHaveCount(0);
  await expect(admin.getByRole("status").filter({ hasText: "Reversed, but" })).toHaveText(
    `Reversed, but App: ${app.title} can't go back in the store yet. The maker was told: It stays out of the store ` +
      "while it's frozen (Checking its new release.). It goes back in once Slop can read its repository again.",
  );

  await maker.reload();
  await expect(decision.getByText("REVERSED", { exact: true })).toBeVisible();
  await expect(decision.getByText(/Your appeal succeeded: the decision is reversed\. The video shows the level\./)).toBeVisible();
  await ok("admin", "POST", `/v1/admin/apps/${app.slug}/unfreeze`, { note: "Release checked." });
  await seeDecisions("security");
});

test("the report form: already reported, too many, and a signed-out reporter's details", async ({ browser }) => {
  const t = tag();
  const team = `adm-rep-${t}`;
  await newTeam("overdue", team, `Spam Squad ${t}`);
  const page = await newPage(browser);
  await signIn(page, "finance");
  await page.goto(`/teams/${team}`);
  const send = async () => {
    await page.getByRole("button", { name: "⚑ Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this team" });
    await dialog.getByRole("radio", { name: "Spam" }).check();
    await dialog.getByRole("textbox", { name: /What did you see/ }).fill("Links to a coin giveaway.");
    await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
    await dialog.getByRole("button", { name: "Send report" }).click();
    return dialog;
  };
  const first = await send();
  await expect(first.getByRole("heading", { name: "Thanks: we've got your report" })).toBeVisible();
  await first.getByRole("button", { name: "Close" }).click();
  // Again, while it waits.
  const again = await send();
  await expect(again.getByRole("alert")).toHaveText("you've already reported this, and it's waiting for review");
  await again.getByRole("button", { name: "Cancel" }).click();

  // Too many reports for now (the limit is ten an hour: the answer is faked).
  await page.route("**/v1/reports", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 429,
          headers: { "retry-after": "600" },
          json: { error: { code: "rate_limited", message: "too many reports for now: try again in a while" } },
        })
      : route.fallback(),
  );
  const limited = await send();
  await expect(limited.getByRole("alert")).toHaveText("too many reports for now: try again in a while");
  await limited.getByRole("button", { name: "Cancel" }).click();

  // Signed out, Slop asks who's reporting.
  const visitor = await newPage(browser);
  await visitor.goto(`/teams/${team}`);
  await visitor.getByRole("button", { name: "⚑ Report" }).click();
  const dialog = visitor.getByRole("dialog", { name: "Report this team" });
  await expect(dialog.getByRole("textbox", { name: /Your name/ })).toBeVisible();
  await expect(dialog.getByRole("textbox", { name: /Your email/ })).toBeVisible();
});
