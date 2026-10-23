// SPDX-License-Identifier: AGPL-3.0-or-later
// A People job from start to finish, in two browsers: Cleo (client) hires Vera (creative) for
// Cleo's Tavern, through offer, payment on the fake Stripe, sign-off, delivery, final files,
// review, reply, review dispute and the mediator's decision. Its own job: the seeded ones stay.
import { readFileSync } from "node:fs";
import type { Browser, Page } from "@playwright/test";
import { expect, ids, signIn, test, type Who } from "./fixtures";
import { call, hireVera, inDays, payOnFakeStripe } from "./people-helpers";

async function as(browser: Browser, who: Who, path: string): Promise<Page> {
  const page = await browser.newPage();
  await signIn(page, who);
  await page.goto(path);
  return page;
}

test.afterAll(async () => {
  // Whatever happens above, don't leave Cleo with open requests to Vera (she may have three).
  const jobs = await call<{ as_client: Array<{ id: string; status: string }> }>("client", "GET", "/v1/me/jobs");
  for (const j of jobs.as_client)
    if (["requested", "offered"].includes(j.status) && j.id !== ids.requestedJob)
      await call("client", "POST", `/v1/jobs/${j.id}/decline`, { note: "" });
});

test("a job from request to review: offer, pay half, sign-off, preview, pay the rest, final files", async ({ browser }) => {
  test.slow();
  const job = await hireVera("A tavern map, 20×15 tiles, top-down, with a bar and four tables.");
  const vera = await as(browser, "creative", `/jobs/${job}`);
  await expect(vera.getByRole("status")).toContainText("They're waiting for your offer.");
  await expect(vera.getByText(/This client: paid in full on \d+ of \d+ finished job/)).toBeVisible();

  // Vera's offer: the price must be a number.
  const offer = vera.locator("form.offer-form");
  await expect(offer.getByRole("heading", { name: "Send an offer" })).toBeVisible();
  await offer.getByRole("textbox", { name: "What they'll get" }).fill("Tavern map, 20×15 tiles");
  await offer.getByRole("textbox", { name: /Exactly what's included/ }).fill("One 20×15 tile map: floor, bar, four tables, PNG and Tiled files.");
  await offer.getByRole("textbox", { name: "Price" }).fill("two hundred");
  await offer.getByRole("button", { name: "Send offer" }).click();
  await expect(offer.getByText("Write the price as a number, like 120 or 120.50.")).toBeVisible();
  await offer.getByRole("textbox", { name: "Price" }).fill("200");
  await expect(offer.getByText(/They pay \$200\. You receive about \$190/)).toBeVisible();
  await expect(offer.getByRole("radio", { name: /Half now, half when they accept the delivery/ })).toBeChecked();
  await offer.getByRole("checkbox", { name: /Sign-off on the direction first/ }).check();
  await offer.getByRole("radio", { name: /Licence for this project/ }).check();
  await offer.getByLabel("Due").fill(inDays(21));
  await offer.getByRole("button", { name: "Send offer" }).click();
  await expect(vera.getByRole("status")).toContainText("Offer sent. Waiting for the client.");

  // A new offer replaces the first: the form keeps what was sent.
  await expect(offer.getByRole("heading", { name: "Send a new offer" })).toBeVisible();
  await expect(offer.getByRole("textbox", { name: "Price" })).toHaveValue("200");
  await offer.getByRole("textbox", { name: "Price" }).fill("180");
  await offer.getByRole("button", { name: "Send offer" }).click();
  await expect(vera.locator(".timeline li").filter({ hasText: "sent an offer" })).toHaveCount(2);

  // Cleo reads the offer in plain words and accepts it: Accept waits for the exact-terms box.
  const cleo = await as(browser, "client", `/jobs/${job}`);
  await expect(cleo.getByRole("status")).toContainText("Vera Okafor sent an offer. Read it carefully");
  const summary = cleo.locator(".offer-summary");
  await expect(summary).toContainText("$180");
  await expect(summary.getByText(/Read every word/)).toBeVisible();
  await summary.getByText("Read the full Service Agreement").click();
  await expect(summary.locator(".agreement .prose")).toContainText("Vera Okafor");
  const accept = summary.getByRole("button", { name: "Accept & pay $90" });
  await expect(accept).toBeDisabled();
  await summary.getByRole("checkbox", { name: /exactly/ }).check();
  await accept.click();
  await expect(cleo.getByRole("status")).toContainText("Pay $90 to start the work.");

  // Pay the first half on (fake) Stripe; back on the job, the page asks Stripe how it went.
  const checked = cleo.waitForRequest((r) => r.url().endsWith(`/v1/jobs/${job}/payment/check`) && r.method() === "POST");
  await cleo.getByRole("button", { name: "Pay $90" }).click();
  await payOnFakeStripe(cleo, job);
  await checked;
  await expect(cleo.getByRole("status")).toContainText("Vera Okafor is working on it.");
  await expect(cleo.locator(".pay-table tr").filter({ hasText: "First half" })).toContainText("Paid");
  await expect(cleo.locator(".pay-table tr").filter({ hasText: "Second half" })).toContainText("On delivery");

  // Vera sends the direction for sign-off; Cleo signs off.
  await vera.reload();
  await expect(vera.getByRole("status")).toContainText("Send the direction for sign-off first.");
  const signOff = vera.locator("form").filter({ has: vera.getByRole("heading", { name: "Send the direction for sign-off" }) });
  await signOff.getByRole("textbox", { name: /Links to the files/ }).fill("https://drive.example/tavern-sketch.png");
  await signOff.getByRole("textbox", { name: /Note/ }).fill("Rough layout: bar on the left.");
  await signOff.getByRole("button", { name: "Send for sign-off" }).click();
  await expect(vera.getByRole("status")).toContainText("Waiting for the client to sign off on the direction.");

  await cleo.reload();
  await expect(cleo.locator(".timeline")).toContainText("https://drive.example/tavern-sketch.png");
  await cleo.getByRole("button", { name: "Sign off on this direction" }).click();
  await expect(cleo.getByRole("status")).toContainText("Vera Okafor is working on it.");

  // Vera delivers: half is still owed on delivery, so she sends a preview.
  await vera.reload();
  const deliver = vera.locator("form").filter({ has: vera.getByRole("heading", { name: "Deliver the work" }) });
  await expect(deliver.getByText(/Money is still owed on delivery, so send a preview/)).toBeVisible();
  await deliver.getByRole("textbox", { name: /Links to the files/ }).fill("https://drive.example/tavern-preview.png");
  await deliver.getByRole("button", { name: "Mark delivered" }).click();
  await expect(vera.getByRole("status")).toContainText("Delivered. Waiting for the client to accept.");

  // Cleo pays the rest, which accepts the delivery.
  await cleo.reload();
  await expect(cleo.getByRole("status")).toContainText("pay the remaining $90");
  await expect(cleo.getByText(/If you don't answer by .*, the delivery counts as accepted\./)).toBeVisible();
  await cleo.getByRole("button", { name: "Accept the delivery — pay $90" }).click();
  await payOnFakeStripe(cleo, job);
  await expect(cleo.getByRole("status")).toContainText("Paid in full. Waiting for Vera Okafor to send the final files.");

  // Vera sends the final files: done.
  await vera.reload();
  const finals = vera.locator("form").filter({ has: vera.getByRole("heading", { name: "Send the final files" }) });
  await finals.getByRole("textbox", { name: /Links to the files/ }).fill("https://drive.example/tavern-final.zip");
  await finals.getByRole("button", { name: "Send final files" }).click();
  await expect(vera.getByRole("status")).toContainText("Completed.");
  await expect(vera.getByText(/You receive about \$171 of the original price/)).toBeVisible();

  // Cleo's review: a vote is needed; it can be changed later.
  await cleo.reload();
  await expect(cleo.getByRole("status")).toContainText("Completed.");
  await expect(cleo.getByText("Credited on Cleo's Tavern's page.")).toBeVisible();
  const review = cleo.locator("form").filter({ has: cleo.getByRole("heading", { name: "Was Vera Okafor's work slop?" }) });
  await expect(review.getByRole("button", { name: "Post review" })).toBeDisabled();
  await review.getByRole("button", { name: "👍 Not Slop" }).click();
  await review.getByRole("textbox", { name: /A few words/ }).fill("Exactly the map we asked for, on time.");
  await review.getByRole("button", { name: "Post review" }).click();
  const mine = cleo.locator("form").filter({ has: cleo.getByRole("heading", { name: "Your review" }) });
  await expect(mine.getByRole("button", { name: "👍 Not Slop" })).toHaveAttribute("aria-pressed", "true");
  await mine.getByRole("textbox", { name: /A few words/ }).fill("Exactly the map we asked for, two days early.");
  await mine.getByRole("button", { name: "Update review" }).click();
  await expect(mine.getByRole("button", { name: "Update review" })).toBeEnabled();
  expect((await call("client", "GET", `/v1/jobs/${job}`)).review.body).toBe("Exactly the map we asked for, two days early.");

  // Either side downloads the full record, as Markdown.
  const download = cleo.waitForEvent("download");
  await cleo.getByRole("button", { name: "Download the full record" }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe(`slop-job-${job}.md`);
  const record = readFileSync(await file.path(), "utf8");
  expect(record).toContain("Tavern map, 20×15 tiles");
  expect(record).toContain("https://drive.example/tavern-final.zip");

  // Both see the job in their lists.
  await cleo.goto("/people/jobs");
  await expect(cleo.getByRole("heading", { name: "Hiring" })).toBeVisible();
  await expect(cleo.getByRole("link", { name: /Tavern map, 20×15 tiles/ })).toContainText("COMPLETED");
  await vera.goto("/people/jobs");
  await expect(vera.getByRole("heading", { name: "Working on" })).toBeVisible();
  await expect(vera.getByRole("link", { name: /Tavern map, 20×15 tiles/ })).toContainText("Cleo Client");

  // On her page, Vera replies to the review, edits the reply, and disputes it.
  await vera.goto(`/people/${ids.creativeHandle}`);
  const card = vera.locator(`#review-${job}`);
  await expect(card).toContainText("Exactly the map we asked for, two days early.");
  await expect(vera.locator(`#review-${ids.completedJob}`).getByText("Dispute: waiting for a mediator")).toBeVisible();
  await card.getByRole("button", { name: "Reply" }).click();
  await card.getByRole("textbox", { name: "Your public reply" }).fill("Thank you, Cleo!");
  await card.getByRole("button", { name: "Save reply" }).click();
  await expect(card.locator(".review-reply")).toContainText("Thank you, Cleo!");
  await card.getByRole("button", { name: "Edit your reply" }).click();
  await card.getByRole("textbox", { name: "Your public reply" }).fill("Thank you, Cleo! Enjoy the tavern.");
  await card.getByRole("button", { name: "Save reply" }).click();
  await expect(card.locator(".review-reply")).toContainText("Enjoy the tavern.");

  // Its own words, so the mediator's queue row is this job's even when run again.
  const onTime = `It was on time (${job.slice(-6)}), not two days early.`;
  await card.getByRole("button", { name: "Dispute this review" }).click();
  await card.getByRole("radio", { name: /It states something the job record shows is false/ }).check();
  await card.getByRole("textbox", { name: /Explain, pointing to the job record/ }).fill(onTime);
  await card.getByRole("button", { name: "Send to a mediator" }).click();
  await expect(card.getByText("Dispute: waiting for a mediator")).toBeVisible();
  await expect(card.getByRole("button", { name: "Dispute this review" })).toHaveCount(0);

  // Cleo answers the dispute from the job page.
  await cleo.goto(`/jobs/${job}`);
  await expect(cleo.locator(".review-reply")).toContainText("Enjoy the tavern.");
  const dispute = cleo.locator(".notice").filter({ hasText: "Vera Okafor disputes this review" });
  await expect(dispute).toContainText(onTime);
  await expect(dispute.getByRole("button", { name: "Send your answer" })).toBeDisabled();
  await dispute.getByRole("textbox", { name: "Your side, for the mediator" }).fill("It arrived two days before the date.");
  await dispute.getByRole("button", { name: "Send your answer" }).click();
  await expect(dispute).toContainText("Your answer: It arrived two days before the date.");

  // A mediator reads the record and keeps the review, with a note.
  const admin = await as(browser, "admin", "/admin?tab=queue");
  const row = admin.locator("li.panel").filter({ hasText: onTime });
  await expect(row).toContainText("It arrived two days before the date.");
  await row.getByRole("button", { name: "Read the job record (logged)" }).click();
  await expect(row.locator("pre.record")).toContainText("Tavern map, 20×15 tiles");
  await row.getByRole("button", { name: "Keep with a note" }).click();
  await row.getByRole("textbox", { name: "Keep with a note: reason" }).fill("The record shows delivery on the due date.");
  await row.getByRole("button", { name: "Keep with a note" }).click();
  await expect(row).toHaveCount(0);
  // The seed's dispute is still waiting.
  await expect(admin.locator("li.panel").filter({ hasText: "The client wants free work in exchange" })).toHaveCount(1);

  await vera.reload();
  await expect(card.locator(".mediator-note")).toContainText("The record shows delivery on the due date.");
  await expect(card.getByText("Dispute: kept, with a mediator's note")).toBeVisible();
});

test("Your jobs: signed out, and with no jobs yet", async ({ browser }) => {
  const visitor = await browser.newPage();
  await visitor.goto("/people/jobs");
  await expect(visitor.getByText("Sign in to see your jobs.")).toBeVisible();

  const mo = await as(browser, "moderator", "/people/jobs");
  await expect(mo.getByRole("heading", { name: "Your jobs", level: 1 })).toBeVisible();
  await expect(mo.getByText("No jobs yet.")).toBeVisible();
  await mo.getByRole("link", { name: "Find someone to hire" }).click();
  await expect(mo).toHaveURL("/people");

  // Cleo's list marks what waits on her, and inquiries.
  const cleo = await as(browser, "client", "/people/jobs");
  await expect(cleo.getByRole("link", { name: /Knight sprite sheet/ })).toContainText("YOUR TURN");
  await expect(cleo.getByRole("link", { name: /Tavern floor tiles/ })).toContainText("IN PROGRESS");
});
