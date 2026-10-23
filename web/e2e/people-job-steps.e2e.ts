// SPDX-License-Identifier: AGPL-3.0-or-later
// A People job's other steps: changes to the agreement, the recorded conversation, problems
// (proposals, evidence, a mediator) and the ways a job ends early. Cleo hires Vera through the API
// and the two work it through in the browser; the seeded jobs stay as they are.
import { readFileSync } from "node:fs";
import type { Browser, Page } from "@playwright/test";
import { expect, ids, signIn, test, type Who } from "./fixtures";
import { call, hireVera, offerAndAccept, offerBody, payOnFakeStripe } from "./people-helpers";

async function as(browser: Browser, who: Who, path: string): Promise<Page> {
  const page = await browser.newPage();
  await signIn(page, who);
  await page.goto(path);
  return page;
}

test.afterAll(async () => {
  // Don't leave Cleo with open requests to Vera (she may have three at once).
  const jobs = await call<{ as_client: Array<{ id: string; status: string }> }>("client", "GET", "/v1/me/jobs");
  for (const j of jobs.as_client)
    if (["requested", "offered"].includes(j.status) && j.id !== ids.requestedJob)
      await call("client", "POST", `/v1/jobs/${j.id}/decline`, { note: "" });
});

test("changes to the agreement: asked for, declined, offered with a price, accepted and paid, withdrawn", async ({ browser }) => {
  test.slow();
  const job = await hireVera("Six wall torches, animated, 16×32.");
  await offerAndAccept(job, offerBody("Wall torches", "on_delivery"));
  const cleo = await as(browser, "client", `/jobs/${job}`);
  const vera = await as(browser, "creative", `/jobs/${job}`);
  const cleoChanges = cleo.locator(".changes-panel");
  const veraChanges = vera.locator(".changes-panel");

  // Cleo asks for a change: it needs words.
  await cleoChanges.getByRole("button", { name: "Ask for a change" }).click();
  const send = cleoChanges.getByRole("button", { name: "Ask for a change" });
  await expect(send).toBeDisabled();
  await cleoChanges.getByRole("textbox", { name: "Ask for a change" }).fill("Make them eight torches instead of six.");
  await send.click();
  await expect(cleoChanges.getByText("You asked:")).toBeVisible();
  await expect(cleoChanges).toContainText("Make them eight torches instead of six.");

  // Vera declines it: the work stays as agreed.
  await vera.reload();
  await expect(veraChanges).toContainText("They asked: Make them eight torches instead of six.");
  await veraChanges.getByRole("button", { name: "Decline the change" }).click();
  await veraChanges.getByRole("textbox", { name: "Decline the change" }).fill("Eight won't fit the date.");
  await veraChanges.getByRole("button", { name: "Decline the change" }).click();
  await expect(vera.locator(".timeline")).toContainText("declined the change");

  // Asked again, Vera writes a Change to Agreement with a price and a new date (the declined
  // request was change 1).
  await cleo.reload();
  await cleoChanges.getByRole("button", { name: "Ask for a change" }).click();
  await cleoChanges.getByRole("textbox", { name: "Ask for a change" }).fill("Two more torches, please, and I can wait.");
  await cleoChanges.getByRole("button", { name: "Ask for a change" }).click();
  await vera.reload();
  await veraChanges.getByRole("button", { name: "Write a Change to Agreement" }).click();
  await veraChanges.getByRole("textbox", { name: "What changes" }).fill("Two more torches");
  await veraChanges.getByRole("textbox", { name: "Exactly what's included" }).fill("Torches 7 and 8, same style, 16×32.");
  await veraChanges.getByRole("textbox", { name: /Extra price/ }).fill("abc");
  await expect(veraChanges.getByRole("button", { name: "Send the change" })).toBeDisabled();
  await veraChanges.getByRole("textbox", { name: /Extra price/ }).fill("25");
  await veraChanges.getByRole("button", { name: "Send the change" }).click();
  await expect(veraChanges.getByText("Change 2: Two more torches")).toBeVisible();
  await expect(veraChanges.getByRole("button", { name: "Withdraw this change" })).toBeVisible();

  // Cleo accepts it with the exact-terms box, and pays its price.
  await cleo.reload();
  await expect(cleoChanges).toContainText("Extra price: $25, paid when accepted");
  const accept = cleoChanges.getByRole("button", { name: "Accept this change and pay $25" });
  await expect(accept).toBeDisabled();
  await cleoChanges.getByRole("checkbox", { name: /This change too\./ }).check();
  await accept.click();
  await expect(cleo.getByRole("status")).toContainText("Pay $25 to start the work.");
  await cleo.getByRole("button", { name: "Pay $25" }).click();
  await payOnFakeStripe(cleo, job);
  await expect(cleo.locator(".pay-table tr").filter({ hasText: "Change 2" })).toContainText("Paid");
  await expect(cleo.getByText(/Change to Agreement 2 · accepted/)).toBeVisible();

  // Vera proposes a change herself, then withdraws it.
  await vera.reload();
  await veraChanges.getByRole("button", { name: "Write a Change to Agreement" }).click();
  await veraChanges.getByRole("textbox", { name: "What changes" }).fill("A free smoke effect");
  await veraChanges.getByRole("button", { name: "Send the change" }).click();
  await expect(veraChanges.getByText("Change 3: A free smoke effect")).toBeVisible();
  await expect(veraChanges).toContainText("Extra price: none");
  await veraChanges.getByRole("button", { name: "Withdraw this change" }).click();
  await veraChanges.getByRole("button", { name: "Withdraw this change" }).click();
  await expect(vera.locator(".timeline")).toContainText("withdrew the change");
  await expect(veraChanges.getByText("Change 3: A free smoke effect")).toHaveCount(0);

  // Vera ends the job: as the creative she has to say why.
  await vera.locator(".job-secondary").getByRole("button", { name: "Cancel the job" }).click();
  const cancel = vera.locator(".job-secondary").getByRole("button", { name: "Cancel the job" });
  await expect(cancel).toBeDisabled();
  await vera.getByRole("textbox", { name: "Cancel the job" }).fill("I'm ill and can't finish; refunding in full.");
  await cancel.click();
  await expect(vera.getByRole("status")).toContainText("Cancelled. Any refund comes from Vera Okafor through Stripe.");
});

test("the conversation: asked for, opened, recorded, kept on a failed send (b66), copied, reported, closed", async ({ browser }) => {
  test.slow();
  const job = await hireVera("A shop sign: 'Cleo's Tavern', hand-painted look.");
  const cleo = await as(browser, "client", `/jobs/${job}`);
  const conv = (p: Page) => p.locator("section.conversation");

  // Cleo asks to talk; Vera decides when.
  await expect(conv(cleo).getByRole("note")).toContainText("recorded");
  await conv(cleo).getByRole("textbox", { name: /Ask Vera Okafor for a conversation/ }).fill("Which font styles can you do?");
  await conv(cleo).getByRole("button", { name: "Ask to talk" }).click();
  await expect(conv(cleo).getByText("You asked for a conversation. Vera Okafor decides when to open it.")).toBeVisible();

  const vera = await as(browser, "creative", "/people/jobs");
  await expect(vera.locator(`a[href="/jobs/${job}"]`)).toContainText("WANTS TO TALK");
  await vera.goto(`/jobs/${job}`);
  await expect(conv(vera).getByText("The client asked to talk.")).toBeVisible();
  await expect(conv(vera)).toContainText("Which font styles can you do?");
  await conv(vera).getByRole("button", { name: "Open a conversation" }).click();

  // Sending needs the recording box. A failed send keeps the message.
  const box = conv(vera).getByRole("textbox", { name: "Message" });
  const sendBtn = conv(vera).getByRole("button", { name: "Send" });
  await box.fill("Serif, script or blackletter. Any favourite?");
  await expect(sendBtn).toBeDisabled();
  await conv(vera).getByRole("checkbox", { name: "I understand this conversation is recorded." }).check();
  await vera.route(`**/v1/jobs/${job}/messages`, (route) =>
    route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "Slop is busy; try again" } } }),
  );
  await sendBtn.click();
  await expect(vera.getByRole("alert")).toContainText("Slop is busy; try again");
  await expect(box).toHaveValue("Serif, script or blackletter. Any favourite?");
  await vera.unroute(`**/v1/jobs/${job}/messages`);
  await sendBtn.click();
  await expect(conv(vera).locator("li.mine")).toContainText("Serif, script or blackletter. Any favourite?");
  await expect(box).toHaveValue("");
  await expect(vera.getByRole("alert")).toHaveCount(0);
  // Acknowledged once, it isn't asked again.
  await expect(conv(vera).getByRole("checkbox", { name: /recorded/ })).toHaveCount(0);

  // Cleo answers; she can report Vera's message.
  await cleo.reload();
  const theirs = conv(cleo).locator("ol.messages li").filter({ hasText: "Any favourite?" });
  await expect(theirs).toContainText("Vera Okafor");
  await theirs.getByRole("button", { name: "⚑ Report" }).click();
  await expect(cleo.getByRole("dialog", { name: "Report this message" })).toBeVisible();
  await cleo.keyboard.press("Escape");
  await conv(cleo).getByRole("textbox", { name: "Message" }).fill("Blackletter, please.");
  await conv(cleo).getByRole("checkbox", { name: /recorded/ }).check();
  await conv(cleo).getByRole("button", { name: "Send" }).click();
  await expect(conv(cleo).locator("li.mine")).toContainText("Blackletter, please.");

  // Her copy of the conversation, to keep.
  const download = cleo.waitForEvent("download");
  await conv(cleo).getByRole("button", { name: "Download your copy" }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe(`slop-conversation-${job}.json`);
  const copyPath = await file.path();
  const copy = JSON.parse(readFileSync(copyPath, "utf8"));
  expect(JSON.stringify(copy)).toContain("Blackletter, please.");

  // Vera closes it: the text is kept for a while, then deleted.
  await vera.reload();
  await conv(vera).getByRole("button", { name: "Close the conversation" }).click();
  await expect(conv(vera).getByText(/Closed\. Slop deletes the text after .*: download your copy before then\./)).toBeVisible();
  await expect(conv(vera).getByRole("textbox", { name: "Message" })).toHaveCount(0);

  // The job goes ahead, and Cleo opens a problem: her copy is evidence that matches Slop's record.
  await offerAndAccept(job, offerBody("Tavern shop sign", "on_delivery"));
  await cleo.reload();
  const problem = cleo.locator("section.problem-panel");
  await problem.getByRole("button", { name: "Open a problem" }).click();
  await problem.getByRole("textbox", { name: "Open a problem" }).fill("The agreement says blackletter; the sketch is serif.");
  await problem.getByRole("button", { name: "Open a problem" }).click();
  await expect(problem.getByText("You said:")).toBeVisible();
  const upload = problem.getByLabel(/Submit your copy of the conversation/);
  await upload.setInputFiles({ name: "notes.json", mimeType: "application/json", buffer: Buffer.from("not a copy") });
  await expect(problem.getByText("That isn't a conversation copy downloaded from Slop.")).toBeVisible();
  await upload.setInputFiles(copyPath);
  await expect(problem.getByText("2 messages match Slop's fingerprints, 0 don't.")).toBeVisible();
  await problem.getByRole("button", { name: "Withdraw the problem" }).click();
  await expect(problem.getByText(/Last problem: withdrawn/)).toBeVisible();

  await vera.reload();
  await vera.locator(".job-secondary").getByRole("button", { name: "Cancel the job" }).click();
  await vera.getByRole("textbox", { name: "Cancel the job" }).fill("Ending the test job.");
  await vera.locator(".job-secondary").getByRole("button", { name: "Cancel the job" }).click();
  await expect(vera.getByRole("status")).toContainText("Cancelled.");
  // Ended: nobody can ask to talk or open a conversation again.
  await expect(conv(vera).getByRole("button", { name: "Open a conversation" })).toHaveCount(0);
});

test("a problem: proposals, an answer, a mediator's suggestion, closed unresolved, then settled by cancelling", async ({ browser }) => {
  test.slow();
  const job = await hireVera("Four tavern windows, day and night versions.");
  await offerAndAccept(job, offerBody("Tavern windows", "on_delivery"));
  const cleo = await as(browser, "client", `/jobs/${job}`);
  const problem = (p: Page) => p.locator("section.problem-panel");
  // Its own words, so the mediator's queue row is this job's even when run again.
  const due = `The windows were due Monday (${job.slice(-6)}); nothing yet.`;

  await problem(cleo).getByRole("button", { name: "Open a problem" }).click();
  await expect(problem(cleo).getByRole("button", { name: "Open a problem" })).toBeDisabled();
  await problem(cleo).getByRole("textbox", { name: "Open a problem" }).fill(due);
  await problem(cleo).getByRole("button", { name: "Open a problem" }).click();
  await expect(problem(cleo)).toContainText(`You said: ${due}`);

  // Cleo proposes a refund: nothing was paid, so that's refused, and her words stay (b66).
  await problem(cleo).getByRole("combobox", { name: "Propose" }).selectOption("refund");
  await problem(cleo).getByRole("textbox", { name: "Amount" }).fill("30");
  await problem(cleo).getByRole("textbox", { name: "In words" }).fill("Thirty off for the delay.");
  await problem(cleo).getByRole("button", { name: "Propose" }).click();
  await expect(cleo.getByRole("alert")).toContainText("a refund can't be more than what was paid");
  await expect(problem(cleo).getByRole("textbox", { name: "In words" })).toHaveValue("Thirty off for the delay.");
  // A redo instead; then she thinks better of it.
  await problem(cleo).getByRole("combobox", { name: "Propose" }).selectOption("redo");
  await problem(cleo).getByRole("textbox", { name: "In words" }).fill("Redo the night versions brighter.");
  await problem(cleo).getByRole("button", { name: "Propose" }).click();
  const redo = problem(cleo).locator(".proposals li").filter({ hasText: "Redo the night versions" });
  await expect(redo).toContainText("You proposed: redo");
  await expect(redo).toContainText("(pending)");
  await expect(problem(cleo).getByRole("textbox", { name: "In words" })).toHaveValue("");
  await redo.getByRole("button", { name: "Withdraw" }).click();
  await expect(redo).toContainText("(withdrawn)");

  // Vera answers, and proposes redoing it; Cleo rejects that.
  const vera = await as(browser, "creative", `/jobs/${job}`);
  await expect(problem(vera)).toContainText("They said: The windows were due Monday");
  await problem(vera).getByRole("button", { name: "Answer" }).click();
  await problem(vera).getByRole("textbox", { name: "Answer" }).fill("Sorry: they're coming Friday.");
  await problem(vera).getByRole("button", { name: "Answer" }).click();
  await expect(problem(vera)).toContainText("You answered: Sorry: they're coming Friday.");
  await problem(vera).getByRole("combobox", { name: "Propose" }).selectOption("other");
  await expect(problem(vera).getByRole("textbox", { name: "Amount" })).toHaveCount(0);
  await problem(vera).getByRole("textbox", { name: "In words" }).fill("Friday delivery plus a bonus window.");
  await problem(vera).getByRole("button", { name: "Propose" }).click();

  await cleo.reload();
  await expect(problem(cleo)).toContainText("They answered: Sorry: they're coming Friday.");
  const bonus = problem(cleo).locator(".proposals li").filter({ hasText: "bonus window" });
  await expect(bonus).toContainText("They proposed: other");
  await bonus.getByRole("button", { name: "Reject" }).click();
  await expect(bonus).toContainText("(rejected)");

  // A mediator reads the record and suggests an outcome; it binds no one. Then closes it.
  const admin = await as(browser, "admin", "/admin?tab=queue");
  const row = admin.locator("li.panel").filter({ hasText: due });
  await expect(row).toContainText("Answer: Sorry: they're coming Friday.");
  await expect(row).toContainText("other — Friday delivery plus a bonus window. (rejected)");
  await row.getByRole("button", { name: "Read the job record (logged)" }).click();
  await expect(row.locator("pre.record")).toContainText("Tavern windows");
  await row.getByRole("button", { name: "Suggest an outcome" }).click();
  await row.getByRole("textbox", { name: "Suggest an outcome: reason" }).fill("Deliver Friday; refund $10 for the wait.");
  await row.getByRole("button", { name: "Suggest an outcome" }).click();
  await expect(row).toContainText("Suggested: Deliver Friday; refund $10 for the wait.");
  await cleo.reload();
  await expect(problem(cleo).locator(".mediator-note")).toContainText("Deliver Friday; refund $10 for the wait.");

  await row.getByRole("button", { name: "Close unresolved" }).click();
  await row.getByRole("textbox", { name: "Close unresolved: reason" }).fill("No agreement after the suggestion.");
  await row.getByRole("button", { name: "Close unresolved" }).click();
  await expect(row).toHaveCount(0);
  // The seed's problem is still open.
  await expect(admin.locator("li.panel").filter({ hasText: "Vera stopped answering a week ago" })).toHaveCount(1);
  await cleo.reload();
  await expect(problem(cleo)).toContainText("No agreement after the suggestion.");

  // A new problem, settled: Vera proposes cancelling, Cleo accepts, and the job ends.
  await problem(cleo).getByRole("button", { name: "Open a problem" }).click();
  await problem(cleo).getByRole("textbox", { name: "Open a problem" }).fill("Still nothing on Friday.");
  await problem(cleo).getByRole("button", { name: "Open a problem" }).click();
  await vera.reload();
  await problem(vera).getByRole("combobox", { name: "Propose" }).selectOption("cancel");
  await problem(vera).getByRole("textbox", { name: "In words" }).fill("Let's cancel; nothing was paid.");
  await problem(vera).getByRole("button", { name: "Propose" }).click();
  await cleo.reload();
  await problem(cleo).locator(".proposals li").filter({ hasText: "Let's cancel" }).getByRole("button", { name: "Accept" }).click();
  await expect(cleo.getByRole("status")).toContainText("Cancelled.");
  await expect(problem(cleo)).toContainText("Both agreed: cancel — Let's cancel; nothing was paid.");
});

test("the client declines an offer, and cancels before paying; payments off can't be paid", async ({ browser }) => {
  // An offer turned down, with a note Vera sees.
  const declined = await hireVera("A tavern cat, sleeping.");
  await call("creative", "POST", `/v1/jobs/${declined}/offer`, offerBody("Sleeping cat", "upfront"));
  const cleo = await as(browser, "client", `/jobs/${declined}`);
  await cleo.locator(".job-secondary").getByRole("button", { name: "Decline" }).click();
  await cleo.getByRole("textbox", { name: "Decline" }).fill("Too dear for a cat, sorry.");
  await cleo.locator(".job-secondary").getByRole("button", { name: "Decline" }).click();
  await expect(cleo.getByRole("status")).toContainText("Declined.");
  await expect(cleo.locator(".offer-summary")).toHaveCount(0);
  const vera = await as(browser, "creative", `/jobs/${declined}`);
  await expect(vera.locator(".timeline")).toContainText("Too dear for a cat, sorry.");

  // Accepted, all up front: with payments off the Pay button is off and says why.
  const unpaid = await hireVera("A tavern dog, awake.");
  await offerAndAccept(unpaid, offerBody("Awake dog", "upfront"));
  await cleo.route(`**/v1/jobs/${unpaid}`, async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), payment_mode: "off" } });
  });
  await cleo.goto(`/jobs/${unpaid}`);
  await expect(cleo.getByRole("button", { name: "Pay $100" })).toBeDisabled();
  await expect(cleo.getByText(/Payments aren't switched on yet on this server\./)).toBeVisible();
  await cleo.unroute(`**/v1/jobs/${unpaid}`);
  await cleo.reload();
  await expect(cleo.getByRole("button", { name: "Pay $100" })).toBeEnabled();
  await expect(cleo.getByText(/Paid directly to Vera Okafor through Stripe\./)).toBeVisible();

  // The client may cancel before paying, without a reason.
  await cleo.locator(".job-secondary").getByRole("button", { name: "Cancel the job" }).click();
  await cleo.locator(".job-secondary").getByRole("button", { name: "Cancel the job" }).click();
  await expect(cleo.getByRole("status")).toContainText("Cancelled.");
});

test("withdrawing from a contract asks first (consumer cancellation right)", async ({ page }) => {
  // No seeded client has a withdrawal right (Cleo is in the US), so the page is shown one.
  await signIn(page, "client");
  let withdrew = false;
  await page.route(`**/v1/jobs/${ids.workingJob}`, async (route) => {
    const res = await route.fetch();
    const j = await res.json();
    await route.fulfill({
      response: res,
      json: { ...j, actions: [...j.actions, "withdraw"], withdrawal: { until: "2099-01-01T00:00:00Z", open: true, started_early: true } },
    });
  });
  await page.route(`**/v1/jobs/${ids.workingJob}/withdraw`, async (route) => {
    withdrew = true;
    const j = await call("client", "GET", `/v1/jobs/${ids.workingJob}`);
    await route.fulfill({ json: { ...j, status: "withdrawn", status_text: "Withdrawn by the client.", actions: [] } });
  });
  await page.goto(`/jobs/${ids.workingJob}`);
  await expect(page.getByText(/You can withdraw from this contract until .*\(your legal cancellation right\)/)).toBeVisible();
  await expect(page.getByText(/You asked for work to start straight away/)).toBeVisible();
  const withdraw = page.getByRole("button", { name: "Withdraw from this contract" });

  // Saying no to the question does nothing.
  page.once("dialog", (d) => void d.dismiss());
  await withdraw.click();
  await expect(page.getByRole("status")).not.toContainText("Withdrawn");
  expect(withdrew).toBe(false);

  page.once("dialog", (d) => {
    expect(d.message()).toContain("The creative refunds what's owed within 14 days.");
    void d.accept();
  });
  await withdraw.click();
  await expect(page.getByRole("status")).toContainText("Withdrawn by the client.");
  expect(withdrew).toBe(true);
});
