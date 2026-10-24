// SPDX-License-Identifier: AGPL-3.0-or-later
// What makers manage on their account: AI agent tokens, signing keys, a private repository's
// token, and the passkey and agreement asked for before publishing (components/AiAgents.tsx,
// SigningKeys.tsx, RepoToken.tsx, PasskeyNotice.tsx, PasskeyRequired.tsx, Terms.tsx).
import AxeBuilder from "@axe-core/playwright";
import { generateKeyPairSync } from "node:crypto";

import { apiError, call, fake, noPasskeys, openAccount, passkeyOptions, stubPasskeys, tweak } from "./accounts-helpers";
import { expect, server, signIn, test } from "./fixtures";

/** A new Ed25519 public key, as the line in id_ed25519.pub. */
function sshPublicKey(): string {
  const raw = generateKeyPairSync("ed25519").publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const field = (b: Buffer) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(b.length);
    return Buffer.concat([len, b]);
  };
  return `ssh-ed25519 ${Buffer.concat([field(Buffer.from("ssh-ed25519")), field(raw)]).toString("base64")} test@e2e`;
}

const inDays = (days: number) =>
  new Date(Date.now() + days * 86_400_000).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });

test.describe("AI agent tokens", () => {
  test("a look-only token for one app, for a week, with its command to copy", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: server.url });
    await signIn(page, "maker");
    await page.goto("/account#ai-agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("Read-only helper");
    await section.getByRole("combobox", { name: "Can" }).selectOption("read");
    await section.getByRole("checkbox", { name: "Only Lantern Keep" }).check();
    await expect(section.getByRole("radio", { name: /All my apps/ })).not.toBeChecked();
    await section.getByRole("combobox", { name: "Expires after" }).selectOption("7");
    const made = page.waitForRequest((r) => r.url().endsWith("/v1/me/maker-tokens") && r.method() === "POST");
    await section.getByRole("button", { name: "Make token" }).click();
    expect((await made).postDataJSON()).toEqual({ name: "Read-only helper", scope: "read", apps: ["lantern-keep"], people: false, days: 7 });
    await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
    // Only a publishing token is told about passkeys.
    await expect(section.getByText(/Confirm with a passkey before making a token/)).toHaveCount(0);
    await section.getByRole("button", { name: "Copy command" }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toMatch(/^claude mcp add --transport http slop .*slopmk_/);
    await section.getByRole("button", { name: "Done" }).click();
    const row = section.getByRole("listitem").filter({ hasText: "Read-only helper" });
    await expect(row).toContainText("looks · lantern-keep");
    await expect(row).toContainText(`expires ${inDays(7)}`);
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
  });

  test("a publishing token made without a passkey says what that means", async ({ page }) => {
    await signIn(page, "maker");
    await page.goto("/account#ai-agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("Publisher bot");
    await section.getByRole("combobox", { name: "Expires after" }).selectOption("1");
    await section.getByRole("button", { name: "Make token" }).click();
    await expect(section.getByText(/Confirm with a passkey before making a token/)).toBeVisible();
    await section.getByRole("button", { name: "Done" }).click();
    const row = section.getByRole("listitem").filter({ hasText: "Publisher bot" });
    await expect(row).toContainText("publishes · all apps");
    await expect(row).toContainText(`expires ${inDays(1)}`);
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
  });

  test("a refused token says why", async ({ page }) => {
    await fake(page, "POST", "/v1/me/maker-tokens", { status: 409, body: apiError("conflict", "you have 10 tokens: remove one first") });
    await signIn(page, "maker");
    await page.goto("/account#ai-agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("One too many");
    await section.getByRole("button", { name: "Make token" }).click();
    await expect(section.getByRole("alert")).toHaveText("you have 10 tokens: remove one first");
    await expect(section.getByText("Copy it now")).toHaveCount(0);
  });

  test("making a token never asks to confirm it's you", async ({ page }) => {
    // The server no longer asks for a recent sign-in (maker_agents.rs; the API tests sign in days
    // before). Here: the page goes straight from Make token to the token, with no passkey or
    // sign-in prompt on the way.
    await stubPasskeys(page);
    const stepUp = await fake(page, "POST", /^\/v1\/me\/step-up\//, { status: 500, body: apiError("internal", "not expected") });
    await signIn(page, "maker");
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("Straight through");
    await section.getByRole("combobox", { name: "Expires after" }).selectOption("1");
    const made = page.waitForResponse((r) => r.url().endsWith("/v1/me/maker-tokens") && r.request().method() === "POST");
    await section.getByRole("button", { name: "Make token" }).click();
    expect((await made).status()).toBe(201);
    await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
    await expect(page.getByText(/For your safety|confirm it's you/i)).toHaveCount(0);
    await expect(section.getByRole("alert")).toHaveCount(0);
    expect(stepUp.calls).toHaveLength(0);
    expect(await page.evaluate(() => (window as unknown as { __passkeyPrompts: number }).__passkeyPrompts)).toBe(0);
    await section.getByRole("button", { name: "Done" }).click();
    await section.getByRole("listitem").filter({ hasText: "Straight through" }).getByRole("button", { name: "Remove" }).click();
    await expect(section.getByRole("listitem").filter({ hasText: "Straight through" })).toHaveCount(0);
  });
});

test.describe("signing keys", () => {
  test("a key added counts after 72 hours, and can be removed", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#signing-keys");
    const section = page.locator("#signing-keys");
    const add = section.getByRole("button", { name: "Add signing key" });
    await expect(add).toBeDisabled();
    await section.getByRole("textbox", { name: "Name" }).fill("Laptop");
    await section.getByRole("textbox", { name: /Public key/ }).fill(sshPublicKey());
    await add.click();
    const row = section.getByRole("listitem").filter({ hasText: "Laptop" });
    await expect(row).toContainText(`works from ${inDays(3)}`);
    await expect(row.locator("code")).toHaveText(/^SHA256:/);
    await expect(section.getByRole("textbox", { name: /Public key/ })).toHaveValue("");
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
  });

  test("a key that isn't Ed25519 is refused", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#signing-keys");
    const section = page.locator("#signing-keys");
    await section.getByRole("textbox", { name: /Public key/ }).fill("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC7 old@laptop");
    await section.getByRole("button", { name: "Add signing key" }).click();
    await expect(section.getByRole("alert")).toContainText(/Ed25519/i);
    await expect(section.getByRole("listitem")).toHaveCount(0);
  });

  test("an old sign-in is asked to sign in again first", async ({ page }) => {
    await fake(page, "POST", "/v1/me/signing-keys", { status: 403, body: apiError("step_up_required", "this needs a recent sign-in") });
    await signIn(page, "support");
    await page.goto("/account#signing-keys");
    const section = page.locator("#signing-keys");
    await section.getByRole("textbox", { name: /Public key/ }).fill(sshPublicKey());
    await section.getByRole("button", { name: "Add signing key" }).click();
    await expect(section.getByRole("alert")).toContainText("For your safety, this needs a sign-in from the last few minutes.");
    await expect(section.getByRole("link", { name: "sign in again" })).toHaveAttribute("href", "/signin?return_to=%2Faccount%2Fagents%23signing-keys");
  });
});

test.describe("a private repository's token", () => {
  // Only private apps outside GitHub have one, and every seeded app is public on GitHub: Mia's
  // first game is shown as private, and the server's answers about its token are stood in for.
  const unsaved = { saved: false, kind: null, saved_at: null, last_used_at: null, available: true, github: false };
  test.beforeEach(async ({ page }) => {
    await tweak(page, "/v1/apps/mias-first", (app) => ({ ...app, private: true, repo_url: "https://gitlab.com/mia/mias-first" }));
    await signIn(page, "maker");
  });

  test("a token the repository's host doesn't accept is refused with its reason", async ({ page }) => {
    await fake(page, "GET", "/v1/apps/mias-first/repo-token", { status: 200, body: unsaved });
    const put = await fake(page, "PUT", "/v1/apps/mias-first/repo-token", {
      status: 422,
      body: apiError("invalid", "GitLab said 401 Unauthorized: check the token and its scopes", "repo_token"),
    });
    await page.goto("/publish/mias-first/manage");
    const panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "Private repository access" }) });
    await expect(panel).toContainText("Slop needs a read-only token to fetch releases from this private repository and share them.");
    // Guessed from the repository's address.
    await expect(panel.getByRole("combobox", { name: "Host" })).toHaveValue("gitlab");
    await expect(panel).toContainText("Role Reporter, scopes read_api and read_repository.");
    await panel.getByRole("combobox", { name: "Host" }).selectOption("gitea");
    await expect(panel).toContainText("Your settings → Applications → Generate new token");
    await panel.getByRole("combobox", { name: "Host" }).selectOption("gitlab");
    const save = panel.getByRole("button", { name: "Save token" });
    await expect(save).toBeDisabled();
    await panel.getByLabel("Read-only token").fill("glpat-not-a-real-token");
    await save.click();
    await expect(panel.getByRole("alert")).toHaveText("GitLab said 401 Unauthorized: check the token and its scopes");
    await expect(panel.getByRole("button", { name: "Remove token" })).toHaveCount(0);
    expect(put.calls[0].body).toEqual({ kind: "gitlab", token: "glpat-not-a-real-token" });
  });

  test("a saved token can be replaced or removed", async ({ page }) => {
    let saved = false;
    const status = () => ({
      saved,
      kind: saved ? "gitlab" : null,
      saved_at: saved ? "2026-10-23T10:00:00Z" : null,
      last_used_at: null,
      available: true,
      github: false,
    });
    await fake(page, "GET", "/v1/apps/mias-first/repo-token", () => ({ status: 200, body: status() }));
    const put = await fake(page, "PUT", "/v1/apps/mias-first/repo-token", () => {
      saved = true;
      return { status: 200, body: status() };
    });
    const del = await fake(page, "DELETE", "/v1/apps/mias-first/repo-token", () => {
      saved = false;
      return { status: 204, body: undefined };
    });
    await page.goto("/publish/mias-first/manage");
    const panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "Private repository access" }) });
    await panel.getByLabel("Read-only token").fill("glpat-abc");
    await panel.getByRole("button", { name: "Save token" }).click();
    await expect(panel).toContainText("A GitLab token is saved (since Oct 23, 2026). Replace it before it expires.");
    await expect(panel.getByLabel("Read-only token")).toHaveValue("");
    expect(put.calls[0].body).toEqual({ kind: "gitlab", token: "glpat-abc" });
    await expect(panel.getByRole("button", { name: "Replace token" })).toBeDisabled();
    await panel.getByRole("button", { name: "Remove token" }).click();
    await expect(panel).toContainText("Slop needs a read-only token");
    expect(del.calls).toHaveLength(1);
  });

  test("a server that can't keep tokens says so", async ({ page }) => {
    await fake(page, "GET", "/v1/apps/mias-first/repo-token", {
      status: 200,
      body: { saved: false, kind: null, saved_at: null, last_used_at: null, available: false, github: false },
    });
    await page.goto("/publish/mias-first/manage");
    await expect(page.getByText("This server can't keep repository tokens yet.")).toBeVisible();
  });
});

test.describe("the passkey deadline", () => {
  test("before it, publishing pages say from when, with a link that isn't only a colour", async ({ page }) => {
    // Bo first published through the API, which started his 30 days (passkeys.rs).
    const due = (await call("maker2", "GET", "/v1/me")).passkey_due_at as string;
    await signIn(page, "maker2");
    await page.goto("/publish");
    const note = page.getByRole("note").filter({ hasText: "need a passkey" });
    const date = new Date(due).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
    await expect(note).toContainText(`From ${date}, publishing and changing where money goes need a passkey`);
    await expect(note.getByRole("link", { name: "Add a passkey" })).toHaveAttribute("href", "/account/security#passkeys");
    // The notice's link was told apart by colour alone (axe link-in-text-block).
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page }).include(".notice").withRules(["link-in-text-block"]).analyze();
    expect(results.violations.map((v) => v.id)).toEqual([]);
    await expect(note.getByRole("link", { name: "Add a passkey" })).toHaveCSS("text-decoration-line", "underline");
  });

  test("the whole publishing page passes the accessibility check with the notice", async ({ page }) => {
    await signIn(page, "maker2");
    await page.goto("/publish");
    await expect(page.getByRole("note").filter({ hasText: "need a passkey" })).toBeVisible();
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    const serious = results.violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
    expect(serious, serious.join("\n")).toEqual([]);
  });

  test("a creative's People page shows it too", async ({ page }) => {
    await signIn(page, "creative");
    await page.goto("/people/me");
    await expect(page.getByRole("note").filter({ hasText: "need a passkey" })).toContainText("Add a passkey");
  });

  test("grace from staff says until when; with a passkey, nothing shows", async ({ page }) => {
    const grace = new Date(Date.now() + 5 * 86_400_000).toISOString();
    let hasPasskey = false;
    await tweak(page, "/v1/me", (me) => ({ ...me, passkey_grace_until: grace, has_passkey: hasPasskey }));
    await signIn(page, "overdue");
    await page.goto("/publish");
    await expect(page.getByRole("note")).toContainText(
      `You can publish and manage payouts without a passkey until ${inDays(5)}.`,
    );
    hasPasskey = true;
    await page.reload();
    await expect(page.getByRole("heading", { name: /Your apps|Publish/ }).first()).toBeVisible();
    await expect(page.getByRole("note").filter({ hasText: "passkey" })).toHaveCount(0);
  });

  test("an account with no deadline sees nothing", async ({ page }) => {
    await signIn(page, "maker");
    await page.goto("/publish");
    await expect(page.getByRole("heading", { name: /Your apps|Publish/ }).first()).toBeVisible();
    await expect(page.getByRole("note").filter({ hasText: "passkey" })).toHaveCount(0);
  });
});

/** Fills the new-app form far enough to submit it. */
async function newApp(page: import("@playwright/test").Page, name: string) {
  await page.goto("/publish/new");
  await page.getByLabel("Name", { exact: true }).fill(name);
  await page.getByLabel("One-line summary").fill("A test app that never gets made.");
  await page.getByRole("textbox", { name: /Source repository/ }).fill(`${server.fake}mia/never-made`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("checkbox", { name: /Windows/ }).check();
}

/** After the gate, the create call runs again; the second answer shows it did, and nothing is made. */
const retried = { status: 422, body: apiError("invalid", "Second try reached the server.", "slug") };

test.describe("asked for a passkey when publishing", () => {
  test("confirming with a passkey runs it again", async ({ page }) => {
    let n = 0;
    const create = await fake(page, "POST", "/v1/apps", () =>
      ++n === 1 ? { status: 403, body: apiError("passkey_required", "publishing needs a passkey") } : retried,
    );
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/me/step-up/finish", { status: 204, body: undefined });
    await signIn(page, "maker");
    await newApp(page, "Gated Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Passkey needed" });
    await expect(dialog.getByRole("heading", { name: "Confirm with your passkey" })).toBeVisible();
    await expect(dialog).toContainText("Use it once, and it counts for the next 12 hours.");
    await dialog.getByRole("button", { name: "Confirm with your passkey" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByText("Second try reached the server.")).toBeVisible();
    expect(create.calls).toHaveLength(2);
    expect(finish.calls).toHaveLength(1);
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Gated Game");
  });

  test("a passkey that wasn't used says so and the dialog stays", async ({ page }) => {
    await fake(page, "POST", "/v1/apps", { status: 403, body: apiError("passkey_required", "publishing needs a passkey") });
    await stubPasskeys(page, "cancel");
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    await signIn(page, "maker");
    await newApp(page, "Gated Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Passkey needed" });
    await dialog.getByRole("button", { name: "Confirm with your passkey" }).click();
    await expect(dialog.getByRole("alert")).toHaveText("The passkey wasn't used. Try again.");
    await expect(dialog).toBeVisible();
  });

  test("with none yet: I've added one tries again", async ({ page }) => {
    let n = 0;
    const create = await fake(page, "POST", "/v1/apps", () =>
      ++n === 1 ? { status: 403, body: apiError("passkey_required", "add a passkey first", "add") } : retried,
    );
    await signIn(page, "maker");
    await newApp(page, "Gated Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Passkey needed" });
    await expect(dialog.getByRole("heading", { name: "Add a passkey first" })).toBeVisible();
    await expect(dialog.getByRole("link", { name: "Add a passkey" })).toHaveAttribute("target", "_blank");
    await dialog.getByRole("button", { name: "I've added one: try again" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByText("Second try reached the server.")).toBeVisible();
    expect(create.calls).toHaveLength(2);
  });

  test("a browser without passkeys is told to use another", async ({ page }) => {
    await noPasskeys(page);
    await fake(page, "POST", "/v1/apps", { status: 403, body: apiError("passkey_required", "publishing needs a passkey") });
    await signIn(page, "maker");
    await newApp(page, "Gated Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Passkey needed" });
    await expect(dialog).toContainText("This browser can't use passkeys. Try another browser or device.");
    await expect(dialog.getByRole("button", { name: "Confirm with your passkey" })).toBeDisabled();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByText("publishing needs a passkey")).toBeVisible();
  });
});

test.describe("asked to accept the Publisher Agreement", () => {
  test("accepting runs the action again, keeping the form", async ({ page }) => {
    // Every seeded account has accepted it; the server's refusal for a new maker is stood in for.
    let n = 0;
    const create = await fake(page, "POST", "/v1/apps", () =>
      ++n === 1 ? { status: 403, body: apiError("terms_required", "accept the Publisher Agreement first", "publisher") } : retried,
    );
    const accepted = page.waitForRequest((r) => r.url().endsWith("/v1/me/legal/accept"));
    await signIn(page, "maker");
    await newApp(page, "Agreed Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Publisher Agreement" });
    await expect(dialog.getByRole("heading", { name: "Before you continue: the Publisher Agreement" })).toBeVisible();
    await expect(dialog.getByRole("region", { name: "Publisher Agreement, full text" }).or(dialog.locator(".legal-scroll"))).toBeVisible();
    await expect(dialog.getByRole("link", { name: "Open it on its own page" })).toHaveAttribute("href", "/legal/publisher");
    const accept = dialog.getByRole("button", { name: "Accept and continue" });
    await expect(accept).toBeDisabled();
    await dialog.getByRole("checkbox", { name: "I've read and accept the Publisher Agreement." }).check();
    await accept.click();
    expect((await accepted).postDataJSON()).toMatchObject({ kind: "publisher" });
    await expect(dialog).toBeHidden();
    await expect(page.getByText("Second try reached the server.")).toBeVisible();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Agreed Game");
    expect(create.calls).toHaveLength(2);
  });

  test("Not now leaves it, with the reason", async ({ page }) => {
    await fake(page, "POST", "/v1/apps", { status: 403, body: apiError("terms_required", "accept the Publisher Agreement first", "publisher") });
    await signIn(page, "maker");
    await newApp(page, "Agreed Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Publisher Agreement" });
    await dialog.getByRole("button", { name: "Not now" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByText("accept the Publisher Agreement first")).toBeVisible();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue("Agreed Game");
  });

  test("an agreement that won't load says so and can't be accepted", async ({ page }) => {
    await fake(page, "POST", "/v1/apps", { status: 403, body: apiError("terms_required", "accept the Publisher Agreement first", "publisher") });
    await fake(page, "GET", "/v1/legal/publisher", { status: 503, body: apiError("unavailable", "Slop is busy: try again in a minute") });
    await signIn(page, "maker");
    await newApp(page, "Agreed Game");
    await page.getByRole("button", { name: "Create draft" }).click();
    const dialog = page.getByRole("dialog", { name: "Agreement" });
    await expect(dialog.getByRole("alert")).toHaveText("Slop is busy: try again in a minute");
    await expect(dialog.getByRole("button", { name: "Accept and continue" })).toBeDisabled();
  });
});
