// SPDX-License-Identifier: AGPL-3.0-or-later
// The account page (web/src/pages/Auth.tsx Account, components/AccountSections.tsx): privacy,
// where you're signed in, sign-ins and passkeys, notifications, agreements, your data.
// Sue Support is the account these tests change, and each test puts back what it changed.
import { apiError, call, fake, noPasskeys, passkeyOptions, stubPasskeys, tweak } from "./accounts-helpers";
import type { Page } from "@playwright/test";

import { expect, server, signIn, test } from "./fixtures";

test.describe("privacy", () => {
  test("signed out, the account page asks you to sign in", async ({ page }) => {
    await page.goto("/account");
    await expect(page.getByText("to see your account.")).toBeVisible();
    await expect(page.getByRole("main").getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/signin?return_to=/account");
  });

  test("each switch saves, and shows what's saved", async ({ page }) => {
    await signIn(page, "support");
    try {
      await page.goto("/account#privacy");
      await expect(page.getByRole("heading", { name: "Your account" })).toBeVisible();
      const kv = page.locator("dl.kv").first();
      await expect(kv).toContainText("Sue Support");
      await expect(kv).toContainText("None shared by GitHub");
      await expect(page.getByRole("link", { name: "See your profile" })).toHaveAttribute("href", /^\/u\//);
      for (const [label, field] of [
        [/Show my votes and reviews on my profile/, "profile_public"],
        [/Share my playtime/, "share_playtime"],
        [/Recommendations based on my activity/, "personalize"],
      ] as const) {
        const box = page.getByRole("checkbox", { name: label });
        await expect(box).toBeChecked();
        await box.click();
        await expect(box).not.toBeChecked();
        expect((await call("support", "GET", "/v1/me"))[field]).toBe(false);
      }
      await page.reload();
      await expect(page.getByRole("checkbox", { name: /Share my playtime/ })).not.toBeChecked();
      await page.getByRole("checkbox", { name: /Share my playtime/ }).click();
      await expect(page.getByRole("checkbox", { name: /Share my playtime/ })).toBeChecked();
      expect((await call("support", "GET", "/v1/me")).share_playtime).toBe(true);
    } finally {
      await call("support", "PATCH", "/v1/me", { profile_public: true, share_playtime: true, personalize: true });
    }
  });

  test("a switch that doesn't save says so and stays as it was", async ({ page }) => {
    await fake(page, "PATCH", "/v1/me", { status: 503, body: apiError("unavailable", "Slop is busy: try again in a minute") });
    await signIn(page, "support");
    await page.goto("/account#privacy");
    const box = page.getByRole("checkbox", { name: /Share my playtime/ });
    await box.click();
    await expect(page.getByRole("alert")).toHaveText("Slop is busy: try again in a minute");
    await expect(box).toBeChecked();
  });

  test("a browser sending Global Privacy Control is told it's honoured", async ({ page }) => {
    // The server notes the header for good (auth/mod.rs); only the page's side is shown here.
    await tweak(page, "/v1/me", (me) => ({ ...me, gpc: true }));
    await signIn(page, "support");
    await page.goto("/account#privacy");
    await expect(page.getByText("Your browser sends Global Privacy Control")).toBeVisible();
  });

  test("without Global Privacy Control there's no note", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#privacy");
    await expect(page.getByRole("checkbox", { name: /Share my playtime/ })).toBeVisible();
    await expect(page.getByText("Your browser sends Global Privacy Control")).toHaveCount(0);
  });
});

test.describe("where you're signed in", () => {
  test("lists each kind of session, and a sign-out that fails says why", async ({ page }) => {
    await tweak(page, "/v1/me/sessions", (list: unknown[]) => [
      ...list,
      {
        id: "01a00000-0000-7000-8000-00000000a1a1",
        kind: "agent",
        user_agent: "Claude on my laptop",
        created_at: "2026-10-20T10:00:00Z",
        last_seen_at: "2026-10-21T10:00:00Z",
        current: false,
      },
      {
        id: "01a00000-0000-7000-8000-00000000a2a2",
        kind: "desktop",
        user_agent: null,
        created_at: "2026-10-20T10:00:00Z",
        last_seen_at: "2026-10-21T10:00:00Z",
        current: false,
      },
      {
        id: "01a00000-0000-7000-8000-00000000a3a3",
        kind: "web",
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
        created_at: "2026-10-20T10:00:00Z",
        last_seen_at: "2026-10-21T10:00:00Z",
        current: false,
      },
    ]);
    await signIn(page, "support");
    await page.goto("/account");
    const sessions = page.locator("ul.sessions").last();
    await expect(sessions.getByRole("listitem").filter({ hasText: "Claude on my laptop" })).toBeVisible();
    await expect(sessions.getByRole("listitem").filter({ hasText: "Slop app" })).toBeVisible();
    await expect(sessions.getByRole("listitem").filter({ hasText: "Safari on macOS" })).toBeVisible();
    await expect(sessions.getByRole("listitem").filter({ hasText: "THIS DEVICE" }).getByRole("button")).toHaveCount(0);
    // The made-up session isn't the server's: it refuses, and the page says so.
    await sessions.getByRole("listitem").filter({ hasText: "Safari on macOS" }).getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("alert")).toHaveText("not found");
    await expect(sessions.getByRole("listitem").filter({ hasText: "Safari on macOS" })).toBeVisible();
  });

  test("Sign out signs this browser out and goes home", async ({ page }) => {
    // Signing out for real would end the seeded session every later test uses: the server's
    // answer is stood in for, and the browser forgets the cookie as it would.
    const out = await fake(page, "POST", "/v1/auth/signout", async () => {
      await page.context().clearCookies();
      return { status: 204, body: undefined };
    });
    await signIn(page, "support");
    await page.goto("/account");
    await page.getByRole("button", { name: "Sign out", exact: true }).last().click();
    await expect(page).toHaveURL(`${server.url}/`);
    await expect(page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Sign in" })).toBeVisible();
    expect(out.calls).toHaveLength(1);
  });
});

test.describe("sign-ins", () => {
  test("linking GitHub goes to GitHub, and one that fails says so", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#sign-ins");
    const section = page.locator("#sign-ins");
    // The seed's accounts have no linked sign-ins, so none can be unlinked.
    await expect(section.getByRole("button", { name: "Unlink" })).toHaveCount(0);
    const link = section.getByRole("link", { name: "Link GitHub" });
    await expect(link).toHaveAttribute("href", "/v1/auth/github/link?return_to=%2Faccount%23sign-ins");
    await link.click();
    await page.waitForURL(`${server.fake}login/oauth/authorize?**`);
    const state = new URL(page.url()).searchParams.get("state")!;
    // Back from GitHub; the fake can't finish signing in.
    await page.goto(`/v1/auth/github/callback?code=not-a-real-code&state=${encodeURIComponent(state)}`);
    await expect(page).toHaveURL(`${server.url}/account?error=provider#sign-ins`);
    await expect(section.getByText("That sign-in didn't work. Please try again.")).toBeVisible();
  });

  test("says when a sign-in was linked, or already was", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account?linked=github#sign-ins");
    await expect(page.getByRole("status").filter({ hasText: "GitHub is linked" })).toContainText(
      "you can sign in with it, and Slop knows the repositories it owns are yours.",
    );
    await page.goto("/account?linked=already#sign-ins");
    await expect(page.getByText("That account was already linked.")).toBeVisible();
  });

  test("unlinking needs a recent sign-in or a passkey, and keeps the last sign-in", async ({ page }) => {
    // Linked sign-ins need a real provider, so Sue's two are stood in for.
    let identities = [
      { provider: "github", provider_subject: "1001", login: "sue-gh", created_at: "2026-10-01T10:00:00Z", last_sign_in_at: "2026-10-22T10:00:00Z" },
      { provider: "gitlab", provider_subject: "2002", login: "sue-gl", created_at: "2026-10-02T10:00:00Z", last_sign_in_at: "2026-10-20T10:00:00Z" },
    ];
    await fake(page, "GET", "/v1/me/identities", () => ({ status: 200, body: identities }));
    let refuse = true;
    const unlink = await fake(page, "DELETE", /^\/v1\/me\/identities\/gitlab\/2002$/, () => {
      if (refuse) return { status: 403, body: apiError("step_up_required", "this needs a recent sign-in") };
      identities = identities.slice(0, 1);
      return { status: 204, body: undefined };
    });
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    await fake(page, "POST", "/v1/me/step-up/finish", () => {
      refuse = false;
      return { status: 204, body: undefined };
    });
    await signIn(page, "support");
    await page.goto("/account#sign-ins");
    const section = page.locator("#sign-ins");
    await expect(section.getByText("sue-gh")).toBeVisible();
    const gitlab = section.getByRole("listitem").filter({ hasText: "sue-gl" });
    await gitlab.getByRole("button", { name: "Unlink" }).click();
    const alert = section.getByRole("alert");
    await expect(alert).toContainText("For your safety, this needs a sign-in from the last few minutes.");
    await expect(alert.getByRole("link", { name: "sign in again" })).toHaveAttribute("href", "/signin?return_to=%2Faccount%23sign-ins");
    await alert.getByRole("button", { name: "Confirm with a passkey" }).click();
    await expect(section.getByRole("alert")).toHaveCount(0);
    await gitlab.getByRole("button", { name: "Unlink" }).click();
    await expect(gitlab).toHaveCount(0);
    // One left: it can't be unlinked.
    await expect(section.getByText("sue-gh")).toBeVisible();
    await expect(section.getByRole("button", { name: "Unlink" })).toHaveCount(0);
    expect(unlink.calls).toHaveLength(2);
  });

  test("an account with passkeys confirms with one before linking", async ({ page }) => {
    // What the server sends an account with passkeys to (auth/mod.rs link_start).
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/me/step-up/finish", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto("/account?confirm_link=github#sign-ins");
    const notice = page.getByRole("status").filter({ hasText: "To link GitHub, confirm it's you with one of your passkeys." });
    await expect(notice).toBeVisible();
    await notice.getByRole("button", { name: "Confirm with a passkey" }).click();
    // Confirmed: on to linking, which goes to GitHub.
    await page.waitForURL(`${server.fake}login/oauth/authorize?**`);
    expect(finish.calls[0].body).toMatchObject({ challenge_id: "s1" });
  });

  test("a confirmation that fails says why", async ({ page }) => {
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/me/step-up/start", {
      status: 409,
      body: apiError("conflict", "add a passkey first, or sign in again to confirm"),
    });
    await signIn(page, "support");
    await page.goto("/account?confirm_link=github#sign-ins");
    await page.getByRole("button", { name: "Confirm with a passkey" }).click();
    await expect(page.getByRole("alert")).toHaveText("add a passkey first, or sign in again to confirm");
  });
});

test.describe("passkeys", () => {
  test("adding one (named, or just 'Passkey') and removing one, which asks for a passkey first", async ({ page }) => {
    // Passkeys can't work on 127.0.0.1 (crates/slop-api/src/passkeys.rs): the browser's prompt and
    // the server's answers are stood in for; the page is real.
    let keys: Array<{ id: string; name: string; created_at: string; last_used_at: string | null }> = [];
    await fake(page, "GET", "/v1/me/passkeys", () => ({ status: 200, body: keys }));
    await fake(page, "POST", "/v1/me/passkeys/register/start", { status: 200, body: { challenge_id: "r1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/me/passkeys/register/finish", (route) => {
      const body = route.request().postDataJSON() as { name: string };
      keys = [...keys, { id: `k${keys.length + 1}`, name: body.name, created_at: "2026-10-23T10:00:00Z", last_used_at: null }];
      return { status: 201, body: keys[keys.length - 1] };
    });
    let proved = false;
    const remove = await fake(page, "DELETE", /^\/v1\/me\/passkeys\/k1$/, () => {
      if (!proved) return { status: 403, body: apiError("step_up_required", "this needs a recent sign-in") };
      keys = keys.filter((k) => k.id !== "k1");
      return { status: 204, body: undefined };
    });
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    await fake(page, "POST", "/v1/me/step-up/finish", () => {
      proved = true;
      return { status: 204, body: undefined };
    });
    await stubPasskeys(page);
    await signIn(page, "support");
    await page.goto("/account#sign-ins");
    const section = page.locator(".passkeys");
    await expect(section.getByRole("heading", { name: "Passkeys" })).toBeVisible();
    await section.getByRole("textbox", { name: "Name for the passkey" }).fill("MacBook");
    await section.getByRole("button", { name: "Add a passkey" }).click();
    await expect(section.getByRole("listitem").filter({ hasText: "MacBook" })).toContainText("added Oct 23, 2026");
    await expect(section.getByRole("textbox", { name: "Name for the passkey" })).toHaveValue("");
    await section.getByRole("button", { name: "Add a passkey" }).click();
    await expect(section.getByRole("listitem").filter({ hasText: "Passkey" })).toBeVisible();
    expect(finish.calls.map((c) => (c.body as { name: string }).name)).toEqual(["MacBook", "Passkey"]);

    const mac = section.getByRole("listitem").filter({ hasText: "MacBook" });
    await mac.getByRole("button", { name: "Remove" }).click();
    await expect(section.getByRole("alert")).toContainText("For your safety, this needs a sign-in from the last few minutes.");
    await section.getByRole("button", { name: "Confirm with a passkey" }).click();
    await mac.getByRole("button", { name: "Remove" }).click();
    await expect(mac).toHaveCount(0);
    expect(remove.calls).toHaveLength(2);
    expect(await page.evaluate(() => (window as unknown as { __passkeyPrompts: number }).__passkeyPrompts)).toBe(3);
  });

  test("closing the device's prompt says the passkey wasn't made", async ({ page }) => {
    await stubPasskeys(page, "cancel");
    await fake(page, "POST", "/v1/me/passkeys/register/start", { status: 200, body: { challenge_id: "r1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/me/passkeys/register/finish", { status: 201, body: {} });
    await signIn(page, "support");
    await page.goto("/account#sign-ins");
    await page.locator(".passkeys").getByRole("button", { name: "Add a passkey" }).click();
    await expect(page.locator(".passkeys").getByRole("alert")).toHaveText("The operation either timed out or was not allowed.");
    await expect(page.locator(".passkeys").getByRole("button", { name: "Add a passkey" })).toBeEnabled();
    expect(finish.calls).toHaveLength(0);
  });

  test("a browser without passkeys doesn't offer them", async ({ page }) => {
    await noPasskeys(page);
    await signIn(page, "support");
    await page.goto("/account#sign-ins");
    await expect(page.getByRole("link", { name: "Link GitHub" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Passkeys" })).toHaveCount(0);
  });
});

/** Headless Chrome refuses notifications outright; a person's browser asks first, and they say yes. */
async function askableNotifications(page: Page) {
  await page.addInitScript(() => {
    let permission: NotificationPermission = "default";
    Object.defineProperty(Notification, "permission", { get: () => permission });
    Notification.requestPermission = async () => (permission = "granted");
  });
}

test.describe("notifications", () => {
  test("each email choice saves, and the page says when nothing is emailed", async ({ page }) => {
    await signIn(page, "support");
    try {
      await page.goto("/account#notifications");
      const section = page.locator("#notifications");
      await expect(section).toContainText("(you don't have an email on your account, so nothing is emailed)");
      await expect(section).toContainText("This server doesn't send email yet");
      const jobs = section.getByRole("checkbox", { name: /Email me about people jobs/ });
      await expect(jobs).toBeChecked();
      await jobs.click();
      await expect(jobs).not.toBeChecked();
      const saved = await call("support", "GET", "/v1/me/email-prefs");
      expect(saved.categories.find((c: { category: string }) => c.category === "jobs").email).toBe(false);
      await page.reload();
      await expect(section.getByRole("checkbox", { name: /Email me about people jobs/ })).not.toBeChecked();
      await section.getByRole("checkbox", { name: /Email me about people jobs/ }).click();
      await expect(section.getByRole("checkbox", { name: /Email me about people jobs/ })).toBeChecked();
    } finally {
      await call("support", "PUT", "/v1/me/email-prefs", { jobs: true });
    }
  });

  test("an account with email on a server that sends it says neither caveat; a failed save says why", async ({ page }) => {
    await tweak(page, "/v1/me/email-prefs", (p) => ({ ...p, has_email: true, email_on: true }));
    await fake(page, "PUT", "/v1/me/email-prefs", { status: 400, body: apiError("invalid", "that isn't a kind of notification") });
    await signIn(page, "support");
    await page.goto("/account#notifications");
    const section = page.locator("#notifications");
    await expect(section).toContainText("Choose what's also emailed to you. Slop never sends marketing.");
    await section.getByRole("checkbox", { name: /Email me about your teams/ }).click();
    await expect(section.getByRole("alert")).toHaveText("that isn't a kind of notification");
    await expect(section.getByRole("checkbox", { name: /Email me about your teams/ })).toBeChecked();
  });

  test("phone and browser notifications: off, and this server can't send them", async ({ page }) => {
    await askableNotifications(page);
    await signIn(page, "support");
    await page.goto("/account#notifications");
    const push = page.getByRole("checkbox", { name: /Notifications on this phone or browser/ });
    await expect(push).not.toBeChecked();
    await push.click();
    // The test server has no push keys (main.rs: push: None).
    await expect(page.getByText("Notifications on phones aren't switched on on this server yet.")).toBeVisible();
  });

  test("phone and browser notifications turn on and off", async ({ page }) => {
    await askableNotifications(page);
    // The browser's push service is stood in for (headless Chrome has none), and the server's key.
    await page.addInitScript(() => {
      let sub: unknown = null;
      const fakeSub = {
        endpoint: "https://push.example/abc",
        toJSON: () => ({ endpoint: "https://push.example/abc", keys: { p256dh: "BP", auth: "AU" } }),
        unsubscribe: async () => {
          sub = null;
          return true;
        },
      };
      PushManager.prototype.subscribe = async function () {
        sub = fakeSub;
        return fakeSub as unknown as PushSubscription;
      };
      PushManager.prototype.getSubscription = async function () {
        return sub as PushSubscription | null;
      };
    });
    await fake(page, "GET", "/v1/push/key", { status: 200, body: { public_key: "BEl62iUYgUivxIkv69yViEuiBIa-Ib9-SkvMeAtA3LFgDzkrxZJjSgSnfckjBJuBkr3qBUYIHBQFLXYp5Nksh8U" } });
    const on = await fake(page, "POST", "/v1/me/push", { status: 204, body: undefined });
    const off = await fake(page, "DELETE", "/v1/me/push", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto("/account#notifications");
    const push = page.getByRole("checkbox", { name: /Notifications on this phone or browser/ });
    await push.click();
    await expect(push).toBeChecked();
    expect(on.calls[0].body).toEqual({ endpoint: "https://push.example/abc", keys: { p256dh: "BP", auth: "AU" } });
    await push.click();
    await expect(push).not.toBeChecked();
    expect(off.calls[0].body).toEqual({ endpoint: "https://push.example/abc" });
  });

  test("blocked notifications say where to change it", async ({ page }) => {
    await page.addInitScript(() => Object.defineProperty(Notification, "permission", { get: () => "denied" }));
    await signIn(page, "support");
    await page.goto("/account#notifications");
    await expect(page.getByText("Notifications are blocked for Slop in this browser's settings.")).toBeVisible();
  });

  test("on an iPhone, Slop must be on the Home Screen first", async ({ browser }) => {
    const context = await browser.newContext({
      baseURL: server.url,
      userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
    });
    const page = await context.newPage();
    await signIn(page, "support");
    await page.goto("/account#notifications");
    await expect(page.getByText(/add Slop to your Home Screen/)).toBeVisible();
    await context.close();
  });
});

test.describe("agreements", () => {
  test("up to date, the page says so and links every version", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#agreements");
    const section = page.locator("#agreements");
    await expect(section).toContainText("You're up to date with everything you've agreed to.");
    await expect(section.getByRole("link", { name: "All terms and policies, with every earlier version" })).toHaveAttribute("href", "/legal");
  });

  test("a change to accept opens it; others link to it", async ({ page }) => {
    const publisher = await call<{ version: string }>(null, "GET", "/v1/legal/publisher");
    const privacy = await call<{ version: string }>(null, "GET", "/v1/legal/privacy");
    let accepted = false;
    await fake(page, "GET", "/v1/me/legal", () => ({
      status: 200,
      body: [
        ...(accepted
          ? []
          : [
              {
                kind: "publisher",
                title: "Publisher Agreement",
                version: publisher.version,
                summary: "Clearer rules on refunds.",
                effective_at: "2026-10-01T00:00:00Z",
                in_effect: true,
                first: false,
                action: "accept",
              },
            ]),
        {
          kind: "privacy",
          title: "Privacy Policy",
          version: privacy.version,
          summary: "Names the new backup provider.",
          effective_at: "2026-11-15T00:00:00Z",
          in_effect: false,
          first: false,
          action: "read",
        },
      ],
    }));
    page.on("request", (r) => {
      if (r.url().endsWith("/v1/me/legal/accept") && r.method() === "POST") accepted = true;
    });
    await signIn(page, "support");
    await page.goto("/account#agreements");
    const section = page.locator("#agreements");
    await expect(section.getByRole("listitem").filter({ hasText: "Publisher Agreement" })).toContainText(
      "changed Oct 1, 2026: Clearer rules on refunds.",
    );
    await expect(section.getByRole("listitem").filter({ hasText: "Privacy Policy" })).toContainText(
      "changes Nov 15, 2026: Names the new backup provider.",
    );
    await expect(section.getByRole("link", { name: "Read it" })).toHaveAttribute("href", `/legal/privacy?version=${privacy.version}`);

    await section.getByRole("button", { name: "Review and accept" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Before you continue: the Publisher Agreement" })).toBeVisible();
    const accept = dialog.getByRole("button", { name: "Accept and continue" });
    await expect(accept).toBeDisabled();
    await dialog.getByRole("checkbox", { name: "I've read and accept the Publisher Agreement." }).check();
    await accept.click();
    await expect(dialog).toBeHidden();
    await expect(section.getByRole("button", { name: "Review and accept" })).toHaveCount(0);
    await expect(section.getByRole("listitem").filter({ hasText: "Privacy Policy" })).toBeVisible();
  });

  test("Not now closes the agreement without accepting", async ({ page }) => {
    const terms = await call<{ version: string }>(null, "GET", "/v1/legal/terms");
    await fake(page, "GET", "/v1/me/legal", {
      status: 200,
      body: [
        {
          kind: "terms",
          title: "Terms of Service",
          version: terms.version,
          summary: "",
          effective_at: "2026-10-01T00:00:00Z",
          in_effect: true,
          first: true,
          action: "accept",
        },
      ],
    });
    const accept = await fake(page, "POST", "/v1/me/legal/accept", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto("/account#agreements");
    const section = page.locator("#agreements");
    await expect(section.getByRole("listitem")).toContainText("published Oct 1, 2026");
    await section.getByRole("button", { name: "Review and accept" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Not now" }).click();
    await expect(page.getByRole("dialog")).toBeHidden();
    await expect(section.getByRole("button", { name: "Review and accept" })).toBeVisible();
    expect(accept.calls).toHaveLength(0);
  });
});

test.describe("your data", () => {
  test("Download my data saves everything as slop-data.json", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account#data");
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Download my data" }).click();
    const file = await download;
    expect(file.suggestedFilename()).toBe("slop-data.json");
    const text = await (await file.createReadStream()).toArray();
    const data = JSON.parse(Buffer.concat(text).toString("utf8"));
    expect(JSON.stringify(data)).toContain("Sue Support");
  });

  test("a suspended account can still download its data", async ({ page }) => {
    // Listing Sid's decisions would mark his seeded one seen: leave them out of this page.
    await fake(page, "GET", "/v1/me/decisions", { status: 200, body: [] });
    await signIn(page, "suspended");
    await page.goto("/account#data");
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Download my data" }).click();
    expect((await download).suggestedFilename()).toBe("slop-data.json");
  });

  test("a download that fails says so", async ({ page }) => {
    await fake(page, "GET", "/v1/me/export", { status: 500, body: apiError("internal", "something went wrong") });
    await signIn(page, "support");
    await page.goto("/account#data");
    await page.getByRole("button", { name: "Download my data" }).click();
    await expect(page.locator("#data").getByRole("alert")).toHaveText("Request failed (500)");
  });

  test("closing needs the exact words, and open People jobs and donations stop it", async ({ page }) => {
    // Cleo gives a monthly server donation and has People jobs open: the server refuses.
    await signIn(page, "client");
    await page.goto("/account#data");
    const section = page.locator("#data");
    await section.getByRole("button", { name: "Close my account…" }).click();
    await expect(section.getByRole("heading", { name: "Close your account" })).toBeVisible();
    const close = section.getByRole("button", { name: "Close my account", exact: true });
    const words = section.getByRole("textbox", { name: "Type close my account to confirm" });
    await expect(close).toBeDisabled();
    await words.fill("close my acount");
    await expect(close).toBeDisabled();
    await words.fill("  Close My Account ");
    await expect(close).toBeEnabled();
    await section.getByRole("button", { name: "Keep my account" }).click();
    await expect(section.getByRole("heading", { name: "Close your account" })).toHaveCount(0);

    await section.getByRole("button", { name: "Close my account…" }).click();
    await words.fill("close my account");
    await close.click();
    await expect(section.getByRole("alert")).toContainText("You can't close your account yet: you give a monthly server donation: cancel it first;");
    await expect(section.getByRole("alert")).toContainText("People jobs are still open");
    expect((await call("client", "GET", "/v1/me")).display_name).toBe("Cleo Client");
  });

  test("closing signs out and goes home", async ({ page }) => {
    // Closing for real can't be undone, and every account here is used by other tests.
    const closed = await fake(page, "POST", "/v1/me/close", async () => {
      await page.context().clearCookies();
      return { status: 204, body: undefined };
    });
    await signIn(page, "support");
    await page.goto("/account#data");
    await page.getByRole("button", { name: "Close my account…" }).click();
    await page.getByRole("textbox", { name: "Type close my account to confirm" }).fill("close my account");
    await page.getByRole("button", { name: "Close my account", exact: true }).click();
    await expect(page).toHaveURL(`${server.url}/`);
    await expect(page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Sign in" })).toBeVisible();
    expect(closed.calls[0].body).toEqual({ confirm: "close my account" });
  });
});
