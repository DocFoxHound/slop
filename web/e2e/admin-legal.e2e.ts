// SPDX-License-Identifier: AGPL-3.0-or-later
// Terms and policies, and legal requests: staff announce a new Terms version (everyone is told in
// the bell, people accept it ahead of time) and withdraw it again before it takes effect; staff log a
// request from an authority, answer it, and act on a removal order within the hour.
import { closePages, newPage, newTeam, seeDecisions, tag } from "./admin-helpers";
import { expect, ids, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("a new Terms version: previewed, announced, accepted ahead of time, then withdrawn", async ({ browser }) => {
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=docs");
  const terms = admin.locator("section").filter({ has: admin.getByRole("heading", { name: "Terms of Service", level: 2 }) });
  await expect(terms.getByText(/People accept it\. In effect: .* since/)).toBeVisible();
  await terms.getByRole("button", { name: "Write a new version" }).click();
  const publish = terms.getByRole("button", { name: "Publish with 15 days' notice" });
  const text = terms.getByRole("textbox", { name: "The whole text (Markdown)" });
  const words = `## Browser tests ${tag()}\n\nSlop's browser tests may read this section.`;
  await text.fill(`${await text.inputValue()}\n\n${words}`);
  // The preview shows it as people will read it.
  await terms.getByRole("tab", { name: "Preview" }).click();
  await expect(terms.getByRole("heading", { name: /^Browser tests / })).toBeVisible();
  await terms.getByRole("tab", { name: "Edit" }).click();
  await expect(publish).toBeDisabled();
  await terms.getByRole("textbox", { name: /What changed, in plain words/ }).fill("Adds a section about SLAPorSLOP's browser tests.");
  await publish.click();
  const announced = terms.getByRole("status").filter({ hasText: "Announced:" });
  await expect(announced).toContainText("Adds a section about SLAPorSLOP's browser tests.");
  await expect(announced.getByText(/published from here by Ada Admin/)).toBeVisible();
  await terms.getByText(/The newest version as a file/).click();
  await expect(terms.getByRole("textbox", { name: "Terms of Service as a file" })).toHaveValue(/summary: Adds a section about SLAPorSLOP's browser tests\.\n/);
  // One change at a time: there's nothing to write until this one is in effect or withdrawn.
  await expect(terms.getByRole("button", { name: "Write a new version" })).toHaveCount(0);

  // Everyone is told once, in the bell, not with a banner; they accept it from their account.
  const mo = await newPage(browser);
  await signIn(mo, "moderator");
  await mo.goto("/");
  await expect(mo.getByText(/The Terms of Service changes on/)).toHaveCount(0);
  await mo.goto("/notifications/policies");
  const told = mo.getByRole("listitem").filter({ hasText: /The Terms of Service changes on .*, and you'll be asked to accept it then/ });
  await expect(told).toContainText("Adds a section about SLAPorSLOP's browser tests.");
  await expect(told).toHaveClass(/unread/);
  await told.getByRole("button", { name: /The Terms of Service changes on/ }).click();
  await expect(mo).toHaveURL(/\/legal\/terms\?version=/);
  await mo.goto("/account#agreements");
  const agreements = mo.locator("#agreements");
  await expect(agreements.getByText(/changes .*: Adds a section about SLAPorSLOP's browser tests\./)).toBeVisible();
  await agreements.getByRole("button", { name: "Review and accept" }).click();
  const dialog = mo.getByRole("dialog", { name: "Terms of Service" });
  await expect(dialog.getByRole("heading", { name: "Before you continue: the Terms of Service" })).toBeVisible();
  await expect(dialog.getByText(/takes effect on .* · What changed: Adds a section about SLAPorSLOP's browser tests\./)).toBeVisible();
  const accept = dialog.getByRole("button", { name: "Accept and continue" });
  await expect(accept).toBeDisabled();
  await dialog.getByRole("checkbox", { name: "I've read and accept the Terms of Service." }).check();
  await accept.click();
  await expect(dialog).toHaveCount(0);
  await expect(agreements.getByText("You're up to date with everything you've agreed to.")).toBeVisible();
  // The public list says it's coming.
  await mo.goto("/legal");
  await expect(mo.getByText(/Changes on .*: Adds a section about SLAPorSLOP's browser tests\./)).toBeVisible();

  // Withdrawn before it takes effect: gone for everyone.
  await terms.getByRole("button", { name: "Withdraw it" }).click();
  await terms.getByRole("textbox", { name: "Withdraw it: reason" }).fill("Published by the browser tests.");
  await terms.getByRole("button", { name: "Withdraw it" }).click();
  await expect(announced).toHaveCount(0);
  await expect(terms.getByRole("button", { name: "Write a new version" })).toBeVisible();
  await mo.goto("/legal");
  await expect(mo.getByText(/Adds a section about SLAPorSLOP's browser tests/)).toHaveCount(0);
  // Its notification goes too: it would lead nowhere.
  const player = await newPage(browser);
  await signIn(player, "player");
  await player.goto("/notifications/policies");
  await expect(player.getByRole("heading", { name: "Terms and policies" })).toBeVisible();
  await expect(player.getByText(/The Terms of Service changes on/)).toHaveCount(0);
});

test("legal requests: logged with what the law asks for, then answered", async ({ page }) => {
  await signIn(page, "admin");
  await page.goto("/admin?tab=legal");
  await page.getByRole("button", { name: "Log a request" }).click();
  const form = page.locator("form").filter({ has: page.getByRole("heading", { name: "Log a request" }) });
  const authority = `Leeds Crown Court ${tag()}`;
  await form.getByRole("textbox", { name: "Country" }).fill("G");
  await form.getByRole("textbox", { name: "From" }).fill(authority);
  await form.getByRole("combobox", { name: "Kind" }).selectOption({ label: "Court order" });
  await form.getByRole("textbox", { name: /Legal basis/ }).fill("Criminal Procedure Rules, r. 47");
  await form.getByRole("textbox", { name: "What they ask for" }).fill("Sign-in records for one account, January to March.");
  // An account must be an ID or a profile link.
  await form.getByRole("textbox", { name: /Account ID/ }).fill("Olly Overdue");
  await form.getByRole("button", { name: "Log it" }).click();
  await expect(form.getByRole("alert")).toHaveText("Paste the account's ID, or the link to their profile (…/u/…).");
  await form.getByRole("textbox", { name: /Account ID/ }).fill(`/u/${ids.overdue}`);
  await form.getByRole("button", { name: "Log it" }).click();
  await expect(form.getByRole("alert")).toHaveText("a two-letter country code");
  await form.getByRole("textbox", { name: "Country" }).fill("gb");
  await form.getByRole("button", { name: "Log it" }).click();
  await expect(form).toHaveCount(0);

  const logged = page.getByRole("listitem").filter({ hasText: authority });
  await expect(logged.getByText("OPEN", { exact: true })).toBeVisible();
  await expect(logged.getByText(/^LR-[0-9A-F]+ · Court order$/)).toBeVisible();
  await expect(logged.getByText(new RegExp(`GB · ${authority} · received`))).toBeVisible();
  await expect(logged.getByText("Criminal Procedure Rules, r. 47")).toBeVisible();
  await expect(logged.getByText("Olly Overdue")).toBeVisible();

  // The answer: what SLAPorSLOP gave, and whether the person was told.
  await logged.getByText("Record the answer").click();
  await logged.getByRole("combobox", { name: "Outcome" }).selectOption({ label: "Partly answered" });
  await logged.getByRole("textbox", { name: /What SLAPorSLOP gave/ }).fill("Sign-in dates only; SLAPorSLOP keeps no addresses.");
  await expect(logged.getByRole("combobox", { name: "The person" })).toHaveValue("notified");
  await logged.getByRole("button", { name: "Save the answer" }).click();
  await expect(logged.getByText("PARTLY ANSWERED")).toBeVisible();
  await expect(logged.getByText(/Ada Admin .* · person notified/)).toBeVisible();
  await expect(logged.getByText("Record the answer")).toHaveCount(0);

  // An emergency request is due within eight hours.
  await page.getByRole("button", { name: "Log a request" }).click();
  const emergency = `Metropolitan Police ${tag()}`;
  await form.getByRole("textbox", { name: "Country" }).fill("GB");
  await form.getByRole("textbox", { name: "From" }).fill(emergency);
  await form.getByRole("combobox", { name: "Kind" }).selectOption({ label: "Emergency request (8 hours)" });
  await form.getByRole("textbox", { name: "What they ask for" }).fill("Who runs a team that threatened a school.");
  await form.getByRole("button", { name: "Log it" }).click();
  await expect(page.getByRole("listitem").filter({ hasText: emergency }).getByText(/· due /)).toBeVisible();
});

test("a terrorist content removal order: down at once, timed, and logged", async ({ browser }) => {
  const t = tag();
  const team = `adm-tco-${t}`;
  await newTeam("overdue", team, `Recruiters ${t}`);
  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await admin.goto("/admin?tab=legal");
  await admin.getByRole("button", { name: "Terrorist content removal order" }).click();
  const form = admin.locator("form").filter({ has: admin.getByRole("heading", { name: "Removal order: act within the hour" }) });
  // It arrived 30 minutes ago.
  const received = new Date(Date.now() - 30 * 60_000);
  const local = new Date(received.getTime() - received.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
  await form.getByLabel("Received").fill(local);
  await form.getByRole("textbox", { name: "Country" }).fill("FR");
  await form.getByRole("textbox", { name: "Authority" }).fill("OCLCTIC");
  await form.getByRole("textbox", { name: /The order/ }).fill(`Order 2026-77 names the team page ${team}.`);
  await form.getByRole("combobox", { name: "What" }).selectOption({ label: "A team (its address)" });
  // Something that isn't there.
  await form.getByRole("textbox", { name: "Which" }).fill(`no-such-team-${t}`);
  await form.getByRole("button", { name: "Take it down now" }).click();
  await expect(form.getByRole("alert")).toHaveText("nothing on SLAPorSLOP by that name");
  await form.getByRole("textbox", { name: "Which" }).fill(team);
  await form.getByRole("button", { name: "Take it down now" }).click();
  const done = form.getByRole("status");
  await expect(done).toHaveText(new RegExp(`^Team: Recruiters ${t} is down \\(LR-[0-9A-F]+\\), (\\d+) minutes after the order arrived\\.$`));
  const minutes = Number((await done.innerText()).match(/(\d+) minutes/)![1]);
  expect(minutes).toBeGreaterThanOrEqual(30);
  expect(minutes).toBeLessThan(40);

  // Logged as answered; the team is hidden; its owner has the statement of reasons.
  const logged = admin.getByRole("listitem").filter({ hasText: "OCLCTIC" }).filter({ hasText: `Removed Team: Recruiters ${t}` });
  await expect(logged.getByText("ANSWERED", { exact: true })).toBeVisible();
  await expect(logged.getByText(/Terrorist content removal order \(1 hour\)/)).toBeVisible();
  const olly = await newPage(browser);
  await signIn(olly, "overdue");
  await olly.goto("/account#decisions");
  const decision = olly.getByRole("article", { name: `Team hidden: Team: Recruiters ${t}` });
  await expect(decision.getByText(/Regulation \(EU\) 2021\/784 \(terrorist content online\): removal order from OCLCTIC, FR/)).toBeVisible();
  await seeDecisions("overdue");
});
