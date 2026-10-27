// SPDX-License-Identifier: AGPL-3.0-or-later
// A new app's store page from an AI assistant's handoff link (/publish/new#fill=…, handoff.ts):
// filled in, checked by the person, created by them; kept through signing in; a broken link says
// so. Mia creates one draft of her own here; Bo and Fin only look.
import { expect, server, signIn, test } from "./fixtures";
import { call, uniq } from "./publishing-helpers";

/** The link an assistant builds: the fields as URL-encoded JSON in the fragment. */
const fillLink = (fields: unknown) => `/publish/new#fill=${encodeURIComponent(JSON.stringify(fields))}`;

const notice = (page: import("@playwright/test").Page) => page.getByRole("status", { name: "Filled in by your AI assistant" });

test("an assistant's link fills in a new app; the maker checks it and creates the draft", async ({ page }) => {
  const slug = uniq("handoff");
  const fields = {
    title: `Harbour Lights ${slug.slice(-4)}`,
    slug,
    summary: "Guide the boats home.",
    description_md: "# Harbour Lights\n\nKeep the lamps lit.",
    kind: "game",
    stage: "beta",
    repo_url: `https://github.com/e2e/${slug}`,
    license_spdx: "MIT",
    min_age: 12,
    content_labels: ["violence"],
    online: ["own_servers"],
    online_hosts: ["play.example.com"],
    sandbox_ready: true,
    bug_reports: false,
    accessibility: ["subtitles"],
    platforms: ["windows"],
    tags: ["puzzle", "not-a-real-tag"],
    // Never taken from a link, and not part of the form.
    repo_token: { kind: "gitea", token: "never-in-a-link" },
    price: 5,
  };
  await signIn(page, "maker");
  await page.goto(fillLink(fields));
  await expect(page.getByRole("heading", { name: "New app", level: 1 })).toBeVisible();
  // Out of the address bar, so a reload doesn't apply it over the maker's changes.
  await expect(page).toHaveURL(/\/publish\/new$/);
  expect(await page.evaluate(() => location.hash)).toBe("");

  const told = notice(page);
  await expect(told).toContainText("Check everything, especially the age rating, content labels, licence and source repository, then press Create draft.");
  await expect(told.locator(".handoff-filled")).toHaveText(
    "Filled in: Name, Store address, One-line summary, Description, Type, Stage, Source repository, Licence, Age rating, Content labels, Online, Your servers, Works in sandbox mode, Take bug reports, Accessibility, Runs on and Tags.",
  );
  await expect(told.getByRole("listitem")).toHaveText([
    "Tags: not-a-real-tag isn't one of the choices",
    "Repository token: a token never goes in a link: if the repository is private, paste a read-only token yourself",
  ]);
  await expect(told).toContainText("One other thing in the link isn't part of this form, so it was left out.");

  await expect(page.getByLabel("Name", { exact: true })).toHaveValue(fields.title);
  await expect(page.getByRole("textbox", { name: /^Store address/ })).toHaveValue(slug);
  await expect(page.getByLabel("One-line summary")).toHaveValue(fields.summary);
  await expect(page.getByLabel("Description")).toHaveValue(fields.description_md);
  await expect(page.getByLabel("Type")).toHaveValue("game");
  await expect(page.getByLabel("Stage")).toHaveValue("beta");
  await expect(page.getByRole("textbox", { name: /Source repository/ })).toHaveValue(fields.repo_url);
  await expect(page.getByRole("combobox", { name: "Licence" })).toHaveValue("MIT");
  await expect(page.getByLabel("Age rating")).toHaveValue("12");
  await expect(page.getByRole("group", { name: "Content labels" }).getByRole("button", { name: "Violence" })).toHaveAttribute("aria-pressed", "true");
  const online = page.getByRole("group", { name: "Online" });
  await expect(online.getByRole("button", { name: "Connects to the maker's servers or database" })).toHaveAttribute("aria-pressed", "true");
  await expect(online.getByRole("textbox", { name: /Your servers/ })).toHaveValue("play.example.com");
  await expect(page.getByRole("checkbox", { name: /Works in sandbox mode/ })).toBeChecked();
  await expect(page.getByRole("checkbox", { name: /Take bug reports/ })).not.toBeChecked();
  await expect(page.getByRole("checkbox", { name: "Subtitles or captions" })).toBeChecked();
  await expect(page.getByRole("group", { name: "Runs on" }).getByRole("checkbox", { name: /Windows/ })).toBeChecked();
  await expect(page.getByRole("button", { name: "Puzzle", exact: true })).toHaveAttribute("aria-pressed", "true");
  // The token isn't anywhere on the page.
  await expect(page.getByRole("group", { name: "Private repository on another host" })).toHaveCount(0);

  // The maker changes what they like, then presses Create themselves.
  await page.getByLabel("One-line summary").fill("Guide the fishing boats home.");
  await page.getByRole("button", { name: "Create draft" }).click();
  await expect(page).toHaveURL(new RegExp(`/app/${slug}$`));
  const app = await call<Record<string, unknown>>("maker", "GET", `/v1/apps/${slug}`);
  expect(app).toMatchObject({
    title: fields.title,
    summary: "Guide the fishing boats home.",
    description_md: fields.description_md,
    kind: "game",
    stage: "beta",
    repo_url: fields.repo_url,
    license_spdx: "MIT",
    min_age: 12,
    content_labels: ["violence"],
    online: ["own_servers"],
    online_hosts: ["play.example.com"],
    sandbox_ready: true,
    bug_reports: false,
    accessibility: ["subtitles"],
    platforms: ["windows"],
    tags: ["puzzle"],
  });

  // Used once: the form is empty the next time.
  await page.goto("/publish/new");
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
  await expect(notice(page)).toHaveCount(0);
});

test("a reload doesn't fill it in again, and Clear the form empties it", async ({ page }) => {
  await signIn(page, "finance");
  await page.goto(fillLink({ title: "Tide Clock", summary: "Tells the tides." }));
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Tide Clock");
  // Typed by the assistant without an address: it follows the name, as when typed by hand.
  await expect(page.getByRole("textbox", { name: /^Store address/ })).toHaveValue("tide-clock");
  await page.getByRole("button", { name: "Clear the form" }).click();
  await expect(notice(page)).toHaveCount(0);
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("One-line summary")).toHaveValue("");

  // A new link opened in the same tab only changes the fragment: it's filled in all the same.
  await page.goto(fillLink({ title: "Tide Clock" }));
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Tide Clock");
  expect(await page.evaluate(() => location.hash)).toBe("");
  await page.reload();
  await expect(page.getByRole("heading", { name: "New app", level: 1 })).toBeVisible();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
  await expect(notice(page)).toHaveCount(0);
});

test("signed out, the link waits while the person signs in, then fills in the form", async ({ page }) => {
  await page.goto(fillLink({ title: "Lighthouse Keeper", summary: "Keep the light turning.", stage: "coming_soon", kind: "app" }));
  await expect(page.getByRole("heading", { name: "Publish on SLAPorSLOP", level: 1 })).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Your AI assistant's draft is ready." })).toContainText(
    "Sign in, or make an account, and the form opens with it filled in for you to check.",
  );
  expect(await page.evaluate(() => location.hash)).toBe("");
  await page.getByRole("link", { name: "Sign in to start" }).click();
  await expect(page).toHaveURL(/\/signin\?return_to=%2Fpublish%2Fnew$/);
  // Off to the sign-in provider (the fake GitHub) and back, as signing in goes: the fragment
  // doesn't make that trip, the tab's storage does.
  await page.getByRole("link", { name: "Continue with GitHub" }).click();
  await page.waitForURL(`${server.fake}login/oauth/authorize?**`);
  await signIn(page, "maker2");
  await page.goto("/publish/new");
  await expect(notice(page)).toBeVisible();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Lighthouse Keeper");
  await expect(page.getByLabel("One-line summary")).toHaveValue("Keep the light turning.");
  await expect(page.getByLabel("Type")).toHaveValue("app");
  await expect(page.getByLabel("Stage")).toHaveValue("coming_soon");
  // Bo checks it later: it's applied once.
  await page.reload();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
});

test("a link that can't be read says so, and the form is empty", async ({ page }) => {
  // Signed out: said before signing in, and nothing waits.
  await page.goto("/publish/new#fill=%7Bnot-json");
  await expect(page.getByRole("alert")).toHaveText(
    "The link from your AI assistant couldn't be read: what follows #fill= isn't JSON. Ask your assistant for a new link, or fill in the form yourself after signing in.",
  );
  expect(await page.evaluate(() => location.hash)).toBe("");

  await signIn(page, "finance");
  await page.goto("/publish");
  await page.goto("/publish/new#fill=%E0%A4%A");
  await expect(page.getByRole("alert")).toHaveText(
    "The link from your AI assistant couldn't be read: it isn't URL-encoded properly. The form is empty: fill it in yourself, or ask your assistant for a new link.",
  );
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
  await expect(notice(page)).toHaveCount(0);
  expect(await page.evaluate(() => location.hash)).toBe("");

  // Not an object of fields.
  await page.goto(`/publish/new#fill=${encodeURIComponent('["Harbour"]')}`);
  await expect(page.getByRole("alert")).toContainText('it should hold one JSON object of fields, like {"title": "…"}');
  // The assistant's next link works.
  await page.goto(fillLink({ title: "Second Try" }));
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Second Try");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("what a link holds goes only into the inputs", async ({ page }) => {
  await signIn(page, "finance");
  const html = '<img src=x onerror="window.__pwned=1"><b>bold</b>';
  await page.goto(fillLink({ title: "Safe Harbour", description_md: html, summary: "<script>window.__pwned=1</script>" }));
  await expect(page.getByLabel("Description")).toHaveValue(html);
  await expect(page.getByLabel("One-line summary")).toHaveValue("<script>window.__pwned=1</script>");
  await expect(page.locator("form b", { hasText: "bold" })).toHaveCount(0);
  expect(await page.evaluate(() => (window as { __pwned?: number }).__pwned)).toBeUndefined();
});
