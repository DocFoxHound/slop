// SPDX-License-Identifier: AGPL-3.0-or-later
// People in the review queue: a problem between a client and a creative (the record read, an
// outcome suggested, closed unresolved) and a disputed review (removed with a statement of
// reasons). Cleo hires Vera for each, as the seed does; the seeded problem and dispute stay
// open for the other tests (what staff would send about them is checked without sending it).
import { acceptedJob, call, closePages, completedJob, newPage, ok, openItem, seeDecisions, tag } from "./admin-helpers";
import { expect, signIn, test } from "./fixtures";

test.afterEach(closePages);

test("a People problem: the record read, an outcome suggested, then closed unresolved", async ({ browser }) => {
  const t = tag();
  const title = `Barrel sprites ${t}`;
  const job = await acceptedJob(`Six barrels for the cellar, 32×32 (${t}).`, title);
  await ok("client", "POST", `/v1/jobs/${job}/problem`, { statement: `Nothing has arrived and the due date passed (${t}).` });

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await openItem(admin, "problems", title);
  const problem = admin.getByRole("listitem").filter({ has: admin.getByText(title, { exact: true }) });
  await expect(problem.getByText(/Vera Okafor and Cleo Client · opened by the client ·/)).toBeVisible();
  await expect(problem.getByText(`Nothing has arrived and the due date passed (${t}).`)).toBeVisible();

  // Reading the record is logged.
  await problem.getByRole("button", { name: "Read the job record (logged)" }).click();
  await expect(problem.getByText("The job record")).toBeVisible();
  await expect(problem.locator("pre.record")).toContainText(`Six barrels for the cellar, 32×32 (${t}).`);
  const audit = await ok("admin", "GET", "/v1/admin/audit?q=job.record_read");
  expect(audit.some((e: { subject: string }) => e.subject === job)).toBe(true);

  // A suggestion both sides see; it binds no one.
  await problem.getByRole("button", { name: "Suggest an outcome" }).click();
  await problem.getByRole("textbox", { name: "Suggest an outcome: reason" }).fill("Deliver by Friday, or refund half.");
  await problem.getByRole("button", { name: "Suggest an outcome" }).click();
  await expect(problem.getByText("Suggested: Deliver by Friday, or refund half.")).toBeVisible();
  const cleo = await newPage(browser);
  await signIn(cleo, "client");
  await cleo.goto(`/jobs/${job}`);
  await expect(cleo.getByText(/A Slop mediator suggests.*Deliver by Friday, or refund half\./)).toBeVisible();

  // They didn't agree: closed, with a note both read.
  await problem.getByRole("button", { name: "Close unresolved" }).click();
  await problem.getByRole("textbox", { name: "Close unresolved: reason" }).fill("Neither side accepted; both keep the record.");
  await problem.getByRole("button", { name: "Close unresolved" }).click();
  await expect(problem).toHaveCount(0);
  await cleo.reload();
  await expect(cleo.getByText(/Neither side accepted; both keep the record\./).first()).toBeVisible();
  // The record is for open problems only.
  expect((await call("admin", "GET", `/v1/admin/jobs/${job}/record`)).status).toBe(404);
});

test("a review dispute: the review removed, with a statement of reasons for its writer", async ({ browser }) => {
  const t = tag();
  const job = await completedJob(`A cellar door, 64×64 (${t}).`, `Cellar door ${t}`);
  const review = `Make me a free cellar too or this stays (${t}).`;
  await ok("client", "PUT", `/v1/jobs/${job}/review`, { vote: "slop", body: review });
  await ok("creative", "POST", `/v1/people/reviews/${job}/dispute`, {
    reason: "extortion",
    statement: `The client wants free work for removing it (${t}).`,
  });

  const admin = await newPage(browser);
  await signIn(admin, "admin");
  await openItem(admin, "disputes", review);
  const dispute = admin.getByRole("listitem").filter({ hasText: review });
  await expect(dispute.getByText(/Cleo Client on Vera Okafor: 👎 Slop/)).toBeVisible();
  await expect(dispute.getByText(`The client wants free work for removing it (${t}).`)).toBeVisible();
  await dispute.getByRole("button", { name: "Read the job record (logged)" }).click();
  await expect(dispute.locator("pre.record")).toContainText(`A cellar door, 64×64 (${t}).`);
  await dispute.getByRole("button", { name: "Remove it" }).click();
  await dispute.getByRole("textbox", { name: "Remove it: reason" }).fill("Reviews can't be used to ask for free work.");
  await dispute.getByRole("button", { name: "Remove it" }).click();
  await expect(dispute).toHaveCount(0);

  // Gone from Vera's page; Cleo has the statement of reasons and can appeal.
  const visitor = await newPage(browser);
  await visitor.goto("/people/vera-pixels");
  await expect(visitor.getByRole("heading", { name: "Vera Okafor", level: 1 })).toBeVisible();
  await expect(visitor.getByText(review)).toHaveCount(0);
  const cleo = await newPage(browser);
  await signIn(cleo, "client");
  await cleo.goto("/account#decisions");
  const decision = cleo.getByRole("article", { name: "Review hidden: Review of Vera Okafor by Cleo Client" });
  await expect(decision.getByRole("link", { name: "Content Policy: Reviews used as leverage" })).toBeVisible();
  await expect(decision.getByRole("button", { name: "Appeal this decision" })).toBeVisible();
  await seeDecisions("client");
});

test("a review dispute kept, or kept with a note: what staff send", async ({ page }) => {
  // The seeded dispute stays pending for the other tests: the answers are caught on the way.
  const sent: unknown[] = [];
  await page.route("**/v1/admin/review-disputes/*/decide", async (route) => {
    sent.push(JSON.parse(route.request().postData() ?? "null"));
    await route.fulfill({ status: 204 });
  });
  await signIn(page, "admin");
  await openItem(page, "disputes", "Make me a second sign for free or this review stays.");
  const dispute = page.getByRole("listitem").filter({ hasText: "Make me a second sign for free or this review stays." });
  await expect(dispute.getByText(/Creative's dispute/)).toBeVisible();
  await dispute.getByRole("button", { name: "Keep it" }).click();
  await dispute.getByRole("textbox", { name: "Keep it: reason" }).fill("It's an opinion about the work.");
  await dispute.getByRole("button", { name: "Keep it" }).click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0]).toEqual({ outcome: "keep", reason: "It's an opinion about the work." });
  await page.reload();
  await dispute.getByRole("button", { name: "Keep with a note" }).click();
  await dispute.getByRole("textbox", { name: "Keep with a note: reason" }).fill("The record shows the sign was delivered on time.");
  await dispute.getByRole("button", { name: "Keep with a note" }).click();
  await expect.poll(() => sent.length).toBe(2);
  expect(sent[1]).toEqual({
    outcome: "note",
    reason: "A factual claim differs from the job record.",
    note: "The record shows the sign was delivered on time.",
  });
});
