// SPDX-License-Identifier: AGPL-3.0-or-later
// What makers manage on their account: AI agent tokens, signing keys, a private repository's
// token, the optional passkey suggestion, and the agreement asked for before publishing (components/AiAgents.tsx,
// SigningKeys.tsx, RepoToken.tsx, PasskeyNotice.tsx, Terms.tsx).
import AxeBuilder from "@axe-core/playwright";
import { generateKeyPairSync } from "node:crypto";

import { accountNav, apiError, call, fake, openAccount, stubPasskeys, tweak } from "./accounts-helpers";
import type { Page } from "@playwright/test";

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
    // The command connects Claude Code with the saved variable, never the token.
    await section.getByRole("button", { name: "Copy the Claude Code command" }).click();
    const command = await page.evaluate(() => navigator.clipboard.readText());
    expect(command).toBe(
      `claude mcp add --transport http --scope user slop ${server.url}/v1/maker/mcp --header 'Authorization: Bearer \${SLOP_TOKEN}'`,
    );
    await section.getByRole("button", { name: "Done" }).click();
    const row = section.getByRole("listitem").filter({ hasText: "Read-only helper" });
    await expect(row).toContainText("looks · lantern-keep");
    await expect(row.getByText("expires in 7 days")).toHaveAttribute("title", inDays(7));
    // A token made for a week isn't flagged: it's meant to be short.
    await expect(row.getByText("Expires soon")).toHaveCount(0);
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
  });

  test("a publishing token works without a passkey, and says nothing about one", async ({ page }) => {
    await signIn(page, "maker");
    await page.goto("/account#ai-agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("Publisher bot");
    await section.getByRole("combobox", { name: "Expires after" }).selectOption("1");
    await section.getByRole("button", { name: "Make token" }).click();
    await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
    await expect(section.getByText(/passkey/i)).toHaveCount(0);
    await section.getByRole("button", { name: "Done" }).click();
    const row = section.getByRole("listitem").filter({ hasText: "Publisher bot" });
    await expect(row).toContainText("publishes · all apps");
    await expect(row.getByText("expires within a day")).toHaveAttribute("title", inDays(1));
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

const seriousProblems = async (page: Page, selector: string) => {
  const results = await new AxeBuilder({ page }).include(selector).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  return results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
};

test.describe("AI agent token setup and expiry", () => {
  test("a 12-month token's setup never puts the token in a command, only in the one save step", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: server.url });
    await signIn(page, "maker");
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");
    await section.getByRole("textbox", { name: "Name" }).fill("Year-long helper");
    await section.getByRole("combobox", { name: "Can" }).selectOption("read");
    const lifetime = section.getByRole("combobox", { name: "Expires after" });
    await expect(lifetime.locator("option")).toHaveText(["1 day", "7 days", "30 days", "3 months", "6 months", "12 months"]);
    await lifetime.selectOption({ label: "12 months" });
    const made = page.waitForResponse((r) => r.url().endsWith("/v1/me/maker-tokens") && r.request().method() === "POST");
    await section.getByRole("button", { name: "Make token" }).click();
    const res = await made;
    expect(res.request().postDataJSON()).toMatchObject({ name: "Year-long helper", days: 365 });
    const { secret, id } = (await res.json()) as { secret: string; id: string };
    expect(secret).toMatch(/^slopmk_/);

    const setup = section.getByRole("region", { name: "Set up “Year-long helper” on your computer" });
    await expect(setup.getByRole("note")).toContainText("Never paste this token into a chat with Claude, ChatGPT, Codex, Grok or any AI.");
    await expect(setup.getByRole("note")).toContainText("It's a password for your Slop account.");
    await expect(setup.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
    await expect(setup.getByText(/not after Claude Code's/)).toBeVisible();
    await setup.getByRole("button", { name: "Copy the token" }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(secret);

    // Every computer and every tool: the token appears in the token box and the one command that
    // saves it on the computer, and nowhere else.
    const computers = setup.getByRole("tablist", { name: "Your computer" });
    const tools = setup.getByRole("tablist", { name: "Your AI tool" });
    for (const os of ["Windows", "macOS", "Linux"]) {
      await computers.getByRole("tab", { name: os }).click();
      await expect(computers.getByRole("tab", { name: os })).toHaveAttribute("aria-selected", "true");
      for (const tool of ["Claude Code", "Codex (OpenAI)", "Grok Build (xAI)", "Other AI tools"]) {
        await tools.getByRole("tab", { name: tool }).click();
        for (const more of await setup.locator("details").all()) await more.evaluate((d) => ((d as HTMLDetailsElement).open = true));
        const blocks = await setup.locator("pre").allTextContents();
        const withToken = blocks.filter((b) => b.includes(secret));
        const save = os === "Windows" ? `setx SLOP_TOKEN "${secret}"` : `echo 'export SLOP_TOKEN="${secret}"' >> ${os === "macOS" ? "~/.zshrc" : "~/.bashrc"}`;
        expect(withToken, `${os} / ${tool}`).toEqual([secret, save]);
        expect(blocks.join("\n"), `${os} / ${tool}`).not.toContain(`Bearer ${secret}`);
        // Nothing else on the panel shows it either (the header help says "your token").
        const text = (await setup.innerText()).split(secret).length - 1;
        expect(text, `${os} / ${tool}`).toBe(2);
      }
    }
    await computers.getByRole("tab", { name: "Windows" }).click();
    await tools.getByRole("tab", { name: "Codex (OpenAI)" }).click();
    await setup.getByRole("button", { name: "Copy the Codex command" }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
      `codex mcp add slop --url ${server.url}/v1/maker/mcp --bearer-token-env-var SLOP_TOKEN`,
    );
    await tools.getByRole("tab", { name: "Other AI tools" }).click();
    await expect(setup.getByText("If a tool asks for the token in its chat, don't paste it there:")).toBeVisible();
    await expect(setup.getByText(`${server.url}/v1/maker/mcp`, { exact: true })).toBeVisible();
    await setup.getByRole("button", { name: "Copy what to ask your AI" }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("Read maker_guide, then publish my game on Slop.");

    // The tabs work by keyboard.
    await computers.getByRole("tab", { name: "Windows" }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(computers.getByRole("tab", { name: "macOS" })).toBeFocused();
    await expect(computers.getByRole("tab", { name: "macOS" })).toHaveAttribute("aria-selected", "true");
    await expect(setup.getByText("Press ⌘ Command and Space together")).toBeVisible();
    await page.keyboard.press("End");
    await expect(computers.getByRole("tab", { name: "Linux" })).toBeFocused();
    await expect(setup.getByText(`echo 'export SLOP_TOKEN="${secret}"' >> ~/.bashrc`)).toBeVisible();

    expect(await seriousProblems(page, "#ai-agents")).toEqual([]);

    await setup.getByRole("button", { name: "Done" }).click();
    await expect(setup).toHaveCount(0);
    const row = section.getByRole("listitem").filter({ hasText: "Year-long helper" });
    await expect(row.getByText("expires in 365 days")).toHaveAttribute("title", inDays(365));
    await call("maker", "DELETE", `/v1/me/maker-tokens/${id}`);
  });

  test("a token near its expiry is flagged, badged, and Make a replacement fills in its settings", async ({ page }) => {
    // Tokens can't be aged on the test server: the list gets one expiring in 3 days and one that
    // expired yesterday, beside the real ones.
    const day = 86_400_000;
    const iso = (days: number) => new Date(Date.now() + days * day).toISOString();
    const fakeToken = (id: string, name: string, made: number, expires: number, extra: object = {}) => ({
      id,
      name,
      scope: "publish",
      apps: ["lantern-keep"],
      people: true,
      created_at: iso(made),
      expires_at: iso(expires),
      last_used_at: iso(-1),
      expired: expires <= 0,
      ...extra,
    });
    await tweak(page, "/v1/me/maker-tokens", (list: object[]) => [
      fakeToken("00000000-0000-7000-8000-000000000001", "Old laptop", -87, 3),
      fakeToken("00000000-0000-7000-8000-000000000002", "Studio PC", -366, -1, { scope: "read", apps: [], people: false }),
      ...list,
    ]);
    await signIn(page, "maker");
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");

    // The section's badge, read out with its name.
    await expect(accountNav(page).getByRole("link", { name: /AI agents & keys.*AI agent token has expired/ })).toBeVisible();

    const old = section.getByRole("listitem").filter({ hasText: "Old laptop" });
    await expect(old.getByText("Expires soon")).toBeVisible();
    await expect(old.getByText("expires in 3 days")).toHaveAttribute("title", inDays(3));
    await expect(old).toHaveClass(/token-soon/);
    const studio = section.getByRole("listitem").filter({ hasText: "Studio PC" });
    await expect(studio.getByText("Expired", { exact: true })).toBeVisible();
    await expect(studio.getByText(/expired on .*: it no longer works/)).toBeVisible();
    await expect(studio.getByRole("button", { name: "Make a replacement for Studio PC" })).toBeVisible();

    expect(await seriousProblems(page, "#ai-agents")).toEqual([]);

    await old.getByRole("button", { name: "Make a replacement for Old laptop" }).click();
    const form = section.getByRole("form", { name: "Make a token" });
    await expect(form.getByRole("status")).toContainText("Making a replacement for “Old laptop”, with the same settings.");
    await expect(form.getByRole("status")).toContainText(`The old token keeps working until ${inDays(3)}`);
    await expect(form.getByRole("textbox", { name: "Name" })).toHaveValue("Old laptop");
    await expect(form.getByRole("textbox", { name: "Name" })).toBeFocused();
    await expect(form.getByRole("combobox", { name: "Can" })).toHaveValue("publish");
    await expect(form.getByRole("checkbox", { name: "Only Lantern Keep" })).toBeChecked();
    await expect(form.getByRole("checkbox", { name: /People jobs too/ })).toBeChecked();
    await expect(form.getByRole("combobox", { name: "Expires after" })).toHaveValue("90");
    // Replacing the expired one picks up its own settings, a year.
    await studio.getByRole("button", { name: "Make a replacement for Studio PC" }).click();
    await expect(form.getByRole("textbox", { name: "Name" })).toHaveValue("Studio PC");
    await expect(form.getByRole("combobox", { name: "Can" })).toHaveValue("read");
    await expect(form.getByRole("radio", { name: /All my apps/ })).toBeChecked();
    await expect(form.getByRole("checkbox", { name: /People jobs too/ })).not.toBeChecked();
    await expect(form.getByRole("combobox", { name: "Expires after" })).toHaveValue("365");
    await expect(form.getByRole("status")).toContainText("(it has expired)");

    // Making it sends those settings; the old ones are left as they were.
    const made = page.waitForResponse((r) => r.url().endsWith("/v1/me/maker-tokens") && r.request().method() === "POST");
    await form.getByRole("button", { name: "Make token" }).click();
    const res = await made;
    expect(res.request().postDataJSON()).toEqual({ name: "Studio PC", scope: "read", apps: [], people: false, days: 365 });
    const { id } = (await res.json()) as { id: string };
    await expect(section.getByRole("region", { name: "Set up “Studio PC” on your computer" })).toBeVisible();
    await expect(form.getByRole("status")).toHaveCount(0);
    // The new one replaces the expired one, so it's no longer flagged, and the badge goes to the
    // one still expiring.
    await expect(studio.first().getByText(/replaced by a newer “Studio PC”/)).toBeVisible();
    await expect(accountNav(page).getByRole("link", { name: /AI agents & keys.*an AI agent token expires soon/ })).toBeVisible();
    expect((await call("maker", "GET", "/v1/me/maker-tokens")).map((t: { id: string }) => t.id)).toContain(id);
    await call("maker", "DELETE", `/v1/me/maker-tokens/${id}`);
  });
});

test.describe("AI agent tokens and the Publisher Agreement", () => {
  test("a first publishing token shows the agreement right here, and is made once it's accepted", async ({ page }) => {
    // Nia accepted only the Terms of Service (e2e_server/seed.rs).
    await signIn(page, "newcomer");
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");
    const hint = section.getByText("Publishing tokens need the Publisher Agreement: it's shown when you make the token.");
    await expect(hint).toBeVisible();
    await section.getByRole("combobox", { name: "Can" }).selectOption("read");
    await expect(hint).toHaveCount(0);
    await section.getByRole("combobox", { name: "Can" }).selectOption("publish");
    await section.getByRole("textbox", { name: "Name" }).fill("First agent");
    await section.getByRole("combobox", { name: "Expires after" }).selectOption("1");
    await section.getByRole("button", { name: "Make token" }).click();

    const dialog = page.getByRole("dialog", { name: "Publisher Agreement" });
    await expect(dialog.getByRole("heading", { name: "Before you continue: the Publisher Agreement" })).toBeVisible();
    // Not now: no token, and the page says why.
    await dialog.getByRole("button", { name: "Not now" }).click();
    await expect(dialog).toBeHidden();
    await expect(section.getByRole("alert")).toHaveText(
      "A token that publishes needs the Publisher Agreement: accept it to make one, or choose Look.",
    );
    await expect(section.getByText("Copy it now")).toHaveCount(0);
    expect(await call("newcomer", "GET", "/v1/me/maker-tokens")).toEqual([]);

    // Again, accepting it this time: the token is made, nothing retyped.
    const made = page.waitForResponse((r) => r.url().endsWith("/v1/me/maker-tokens") && r.request().method() === "POST" && r.status() === 201);
    await section.getByRole("button", { name: "Make token" }).click();
    const accept = dialog.getByRole("button", { name: "Accept and continue" });
    await expect(accept).toBeDisabled();
    await dialog.getByRole("checkbox", { name: "I've read and accept the Publisher Agreement." }).check();
    await accept.click();
    await made;
    await expect(dialog).toBeHidden();
    await expect(section.getByText("Copy it now: Slop won't show this token again.")).toBeVisible();
    await expect(section.getByRole("alert")).toHaveCount(0);
    await expect(hint).toHaveCount(0);
    await section.getByRole("button", { name: "Done" }).click();
    const row = section.getByRole("listitem").filter({ hasText: "First agent" });
    await expect(row).toContainText("publishes · all apps");
    expect((await call("newcomer", "GET", "/v1/me/legal/publisher")).accepted).toBe(true);
    // Nothing waits on the AI agents page now.
    await expect(section.getByRole("button", { name: /Review and accept/ })).toHaveCount(0);

    await row.getByRole("button", { name: "Remove" }).click();
    await expect(row).toHaveCount(0);
  });

  test("a changed agreement waits on the AI agents page, with its accept control", async ({ page }) => {
    const publisher = await call<{ version: string }>(null, "GET", "/v1/legal/publisher");
    let accepted = false;
    // As legal.rs answers once a new version Mia hasn't accepted is in effect.
    await fake(page, "GET", "/v1/me/legal", () => ({
      status: 200,
      body: accepted
        ? []
        : [
            {
              kind: "publisher",
              title: "Publisher Agreement",
              version: publisher.version,
              summary: "Clearer rules for AI agents.",
              effective_at: "2026-10-01T00:00:00Z",
              in_effect: true,
              first: false,
              action: "accept",
            },
          ],
    }));
    page.on("request", (r) => {
      if (r.url().endsWith("/v1/me/legal/accept") && r.method() === "POST") accepted = true;
    });
    await signIn(page, "maker");
    await openAccount(page, "agents");
    const section = page.locator("#ai-agents");
    const notice = section.getByRole("status").filter({ hasText: "Your AI agent can't publish until you accept the Publisher Agreement" });
    await expect(notice).toContainText("it changed on Oct 1, 2026. Clearer rules for AI agents.");
    const badge = page.getByRole("navigation", { name: "Account settings" }).getByText("1 to accept");
    await expect(badge).toHaveCount(1);
    await notice.getByRole("button", { name: `Review and accept version ${publisher.version}` }).click();
    const dialog = page.getByRole("dialog", { name: "Publisher Agreement" });
    await dialog.getByRole("checkbox", { name: "I've read and accept the Publisher Agreement." }).check();
    await dialog.getByRole("button", { name: "Accept and continue" }).click();
    await expect(dialog).toBeHidden();
    await expect(notice).toHaveCount(0);
    // The account's agreements badge is reloaded too.
    await expect(badge).toHaveCount(0);
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

  test("adding a key never asks to confirm it's you", async ({ page }) => {
    // No recent sign-in needed (maker_keys.rs): the server's answer is whatever it is, shown as is.
    const stepUp = await fake(page, "POST", /^\/v1\/me\/step-up\//, { status: 500, body: apiError("internal", "not expected") });
    await fake(page, "POST", "/v1/me/signing-keys", { status: 409, body: apiError("conflict", "that key is already on your account") });
    await signIn(page, "support");
    await page.goto("/account#signing-keys");
    const section = page.locator("#signing-keys");
    await section.getByRole("textbox", { name: /Public key/ }).fill(sshPublicKey());
    await section.getByRole("button", { name: "Add signing key" }).click();
    await expect(section.getByRole("alert")).toHaveText("that key is already on your account");
    await expect(section.getByRole("link", { name: "sign in again" })).toHaveCount(0);
    expect(stepUp.calls).toHaveLength(0);
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

test.describe("the optional passkey suggestion", () => {
  test("publishing pages suggest one gently, with a link that isn't only a colour", async ({ page }) => {
    await signIn(page, "maker2");
    await page.goto("/publish");
    const note = page.getByRole("note").filter({ hasText: "passkey" });
    await expect(note).toHaveText("Optional: a passkey lets you sign in quickly, and nobody can phish it.");
    await expect(note.getByRole("link", { name: "passkey" })).toHaveAttribute("href", "/account/security#passkeys");
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page }).include("[role=note]").withRules(["link-in-text-block"]).analyze();
    expect(results.violations.map((v) => v.id)).toEqual([]);
    await expect(note.getByRole("link", { name: "passkey" })).toHaveCSS("text-decoration-line", "underline");
  });

  test("the whole publishing page passes the accessibility check with it", async ({ page }) => {
    await signIn(page, "maker2");
    await page.goto("/publish");
    await expect(page.getByRole("note").filter({ hasText: "passkey" })).toBeVisible();
    await page.waitForLoadState("networkidle");
    const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    const serious = results.violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
    expect(serious, serious.join("\n")).toEqual([]);
  });

  test("a creative's People page suggests it too", async ({ page }) => {
    await signIn(page, "creative");
    await page.goto("/people/me");
    await expect(page.getByRole("note").filter({ hasText: "Optional: a passkey" })).toBeVisible();
  });

  test("with a passkey, nothing shows", async ({ page }) => {
    await tweak(page, "/v1/me", (me) => ({ ...me, has_passkey: true }));
    await signIn(page, "overdue");
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
