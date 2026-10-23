// SPDX-License-Identifier: AGPL-3.0-or-later
// Moderation from the person's side and the legal side: decisions about you and appealing them,
// the reports you sent, the Report dialog, the legal request log and the transparency report
// (components/AccountSections.tsx, Report.tsx, LegalRequests.tsx, pages/Transparency.tsx).
// Fin Finance is the account decided about here; every appeal made is answered again, every
// report made is closed, so the admin queues end as the seed left them.
import { apiError, call, fake, unique } from "./accounts-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test.describe("decisions about you", () => {
  test("are announced, explained, appealed once each, and answered", async ({ page }) => {
    const spam = await call<{ decision: string }>("admin", "POST", `/v1/admin/users/${ids.finance}/decide`, {
      action: "warn",
      rule: "spam",
      facts: "Posted the same coupon link in thirty reviews.",
      law: null,
    });
    const copyright = await call<{ decision: string }>("admin", "POST", `/v1/admin/users/${ids.finance}/decide`, {
      action: "warn",
      rule: "copyright",
      facts: "Used a studio's logo as a team picture after a notice from the studio.",
      law: "17 U.S.C. §512(c)",
    });
    const answered: string[] = [];
    try {
      await signIn(page, "finance");
      await page.goto("/bounties");
      const notice = page.locator(".account-notices");
      await expect(notice).toContainText("Slop made 2 decisions about your content.");
      await notice.getByRole("link", { name: "See what, why, and how to appeal" }).click();
      await expect(page).toHaveURL(/\/account#decisions$/);
      const section = page.locator("#decisions");
      await expect(section.getByRole("heading", { name: "Decisions about your content or account" })).toBeInViewport();
      // Seen now: the notice goes.
      await expect(notice).toHaveCount(0);

      const spamCard = section.getByRole("article", { name: "Warning: Fin Finance" }).filter({ hasText: "coupon link" });
      await expect(spamCard.getByText("NEW", { exact: true })).toBeVisible();
      await expect(spamCard).toContainText("decided by a person, not automated tools");
      await expect(spamCard).toContainText(/D-[0-9A-Z]{4}/);
      await expect(spamCard.getByRole("link", { name: "Content Policy: Spam" })).toHaveAttribute("href", "/legal/content#spam");
      await expect(spamCard.getByText("The law")).toHaveCount(0);
      const crCard = section.getByRole("article").filter({ hasText: "studio's logo" });
      await expect(crCard.getByRole("link", { name: "Content Policy: Copyright and trademarks" })).toHaveAttribute(
        "href",
        "/legal/content#copyright-and-trademarks",
      );
      await expect(crCard).toContainText("17 U.S.C. §512(c)");

      // An ordinary appeal: why, then it waits.
      await spamCard.getByRole("button", { name: "Appeal this decision" }).click();
      await expect(spamCard).not.toContainText("Counter-notice");
      const send = spamCard.getByRole("button", { name: "Send appeal" });
      await expect(send).toBeDisabled();
      await spamCard.getByRole("button", { name: "Cancel" }).click();
      await expect(send).toHaveCount(0);
      await spamCard.getByRole("button", { name: "Appeal this decision" }).click();
      await spamCard.getByRole("textbox", { name: /Why should we look again/ }).fill("Those were links to my own game's sale.");
      await send.click();
      await expect(spamCard).toContainText("A person will look at it again and tell you here.");
      await expect(spamCard.getByRole("button", { name: "Appeal this decision" })).toHaveCount(0);

      // A copyright appeal is a counter-notice: the server checks each part.
      await crCard.getByRole("button", { name: "Appeal this decision" }).click();
      const form = crCard.locator("form");
      await expect(form.getByRole("group", { name: "Counter-notice" })).toBeVisible();
      await form.getByRole("textbox", { name: /Why should we look again/ }).fill("The logo is my own studio's; I'm its owner.");
      const fullName = form.getByRole("textbox", { name: /^Your full name(?!, as)/ });
      await fullName.fill("   ");
      await form.getByRole("textbox", { name: "Your postal address" }).fill("1 Main St, Springfield");
      await form.getByRole("textbox", { name: "Your phone number" }).fill("+1 555 0100");
      await form.getByRole("textbox", { name: "Your full name, as your signature" }).fill("Fin Finance");
      await form.getByRole("button", { name: "Send appeal" }).click();
      // By the field it's about.
      await expect(form.getByRole("alert")).toHaveText("this can't be empty");
      await expect(fullName).toHaveAccessibleName(/this can't be empty/);
      await fullName.fill("Fin Finance");
      await form.getByRole("button", { name: "Send appeal" }).click();
      await expect(form.getByRole("alert")).toHaveText("a counter-notice needs both statements");
      await form.getByRole("checkbox", { name: /Under penalty of perjury/ }).check();
      await form.getByRole("checkbox", { name: /I consent to the jurisdiction/ }).check();
      await form.getByRole("button", { name: "Send appeal" }).click();
      await expect(crCard).toContainText("A person will look at it again and tell you here.");

      // Staff answer both.
      await call("admin", "POST", `/v1/admin/decisions/${spam.decision}/appeal`, {
        reverse: true,
        reason: "We looked again: the links were to your own game.",
      });
      answered.push(spam.decision);
      await call("admin", "POST", `/v1/admin/decisions/${copyright.decision}/appeal`, {
        reverse: false,
        reason: "The studio showed it owns the logo.",
      });
      answered.push(copyright.decision);
      await page.reload();
      await expect(spamCard.getByText("REVERSED", { exact: true })).toBeVisible();
      await expect(spamCard).toContainText("Your appeal succeeded: the decision is reversed. We looked again: the links were to your own game.");
      await expect(crCard).toContainText("Your appeal was looked at, and the decision stands. The studio showed it owns the logo.");
      // An answer is news: marked new once more, then seen.
      await expect(section.getByText("NEW", { exact: true })).toHaveCount(2);
      await page.reload();
      await expect(crCard).toBeVisible();
      await expect(section.getByText("NEW", { exact: true })).toHaveCount(0);
    } finally {
      // An appeal left waiting would add to the admins' queue.
      for (const id of [spam.decision, copyright.decision].filter((d) => !answered.includes(d))) {
        await call("admin", "POST", `/v1/admin/decisions/${id}/appeal`, { reverse: true, reason: "Test clean-up." }).catch(() => undefined);
      }
    }
  });

  test("an account without decisions has no decisions section", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account");
    await expect(page.getByRole("heading", { name: "Terms and policies" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Decisions about your content or account" })).toHaveCount(0);
  });
});

test.describe("your reports", () => {
  test("one waiting for review shows its reference", async ({ page }) => {
    await signIn(page, "client");
    await page.goto("/account#reports");
    const report = page.locator("#reports").getByRole("listitem").filter({ hasText: "App: Frozen Fjord" });
    await expect(report).toContainText("Malware, hidden mining or data theft");
    await expect(report).toContainText(ids.report);
    await expect(report).toContainText("WAITING FOR REVIEW");
  });

  test("one answered shows the outcome", async ({ page }) => {
    const slug = unique("acc-report");
    await call("security", "POST", "/v1/teams", { slug, name: "Report Target", kind: "player" });
    const sent = await call<{ reference: string }>("support", "POST", "/v1/reports", {
      kind: "team",
      subject: slug,
      reason: "spam",
      details: "The team's description is only an advert.",
      good_faith: true,
    });
    const queue = await call<{ reports: Array<{ id: string; reference: string }> }>("admin", "GET", "/v1/admin/queue");
    const id = queue.reports.find((r) => r.reference === sent.reference)!.id;
    await call("admin", "POST", `/v1/admin/reports/${id}/dismiss`, { outcome: "It's an ordinary team description." });
    await signIn(page, "support");
    await page.goto("/account#reports");
    const report = page.locator("#reports").getByRole("listitem").filter({ hasText: sent.reference });
    await expect(report).toContainText("Team: Report Target");
    await expect(report).toContainText("It's an ordinary team description.");
    await expect(report).toContainText(/answered \w{3} \d+, \d{4}/);
    await expect(report).toContainText("NO ACTION");
  });

  test("with none sent, there's no reports section", async ({ page }) => {
    await signIn(page, "finance");
    await page.goto("/account");
    await expect(page.getByRole("heading", { name: "Terms and policies" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Your reports" })).toHaveCount(0);
  });
});

test.describe("the Report dialog", () => {
  test("signed out: the server's complaints show by each field, and a copyright notice is sent whole", async ({ page }) => {
    // The last send is answered by a stand-in, so no report joins the admins' queue.
    await page.goto("/app/castle-rock");
    await page.getByRole("button", { name: "⚑ Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this app" });
    await dialog.getByRole("radio", { name: /Copyright/ }).check();
    await dialog.getByRole("textbox", { name: /What did you see/ }).fill("The title screen is a copy of my game's.");
    await dialog.getByRole("textbox", { name: "The work that's being copied" }).fill("   ");
    await dialog.getByRole("textbox", { name: "Your postal address" }).fill("2 High St, Leeds");
    await dialog.getByRole("textbox", { name: "Your phone number" }).fill("+44 113 496 0000");
    await dialog.getByRole("textbox", { name: "Your full name, as your signature" }).fill("Ana Artist");
    await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
    const send = dialog.getByRole("button", { name: "Send report" });
    await send.click();
    await expect(dialog.getByRole("alert")).toHaveText("add your name");
    await dialog.getByRole("textbox", { name: /Your name/ }).fill("Ana Artist");
    await send.click();
    await expect(dialog.getByRole("alert")).toHaveText("add an email we can reach you at");
    await dialog.getByRole("textbox", { name: /Your email/ }).fill("ana@example.com");
    await send.click();
    // By the field it's about.
    await expect(dialog.locator("fieldset.report-copyright").getByRole("alert")).toBeVisible();
    await dialog.getByRole("textbox", { name: /^The work that's being copied/ }).fill("My game Pixel Keep's title screen.");
    await send.click();
    await expect(dialog.getByRole("alert")).toHaveText("a copyright notice needs both statements");
    await dialog.getByRole("checkbox", { name: /I believe in good faith that this use isn't authorised/ }).check();
    await dialog.getByRole("checkbox", { name: /under penalty of perjury, I'm the owner/ }).check();

    const sent = await fake(page, "POST", "/v1/reports", { status: 201, body: { reference: "R-TEST-0001", due_at: "2026-10-30T12:00:00Z" } });
    await send.click();
    await expect(dialog.getByRole("heading", { name: "Thanks: we've got your report" })).toBeVisible();
    await expect(dialog).toContainText("Your reference is R-TEST-0001. A person will review it by Oct 30, 2026.");
    await expect(dialog).toContainText("Keep the reference if you need to ask us about it.");
    expect(sent.calls[0].body).toEqual({
      kind: "app",
      subject: "castle-rock",
      reason: "copyright",
      details: "The title screen is a copy of my game's.",
      good_faith: true,
      name: "Ana Artist",
      email: "ana@example.com",
      copyright: {
        work: "My game Pixel Keep's title screen.",
        owner: "",
        address: "2 High St, Leeds",
        phone: "+44 113 496 0000",
        good_faith: true,
        accurate_under_penalty: true,
        signature: "Ana Artist",
      },
    });
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();
  });

  test("a child abuse report doesn't need a name or email, and is urgent", async ({ page }) => {
    await page.goto("/app/castle-rock");
    await page.getByRole("button", { name: "⚑ Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this app" });
    await dialog.getByRole("radio", { name: "Child sexual abuse material" }).check();
    await expect(dialog.getByText("We review reports like this within 24 hours.")).toBeVisible();
    await expect(dialog.getByText("Your name (optional)")).toBeVisible();
    await expect(dialog.getByText("Your email (optional)")).toBeVisible();
    await dialog.getByRole("radio", { name: /Copyright/ }).check();
    await expect(dialog.getByText("Your name (optional)")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
  });

  test("signed in: no name or email, and the confirmation links to your reports", async ({ page }) => {
    const sent = await fake(page, "POST", "/v1/reports", { status: 201, body: { reference: "R-TEST-0002", due_at: "2026-10-30T12:00:00Z" } });
    await signIn(page, "support");
    await page.goto("/app/castle-rock");
    await page.getByRole("button", { name: "⚑ Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this app" });
    await expect(dialog.getByRole("textbox", { name: /Your email/ })).toHaveCount(0);
    const send = dialog.getByRole("button", { name: "Send report" });
    await expect(send).toBeDisabled();
    await dialog.getByRole("radio", { name: "A misleading page" }).check();
    await dialog.getByRole("textbox", { name: /What did you see/ }).fill("Test report that a stand-in answers.");
    await expect(send).toBeDisabled();
    await dialog.getByRole("checkbox", { name: /accurate and complete/ }).check();
    await send.click();
    await expect(dialog).toContainText("Your reference is R-TEST-0002.");
    expect(sent.calls[0].body).toEqual({
      kind: "app",
      subject: "castle-rock",
      reason: "misleading",
      details: "Test report that a stand-in answers.",
      good_faith: true,
    });
    await dialog.getByRole("link", { name: "Your reports" }).click();
    await expect(dialog).toBeHidden();
    await expect(page).toHaveURL(/\/account#reports$/);
  });

  test("reasons that won't load say so", async ({ page }) => {
    await fake(page, "GET", "/v1/reports/reasons", { status: 503, body: apiError("unavailable", "Slop is busy: try again in a minute") });
    await page.goto("/app/castle-rock");
    await page.getByRole("button", { name: "⚑ Report" }).click();
    const dialog = page.getByRole("dialog", { name: "Report this app" });
    await expect(dialog.getByRole("alert")).toHaveText("Slop is busy: try again in a minute");
    await expect(dialog.getByRole("radio")).toHaveCount(0);
  });
});

test.describe("legal requests and the transparency report", () => {
  test("staff log a request, record the answer, and act on a removal order; the yearly report counts them", async ({ page }) => {
    const slug = unique("acc-removal");
    await call("finance", "POST", "/v1/teams", { slug, name: "Removal Test Team", kind: "player" });
    await signIn(page, "admin");
    await page.goto("/admin?tab=legal");
    await expect(page.getByRole("tab", { name: "Legal requests" })).toHaveAttribute("aria-selected", "true");

    await page.getByRole("button", { name: "Log a request" }).click();
    const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "Log a request" }) });
    await form.getByRole("textbox", { name: "Country" }).fill("US");
    await form.getByRole("textbox", { name: "From" }).fill("Springfield Police Department");
    await form.getByRole("combobox", { name: "Kind" }).selectOption("subpoena");
    await form.getByRole("textbox", { name: "Legal basis (optional)" }).fill("Grand jury subpoena 26-114");
    await form.getByRole("textbox", { name: "What they ask for" }).fill("Sign-in records for one account, October 2026.");
    await form.getByRole("textbox", { name: /Account ID/ }).fill("01a00000-0000-7000-8000-000000000000");
    await form.getByRole("button", { name: "Log it" }).click();
    await expect(form.getByRole("alert")).toHaveText("no such account");
    await form.getByRole("textbox", { name: /Account ID/ }).fill(ids.finance);
    await form.getByRole("button", { name: "Log it" }).click();
    await expect(form).toHaveCount(0);

    const logged = page.getByRole("listitem").filter({ hasText: "Springfield Police Department" }).first();
    await expect(logged).toContainText("OPEN");
    await expect(logged).toContainText("Subpoena");
    await expect(logged).toContainText("Grand jury subpoena 26-114");
    await expect(logged).toContainText("Fin Finance");
    await logged.getByText("Record the answer").click();
    await logged.getByRole("combobox", { name: "Outcome" }).selectOption("partly_answered");
    await logged.getByRole("textbox", { name: "What Slop gave, or why not" }).fill("Sign-in dates only; Slop keeps no addresses.");
    await expect(logged.getByRole("combobox", { name: "The person" })).toHaveValue("notified");
    await logged.getByRole("button", { name: "Save the answer" }).click();
    await expect(logged).toContainText("PARTLY ANSWERED");
    await expect(logged).toContainText("Sign-in dates only; Slop keeps no addresses.");
    await expect(logged).toContainText("person notified");
    await expect(logged.getByText("Record the answer")).toHaveCount(0);

    await page.getByRole("button", { name: "Terrorist content removal order" }).click();
    const order = page.locator("form").filter({ has: page.getByRole("heading", { name: "Removal order: act within the hour" }) });
    await order.getByRole("textbox", { name: "Country" }).fill("FR");
    await order.getByRole("textbox", { name: "Authority" }).fill("OCLCTIC");
    await order.getByRole("textbox", { name: /The order/ }).fill("TCO-2026-0042 names the team page.");
    await order.getByRole("combobox", { name: "What" }).selectOption("team");
    await order.getByRole("textbox", { name: "Which" }).fill("no-such-team-anywhere");
    await order.getByRole("button", { name: "Take it down now" }).click();
    await expect(order.getByRole("alert")).toHaveText("nothing on Slop by that name");
    await order.getByRole("textbox", { name: "Which" }).fill(slug);
    await order.getByRole("button", { name: "Take it down now" }).click();
    await expect(order.getByRole("status")).toContainText(/Team: Removal Test Team is down \(LR-[0-9A-F]+\), \d+ minutes after the order arrived\./);
    await order.getByRole("button", { name: "Close" }).click();
    await expect(page.getByRole("listitem").filter({ hasText: "OCLCTIC" })).toContainText("Terrorist content removal order");

    // The team's owner sees it's hidden.
    const owner = await page.context().browser()!.newPage();
    await signIn(owner, "finance");
    await owner.goto(`/teams/${slug}`);
    await expect(owner.getByText("This team is hidden by a moderation decision. See your account page.")).toBeVisible();
    await owner.close();

    // Counts only, by year.
    await page.goto("/transparency");
    await expect(page.getByRole("heading", { name: "Transparency report" })).toBeVisible();
    const year = String(new Date().getUTCFullYear());
    await expect(page.getByRole("combobox", { name: "Year" })).toHaveValue(year);
    await expect(page.getByText(`${year} isn't over yet: these are the numbers so far.`)).toBeVisible();
    const table = (caption: string) => page.locator("table").filter({ has: page.locator("caption", { hasText: caption }) });
    await expect(table("By country").first().getByRole("row").filter({ hasText: "US" })).toBeVisible();
    await expect(table("By country").first().getByRole("row").filter({ hasText: "FR" })).toBeVisible();
    await expect(table("By kind").getByRole("row").filter({ hasText: "Subpoenas" })).toBeVisible();
    await expect(table("By kind").getByRole("row").filter({ hasText: "Terrorist content removal orders" })).toBeVisible();
    await expect(table("Decisions by action").getByRole("row").filter({ hasText: "Team hidden" })).toBeVisible();
    await expect(page.getByText(/Decisions made by automated tools alone: \d+\./)).toBeVisible();
  });

  test("staff without the sensitive permission can't read the log", async ({ page }) => {
    await signIn(page, "reviewer");
    await page.goto("/admin?tab=legal");
    await expect(page.getByRole("alert")).toContainText("you can't do this");
    await expect(page.getByRole("button", { name: "Log a request" })).toHaveCount(0);
  });

  test("a year with nothing says None", async ({ page }) => {
    await fake(page, "GET", /^\/v1\/transparency\/\d+$/, {
      status: 200,
      body: {
        complete: true,
        legal_requests: { by_country: [], by_kind: [], by_outcome: [] },
        reports: { by_reason: [], by_outcome: [], median_hours_to_decide: null },
        decisions: { by_action: [], automated: 0 },
        appeals: { by_outcome: [] },
      },
    });
    await page.goto("/transparency");
    await expect(page.locator("table").first().getByRole("cell", { name: "None" })).toBeVisible();
    await expect(page.getByText(/isn't over yet/)).toHaveCount(0);
    await expect(page.getByText(/Half of reports were decided/)).toHaveCount(0);
  });
});
