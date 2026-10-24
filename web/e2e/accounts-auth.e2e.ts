// SPDX-License-Identifier: AGPL-3.0-or-later
// Signing in, signing up, approving the desktop app and combining accounts (web/src/pages/Auth.tsx).
import { apiError, call, desktopSignInRequest, fake, noPasskeys, openAccount, passkeyOptions, stubPasskeys, tryCall } from "./accounts-helpers";
import { expect, server, signIn, test } from "./fixtures";

test.describe("the sign-in page", () => {
  test("offers the sign-ins this server has, and keeps where you were going", async ({ page }) => {
    await page.goto("/signin?return_to=%2Flibrary%3Ftab%3Dinstalled");
    await expect(page.getByRole("heading", { name: "Sign in to Slop" })).toBeVisible();
    // The test server has GitHub and nothing else (crates/slop-api/examples/e2e_server/main.rs).
    const github = page.getByRole("link", { name: "Continue with GitHub" });
    await expect(github).toHaveAttribute("href", "/v1/auth/github/start?return_to=%2Flibrary%3Ftab%3Dinstalled");
    await expect(page.getByRole("link", { name: /Continue with (Google|GitLab|Bitbucket|Microsoft|Discord)/ })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Sign in with a passkey" })).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("never sends you to another site afterwards", async ({ page }) => {
    for (const evil of ["//evil.example/x", "/\\evil.example", "https://evil.example/", "javascript:alert(1)"]) {
      await page.goto(`/signin?return_to=${encodeURIComponent(evil)}`);
      await expect(page.getByRole("link", { name: "Continue with GitHub" })).toHaveAttribute(
        "href",
        "/v1/auth/github/start?return_to=%2F",
      );
    }
  });

  test("explains each way a sign-in can fail", async ({ page }) => {
    const messages: Record<string, string> = {
      cancelled: "Sign-in was cancelled.",
      provider: "That sign-in didn't work. Please try again.",
      closed: "That account was closed, so its sign-in can't be used on Slop again.",
      link_expired: "Linking was started in another session. Start it again from your account page.",
      something_new: "Sign-in failed. Please try again.",
    };
    for (const [code, text] of Object.entries(messages)) {
      await page.goto(`/signin?error=${code}`);
      await expect(page.getByRole("alert")).toHaveText(text);
    }
    await page.goto("/signin?reason=link&return_to=%2Faccount");
    await expect(page.getByText("To link another sign-in, first sign in again with one you already use.")).toBeVisible();
  });

  test("says so when no sign-in is set up", async ({ page }) => {
    await fake(page, "GET", "/v1/auth/providers", {
      status: 200,
      body: { github: false, google: false, gitlab: false, bitbucket: false, microsoft: false, discord: false },
    });
    await page.goto("/signin");
    await expect(page.getByText("Sign-in isn't set up on this server yet.")).toBeVisible();
    await expect(page.getByRole("link", { name: /Continue with/ })).toHaveCount(0);
  });

  test("GitHub sign-in goes to GitHub and back, and a failed or cancelled one says so", async ({ page }) => {
    // The fake GitHub can't sign anyone in: the browser lands on its (empty) sign-in page, then
    // comes back to Slop the way GitHub sends it. Slop can't swap the made-up code, or the
    // person said no.
    const toGithub = async () => {
      await page.getByRole("link", { name: "Continue with GitHub" }).click();
      await page.waitForURL(`${server.fake}login/oauth/authorize?**`);
      return new URL(page.url());
    };
    await page.goto("/signin?return_to=%2Flibrary");
    let github = await toGithub();
    expect(github.searchParams.get("code_challenge_method")).toBe("S256");
    expect(github.searchParams.get("redirect_uri")).toBe(`${server.url}/v1/auth/github/callback`);
    const back = (query: string) =>
      page.goto(`${server.url}/v1/auth/github/callback?${query}&state=${encodeURIComponent(github.searchParams.get("state")!)}`);
    await back("code=not-a-real-code");
    await expect(page).toHaveURL(`${server.url}/signin?error=provider`);
    await expect(page.getByRole("alert")).toHaveText("That sign-in didn't work. Please try again.");

    github = await toGithub();
    await back("error=access_denied");
    await expect(page).toHaveURL(`${server.url}/signin?error=cancelled`);
    await expect(page.getByRole("alert")).toHaveText("Sign-in was cancelled.");
  });

  test("signing in with a passkey goes where you were going", async ({ page }) => {
    await stubPasskeys(page);
    const start = await fake(page, "POST", "/v1/auth/passkey/start", { status: 200, body: { challenge_id: "c1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/auth/passkey/finish", async () => {
      // The server's answer sets the session cookie: Pat's, here.
      await signIn(page, "player");
      return { status: 200, body: { signed_in: true } };
    });
    await page.goto("/signin?return_to=%2Flibrary");
    await page.getByRole("button", { name: "Sign in with a passkey" }).click();
    await expect(page).toHaveURL(/\/library$/);
    await expect(page.getByRole("link", { name: "Pat Player" })).toHaveAttribute("href", "/account");
    expect(start.calls).toHaveLength(1);
    expect(finish.calls[0].body).toMatchObject({ challenge_id: "c1", credential: { type: "public-key", id: "AQIDBA" } });
  });

  test("a passkey prompt closed without choosing says what to do", async ({ page }) => {
    await stubPasskeys(page, "cancel");
    await fake(page, "POST", "/v1/auth/passkey/start", { status: 200, body: { challenge_id: "c1", options: passkeyOptions } });
    await page.goto("/signin");
    await page.getByRole("button", { name: "Sign in with a passkey" }).click();
    await expect(page.getByRole("alert")).toHaveText("The passkey wasn't used. Try again, or use another sign-in.");
    await expect(page).toHaveURL(/\/signin$/);
  });

  test("a passkey Slop doesn't know is refused", async ({ page }) => {
    await stubPasskeys(page);
    await fake(page, "POST", "/v1/auth/passkey/start", { status: 200, body: { challenge_id: "c1", options: passkeyOptions } });
    await fake(page, "POST", "/v1/auth/passkey/finish", {
      status: 400,
      body: apiError("invalid", "Slop doesn't know that passkey", "credential"),
    });
    await page.goto("/signin");
    await page.getByRole("button", { name: "Sign in with a passkey" }).click();
    await expect(page.getByRole("alert")).toHaveText("Slop doesn't know that passkey");
  });

  test("browsers without passkeys aren't offered them", async ({ page }) => {
    await noPasskeys(page);
    await page.goto("/signin");
    await expect(page.getByRole("link", { name: "Continue with GitHub" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Sign in with a passkey" })).toHaveCount(0);
  });
});

test.describe("signing up", () => {
  test("without a sign-in in progress, the page says it expired", async ({ page }) => {
    await page.goto("/signup");
    await expect(page.getByRole("heading", { name: "Sign-up expired" })).toBeVisible();
    await page.getByRole("link", { name: "Start again" }).click();
    await expect(page.getByRole("heading", { name: "Sign in to Slop" })).toBeVisible();
  });

  test("asks for a country, the age and the terms, shows what's wrong, then goes on", async ({ page }) => {
    // A sign-up waits only after a provider signs someone new in, which the fake GitHub can't
    // do: the server's half is stood in for, with its real list of countries.
    const countries = await call<Array<{ code: string; name: string; min_age: number }>>(null, "GET", "/v1/countries");
    countries.push({ code: "ZZ", name: "Testland", min_age: 13 });
    await fake(page, "GET", "/v1/auth/signup", {
      status: 200,
      body: { suggested_name: "Gita Newcomer", email: "gita@example.com", countries },
    });
    let attempt = 0;
    const posted = await fake(page, "POST", "/v1/auth/signup", () =>
      ++attempt === 1
        ? { status: 400, body: apiError("invalid", "use 1 to 40 characters, not only spaces", "display_name") }
        : { status: 201, body: { user: {}, return_to: "//evil.example/steal" } },
    );
    await page.goto("/signup");
    await expect(page.getByRole("heading", { name: "Create your Slop account" })).toBeVisible();
    await expect(page.getByText("Signed in with GitHub as gita@example.com.")).toBeVisible();
    const name = page.getByRole("textbox", { name: "Display name" });
    await expect(name).toHaveValue("Gita Newcomer");
    await expect(name).toHaveAttribute("maxlength", "40");

    const create = page.getByRole("button", { name: "Create account" });
    await expect(create).toBeDisabled();
    const country = page.getByRole("combobox", { name: "Country" });
    await expect(country.getByRole("option", { name: "United States" })).toHaveCount(1);
    await country.selectOption("US");
    await expect(page.getByRole("checkbox", { name: "I am 16 or older." })).toBeVisible();
    await country.selectOption("ZZ");
    await page.getByRole("checkbox", { name: "I am 13 or older." }).check();
    await expect(create).toBeDisabled();
    await page.getByRole("checkbox", { name: /I accept the Terms of Service/ }).check();
    await expect(page.getByRole("link", { name: "Terms of Service" })).toHaveAttribute("href", "/legal/terms");
    await expect(page.getByRole("link", { name: "Privacy Policy" })).toHaveAttribute("href", "/legal/privacy");
    await expect(create).toBeEnabled();

    await name.fill("   ");
    await create.click();
    await expect(page.getByRole("alert")).toHaveText("use 1 to 40 characters, not only spaces");
    await name.fill("Gita");
    await create.click();
    // Where the server says to go next, unless it's another site.
    await expect(page).toHaveURL(`${server.url}/`);
    expect(posted.calls.map((c) => c.body)).toEqual([
      { display_name: "   ", country_code: "ZZ", age_confirmed: true, accept_terms: true },
      { display_name: "Gita", country_code: "ZZ", age_confirmed: true, accept_terms: true },
    ]);
  });

  test("goes back to the page you came from", async ({ page }) => {
    const countries = await call(null, "GET", "/v1/countries");
    await fake(page, "GET", "/v1/auth/signup", { status: 200, body: { suggested_name: "Gita", email: null, countries } });
    await fake(page, "POST", "/v1/auth/signup", { status: 201, body: { user: {}, return_to: "/bounties" } });
    await page.goto("/signup");
    await expect(page.getByText(/Signed in with GitHub as/)).toHaveCount(0);
    await page.getByRole("combobox", { name: "Country" }).selectOption("DE");
    await page.getByRole("checkbox", { name: "I am 16 or older." }).check();
    await page.getByRole("checkbox", { name: /I accept the Terms of Service/ }).check();
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page).toHaveURL(/\/bounties$/);
  });
});

test.describe("approving the desktop app", () => {
  test("signed out, the approval page asks you to sign in first and comes back", async ({ page }) => {
    const req = await desktopSignInRequest("Sam's laptop", 41_234);
    await page.goto(`/desktop-signin/${req.id}`);
    await expect(page).toHaveURL(`${server.url}/signin?return_to=${encodeURIComponent(`/desktop-signin/${req.id}`)}`);
    await expect(page.getByRole("link", { name: "Continue with GitHub" })).toHaveAttribute(
      "href",
      `/v1/auth/github/start?return_to=${encodeURIComponent(`/desktop-signin/${req.id}`)}`,
    );
  });

  test("Allow hands the app its code, and the app's session shows on the account page until signed out", async ({ page }) => {
    const port = 41_000 + Math.floor(Math.random() * 1000);
    const req = await desktopSignInRequest("Sue's test PC", port);
    // The app listens on 127.0.0.1:<port>; here the test does.
    let callback: URL | undefined;
    await page.route(`http://127.0.0.1:${port}/**`, async (route) => {
      callback = new URL(route.request().url());
      await route.fulfill({ status: 200, contentType: "text/html", body: "<title>Slop</title><p>You can close this tab.</p>" });
    });
    await signIn(page, "support");
    await page.goto(`/desktop-signin/${req.id}`);
    await expect(page.getByRole("heading", { name: "Sign in to the Slop app?" })).toBeVisible();
    await expect(page.getByText("Sue's test PC wants to sign in as Sue Support.")).toBeVisible();
    await page.getByRole("button", { name: "Allow", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`^http://127\\.0\\.0\\.1:${port}/callback\\?`));
    expect(callback?.searchParams.get("state")).toBe(req.state);

    // The app swaps the code for its own tokens.
    const tokens = await call<{ access_token: string }>(null, "POST", "/v1/auth/desktop/token", {
      grant_type: "authorization_code",
      code: callback!.searchParams.get("code"),
      code_verifier: req.verifier,
    });
    const asApp = () => fetch(`${server.url}/v1/me`, { headers: { Authorization: `Bearer ${tokens.access_token}` } });
    expect((await asApp()).status).toBe(200);

    // A used request can't be approved again.
    await page.goto(`/desktop-signin/${req.id}`);
    await expect(page.getByRole("heading", { name: "This sign-in link has expired" })).toBeVisible();

    // The account page lists it by the app's name, and signs it out.
    await openAccount(page, "security");
    const sessions = page.locator("ul.sessions").last();
    const app = sessions.getByRole("listitem").filter({ hasText: "Sue's test PC" });
    await expect(app).toBeVisible();
    const here = sessions.getByRole("listitem").filter({ hasText: "THIS DEVICE" });
    // The seed signed Sue in from "e2e", which names no browser.
    await expect(here.locator("b")).toHaveText("Browser");
    await expect(here.getByRole("button", { name: "Sign out" })).toHaveCount(0);
    await app.getByRole("button", { name: "Sign out" }).click();
    await expect(app).toHaveCount(0);
    expect((await asApp()).status).toBe(401);
    await expect(page.getByRole("heading", { name: "Your account" })).toBeVisible();
  });

  test("Don't allow leaves the app signed out", async ({ page }) => {
    const req = await desktopSignInRequest("Someone's PC", 42_345);
    await signIn(page, "support");
    await page.goto(`/desktop-signin/${req.id}`);
    await page.getByRole("button", { name: "Don't allow" }).click();
    await expect(page.getByRole("heading", { name: "Not signed in" })).toBeVisible();
    await expect(page.getByText("The app wasn't given access. You can close this tab.")).toBeVisible();
    // Nothing was approved: the request still waits (and runs out on its own).
    expect((await tryCall("support", "GET", `/v1/auth/desktop/requests/${req.id}`)).status).toBe(200);
  });

  test("an expired or unknown request says so", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/desktop-signin/01a00000-0000-7000-8000-000000000000");
    await expect(page.getByRole("heading", { name: "This sign-in link has expired" })).toBeVisible();
    await expect(page.getByText("Go back to the Slop app and choose Sign in again.")).toBeVisible();
  });

  test("an old sign-in must be confirmed before it can approve a new device", async ({ page }) => {
    const req = await desktopSignInRequest("Sue's other PC", 43_456);
    await stubPasskeys(page);
    // The seeded session is fresh; an old one is refused like this (auth/session.rs).
    const approve = await fake(
      page,
      "POST",
      `/v1/auth/desktop/requests/${req.id}/approve`,
      { status: 403, body: apiError("step_up_required", "this needs a recent sign-in") },
      { times: 1 },
    );
    await fake(page, "POST", "/v1/me/step-up/start", { status: 200, body: { challenge_id: "s1", options: passkeyOptions } });
    const finish = await fake(page, "POST", "/v1/me/step-up/finish", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto(`/desktop-signin/${req.id}`);
    await page.getByRole("button", { name: "Allow", exact: true }).click();
    const alert = page.getByRole("alert");
    await expect(alert).toContainText("For your safety, this needs a sign-in from the last few minutes.");
    await expect(alert.getByRole("link", { name: "sign in again" })).toHaveAttribute(
      "href",
      `/signin?return_to=${encodeURIComponent(`/desktop-signin/${req.id}`)}`,
    );
    await alert.getByRole("button", { name: "Confirm with a passkey" }).click();
    await expect(page.getByRole("alert")).toHaveCount(0);
    expect(finish.calls).toHaveLength(1);
    expect(approve.calls).toHaveLength(1);
    // Confirmed: Allow works now (the server answers this time).
    await page.route("http://127.0.0.1:43456/**", (route) => route.fulfill({ status: 200, body: "ok" }));
    await page.getByRole("button", { name: "Allow", exact: true }).click();
    await expect(page).toHaveURL(/^http:\/\/127\.0\.0\.1:43456\/callback\?/);
  });
});

test.describe("combining accounts", () => {
  // A combination waits only after linking a sign-in that has its own Slop account, which takes
  // a real provider: the waiting combination is stood in for; the rest is the real page.
  const preview = {
    provider: "github",
    name: "Old Sue",
    apps: 1,
    library: 3,
    votes: 1,
    friends: 0,
    teams: 2,
    jobs: 0,
    chats: 0,
    blocked: null as string | null,
  };

  test("lists what moves, and combining goes back to the account page", async ({ page }) => {
    await fake(page, "GET", "/v1/auth/combine", { status: 200, body: preview });
    const confirm = await fake(page, "POST", "/v1/auth/combine", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto("/account/combine");
    await expect(page.getByRole("heading", { name: "Combine accounts?" })).toBeVisible();
    await expect(page.getByText("That GitHub sign-in already has its own Slop account, Old Sue.")).toBeVisible();
    const list = page.getByRole("list").filter({ hasText: "its sign-ins, reports" });
    await expect(list.getByRole("listitem")).toHaveText([
      "1 app",
      "3 games in the library",
      "1 vote",
      "2 teams",
      "its sign-ins, reports, donations and everything else",
    ]);
    await page.getByRole("button", { name: "Combine them" }).click();
    await expect(page).toHaveURL(/\/account\/security\?linked=github#sign-ins$/);
    await expect(page.getByRole("status").filter({ hasText: "GitHub is linked" })).toBeVisible();
    expect(confirm.calls).toHaveLength(1);
  });

  test("a combination that can't happen says why and can't be confirmed", async ({ page }) => {
    await fake(page, "GET", "/v1/auth/combine", {
      status: 200,
      body: { ...preview, blocked: "that account has a People job in progress" },
    });
    const cancel = await fake(page, "DELETE", "/v1/auth/combine", { status: 204, body: undefined });
    await signIn(page, "support");
    await page.goto("/account/combine");
    await expect(page.getByRole("alert")).toHaveText(
      "These accounts can't be combined: that account has a People job in progress.",
    );
    await expect(page.getByRole("button", { name: "Combine them" })).toBeDisabled();
    await page.getByRole("button", { name: "Keep them separate" }).click();
    await expect(page).toHaveURL(/\/account\/security#sign-ins$/);
    expect(cancel.calls).toHaveLength(1);
  });

  test("a refused combination shows the server's reason", async ({ page }) => {
    await fake(page, "GET", "/v1/auth/combine", { status: 200, body: preview });
    await fake(page, "POST", "/v1/auth/combine", { status: 403, body: apiError("step_up_required", "this needs a recent sign-in") });
    await signIn(page, "support");
    await page.goto("/account/combine");
    await page.getByRole("button", { name: "Combine them" }).click();
    await expect(page.getByRole("alert")).toHaveText("this needs a recent sign-in");
    await expect(page).toHaveURL(/\/account\/combine$/);
  });

  test("with nothing waiting (or it expired), there's nothing to combine", async ({ page }) => {
    await signIn(page, "support");
    await page.goto("/account/combine");
    await expect(page.getByText("Nothing to combine (it may have expired).")).toBeVisible();
    await page.getByRole("link", { name: "Back to your account" }).click();
    await expect(page).toHaveURL(/\/account\/security#sign-ins$/);
  });
});
