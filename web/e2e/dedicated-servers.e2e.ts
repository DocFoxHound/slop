// SPDX-License-Identifier: AGPL-3.0-or-later
// Dedicated servers linked to their games (docs/DEDICATED-SERVERS.md; components/DedicatedServers.tsx):
// a game's maker adds one on its Edit store page, from its own repository; it's shown on the game's
// store page (its own address opens the game's), with the file to download and its SHA-256 for a
// host without the SLAPorSLOP app; getting the game gets the client only, with an unticked box for
// the server; the library lists the server under its game; uninstalling the game asks about it.
// The apps are Dee Designer's, who publishes nothing else in these tests, and leave the store when
// the file is done.
import type { Page } from "@playwright/test";
import { expect, server, signIn, test } from "./fixtures";
import { asDesktop } from "./desktop-helpers";
import { call, delist, list, listedApp, released, tryCall } from "./store-helpers";

const GAME = "siege-castle";
const SERVER = "siege-castle-server";

test.beforeAll(async () => {
  await listedApp(GAME, "Siege Castle", { who: "designer", tags: ["multiplayer"] });
  if ((await tryCall("designer", "GET", `/v1/apps/${SERVER}`)).status !== 200) {
    await call("designer", "POST", `/v1/apps/${GAME}/servers`, {
      slug: SERVER,
      title: "Siege Castle Dedicated server",
      summary: "Hosts a castle for up to 16 players.",
      repo_url: `${server.fake}bo/${SERVER}`,
      license_spdx: "MIT",
      platforms: ["windows"],
    });
  }
  await released(SERVER, "designer");
  const status = (await call("designer", "GET", `/v1/apps/${SERVER}`)).status;
  if (status !== undefined && status !== "listed") await list(SERVER);
});

test.afterAll(async () => {
  for (const slug of [GAME, SERVER]) await tryCall("player", "DELETE", `/v1/me/library/${slug}`);
  await delist(SERVER, GAME);
});

const section = (page: Page) => page.locator("section#servers");

test("the game's page shows its dedicated server, with the file and its SHA-256 to check it by hand", async ({ page }) => {
  await page.goto(`/app/${GAME}`);
  const servers = section(page);
  await expect(servers.getByRole("heading", { name: "Dedicated server", exact: true })).toBeVisible();
  await expect(servers.getByText("Run your own server on your computer or a host, so others can play.")).toBeVisible();
  await expect(servers.getByText("Siege Castle Dedicated server")).toBeVisible();
  await expect(servers.getByText("Hosts a castle for up to 16 players.")).toBeVisible();
  await expect(servers.getByText(/^Windows · version 1\.0\.0 · \d/)).toBeVisible();
  // Not a community build of the game.
  await expect(page.locator(".community-builds")).not.toContainText("Dedicated server");

  await servers.getByText("Download it yourself").click();
  await expect(servers.getByText(/Check the file's SHA-256 before you run it/)).toBeVisible();
  await expect(servers.getByRole("button", { name: /^Download for Windows/ })).toBeVisible();
  await expect(servers.getByText(new RegExp(`SHA-256 of ${SERVER}\\.zip: [0-9a-f]{64}`))).toBeVisible();
  await expect(servers.getByRole("button", { name: "Report" })).toBeVisible();
});

test("a server's own address opens its game's page, and it isn't in the store on its own", async ({ page }) => {
  await page.goto(`/app/${SERVER}`);
  await expect(page).toHaveURL(new RegExp(`/app/${GAME}$`));
  await expect(page.getByRole("heading", { name: "Siege Castle", level: 1 })).toBeVisible();
  const search = await call<{ apps: Array<{ slug: string }> }>(null, "GET", `/v1/apps?q=${encodeURIComponent("Siege Castle")}`);
  expect(search.apps.map((a) => a.slug)).toEqual([GAME]);
  // The Servers category lists the game.
  const servers = await call<{ apps: Array<{ slug: string }> }>(null, "GET", "/v1/apps?category=servers&limit=100");
  expect(servers.apps.map((a) => a.slug)).toContain(GAME);
  expect(servers.apps.map((a) => a.slug)).not.toContain(SERVER);
});

test("adding the game to the library adds the client only, unless the server is ticked too", async ({ page }) => {
  await signIn(page, "player");
  await page.goto(`/app/${GAME}`);
  const box = page.locator(".install-box");
  const also = box.getByRole("checkbox", { name: "Also get the dedicated server" });
  await expect(also).not.toBeChecked();
  await also.check();
  await box.getByRole("button", { name: "+ Add to library" }).click();
  await expect(box.getByRole("button", { name: "✓ In your library" })).toBeVisible();
  const library = await call<Array<{ slug: string; server_for?: string }>>("player", "GET", "/v1/me/library");
  expect(library.find((e) => e.slug === SERVER)?.server_for).toBe(GAME);
  expect(library.some((e) => e.slug === GAME)).toBe(true);

  // In the library, the server is right under its game, and the game has a tab for it.
  await page.goto("/library");
  const rows = page.locator(".lib-row");
  await expect(rows.filter({ hasText: "Siege Castle Dedicated server" })).toBeVisible();
  const titles = await rows.allTextContents();
  const at = titles.findIndex((t) => t.includes("Siege Castle") && !t.includes("Dedicated"));
  expect(titles[at + 1]).toContain("Siege Castle Dedicated server");
  await expect(rows.nth(at + 1)).toHaveClass(/nested/);
  await rows.nth(at).click();
  await page.getByRole("tab", { name: "Dedicated server" }).click();
  await expect(page.locator("section#servers").getByText("Siege Castle Dedicated server")).toBeVisible();
});

test("on the desktop app, Install can bring the server, and uninstalling the game asks about it", async ({ page }) => {
  await asDesktop(page, { state: "none" });
  await signIn(page, "player");
  await page.goto(`/app/${GAME}`);
  const box = page.locator(".install-box");
  await box.getByRole("checkbox", { name: "Also get the dedicated server" }).check();
  await box.getByRole("button", { name: "Install", exact: true }).click();
  await expect(box.getByRole("button", { name: "▶ Play" })).toBeVisible();
  const installs = await page.evaluate(() =>
    (window as unknown as { __desktopCalls: Array<{ cmd: string; args: { slug?: string } }> }).__desktopCalls
      .filter((c) => c.cmd === "install_app")
      .map((c) => c.args.slug),
  );
  expect(installs).toEqual([GAME, SERVER]);

  await box.getByRole("button", { name: "Uninstall", exact: true }).click();
  const remove = box.getByRole("checkbox", { name: "Uninstall its dedicated server too" });
  await expect(remove).not.toBeChecked();
  // The confirm's own Uninstall, after the one that opened it.
  await box.locator(".notice").getByRole("button", { name: "Uninstall", exact: true }).click();
  await expect(box.getByRole("button", { name: "Install", exact: true })).toBeVisible();
  const uninstalls = await page.evaluate(() =>
    (window as unknown as { __desktopCalls: Array<{ cmd: string; args: { slug?: string } }> }).__desktopCalls
      .filter((c) => c.cmd === "uninstall_app")
      .map((c) => c.args.slug),
  );
  expect(uninstalls).toEqual([GAME]);
});

test("the maker adds a dedicated server on the game's Edit store page, from its own repository", async ({ page }) => {
  await signIn(page, "designer");
  await page.goto(`/publish/${GAME}`);
  const panel = page.locator("section.servers-panel");
  await expect(panel.getByRole("heading", { name: "Dedicated server (optional)" })).toBeVisible();
  await expect(panel.getByText(/A program players run on their own computer or a host so others can play your game/)).toBeVisible();
  await expect(panel.getByText("Siege Castle Dedicated server")).toBeVisible();
  await expect(panel.getByRole("link", { name: "Releases & progress" })).toHaveAttribute("href", `/publish/${SERVER}/releases`);

  await panel.getByRole("button", { name: "Add another dedicated server" }).click();
  const form = panel.getByRole("form", { name: "Add a dedicated server" });
  await expect(form.getByLabel("Name")).toHaveValue("Siege Castle Dedicated server");
  await expect(form.getByLabel("Address")).toHaveValue(`${GAME}-server-2`);
  // The game's own repository is refused: a server lives in its own.
  await form.getByLabel("Its repository").fill(`${server.fake}bo/${GAME}`);
  await form.getByLabel("Name").fill("Siege Castle Lobby server");
  await form.getByRole("button", { name: "Add the server" }).click();
  await expect(form.getByText("a dedicated server lives in its own repository, not the game's")).toBeVisible();
});

test("a dedicated server's Manage page has no History: nothing is counted for one", async ({ page }) => {
  await signIn(page, "designer");
  await page.goto(`/publish/${SERVER}/manage`);
  await expect(page.getByRole("heading", { name: "Bug reports", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "History", exact: true })).toHaveCount(0);
  // Its game has one.
  await page.goto(`/publish/${GAME}/manage`);
  await expect(page.getByRole("region", { name: "History", exact: true })).toBeVisible();
});

test("only the game's maker can add a dedicated server to it", async () => {
  const res = await tryCall("player", "POST", `/v1/apps/${GAME}/servers`, {
    slug: "not-my-server",
    title: "Not my server",
    summary: "A server.",
    repo_url: `${server.fake}bo/not-my-server`,
    license_spdx: "MIT",
    platforms: ["windows"],
  });
  expect(res.status).toBe(422);
  expect(res.body.error.field).toBe("kind");
});
