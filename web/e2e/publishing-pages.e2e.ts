// SPDX-License-Identifier: AGPL-3.0-or-later
// Publishing, part 5: registering a branch (a port), signing keys, the publishing guide and the
// download page.
import { expect, signIn, test } from "./fixtures";
import { answer, apiError, readNotices, uniq } from "./publishing-helpers";

test("a branch shows its errors by their fields", async ({ page }) => {
  // Fin publishes nothing here: the attempt is refused.
  await signIn(page, "finance");
  await page.goto("/app/neon-drift/branch");
  await expect(page.getByRole("heading", { name: "Register a branch of Neon Drift" })).toBeVisible();
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Neon Drift Linux");
  await page.getByRole("textbox", { name: /^Address/ }).fill(uniq("neon-port"));
  await page.getByRole("textbox", { name: "One-line summary" }).fill("Neon Drift on Linux.");
  await page.getByRole("textbox", { name: /What it changes from Neon Drift/ }).fill("Builds for Linux.");
  // The original's own repository isn't a fork.
  await page.getByRole("textbox", { name: "Your fork" }).fill("https://github.com/e2e/neon-drift");
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  await page.getByRole("group", { name: "Runs on" }).getByRole("checkbox", { name: /Linux/ }).check();
  await page.getByRole("button", { name: "Register the branch" }).click();
  await expect(page.locator("label").filter({ hasText: "Your fork" }).getByRole("alert")).toHaveText(
    "a branch or mod builds from your own repository, not the original one",
  );
});

test("Bo registers a port of Neon Drift, which starts from the original's answers", async ({ page }) => {
  const slug = uniq("pub-neon-port");
  await signIn(page, "maker2");
  await page.goto("/app/neon-drift/branch");
  await expect(page.getByRole("heading", { name: "Register a branch of Neon Drift" })).toBeVisible();
  await expect(page.getByText(/Neon Drift doesn't run on linux yet: a port fills that gap\./)).toBeVisible();
  await expect(page.getByRole("radio", { name: /^Port/ })).toBeChecked();
  // Neon Drift plays online: its answers are ticked to start with.
  const online = page.getByRole("group", { name: "Online", exact: true });
  await expect(online.getByRole("button", { name: "Players can host their own servers" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText(/Ticked from Neon Drift's page: it plays online\./)).toBeVisible();
  // Rated at least as the original.
  await expect(page.getByLabel("Age rating").locator("option")).toHaveCount(6);

  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Neon Drift for Linux");
  await page.getByRole("textbox", { name: /^Address/ }).fill(slug);
  await page.getByRole("textbox", { name: "One-line summary" }).fill("Neon Drift, built for Linux.");
  await page.getByRole("textbox", { name: /What it changes from Neon Drift/ }).fill("A native Linux build.");
  await page.getByRole("textbox", { name: "Your fork" }).fill(`https://github.com/bo/${slug}`);
  await page.getByRole("combobox", { name: "Licence" }).selectOption("MIT");
  const platforms = page.getByRole("group", { name: "Runs on" });
  await platforms.getByRole("checkbox", { name: /Linux/ }).check();
  await expect(platforms.getByRole("checkbox", { name: /\.deb/ })).toBeChecked();
  await page.getByRole("button", { name: "Register the branch" }).click();
  // A draft: next, a release.
  await expect(page).toHaveURL(new RegExp(`/publish/${slug}/releases$`));
  await expect(page.getByText("Linux (a .tar.gz of the app's files), Linux .deb (a .deb package)")).toBeVisible();
  await page.goto(`/app/${slug}`);
  await expect(page.locator(".community-notice")).toContainText("A port of Neon Drift, made by Bo Builder, not Sam Seed. A native Linux build.");
  await readNotices("maker2", slug);
});

test("signing keys: add one (it works 72 hours later), refuse a bad one, remove it", async ({ page }) => {
  // A key works only 72 hours after it's added, which the test server can't wait for: the API's
  // answers are played here, the real ones in the API tests.
  type Key = { id: string; name: string; fingerprint: string; created_at: string; active_from: string; active: boolean };
  const keys: Key[] = [
    {
      id: "k-old",
      name: "Old laptop",
      fingerprint: "SHA256:old0old0old0old0old0old0old0old0old0old0old",
      created_at: "2026-09-01T10:00:00Z",
      active_from: "2026-09-04T10:00:00Z",
      active: true,
    },
  ];
  let added: unknown;
  await page.route("**/v1/me/signing-keys", async (route) => {
    const req = route.request();
    if (req.method() === "GET") return route.fulfill({ json: keys });
    const body = req.postDataJSON() as { name: string; public_key: string };
    if (!body.public_key.startsWith("ssh-ed25519 ")) {
      return route.fulfill({
        status: 422,
        json: apiError("invalid", "paste an Ed25519 public key (the line in id_ed25519.pub, starting with ssh-ed25519)", "public_key"),
      });
    }
    added = body;
    const key = {
      id: "k-new",
      name: body.name,
      fingerprint: "SHA256:new0new0new0new0new0new0new0new0new0new0new",
      created_at: "2026-10-23T10:00:00Z",
      active_from: "2026-10-26T10:00:00Z",
      active: false,
    };
    keys.push(key);
    return route.fulfill({ status: 201, json: key });
  });
  await page.route("**/v1/me/signing-keys/*", (route) => {
    const id = route.request().url().split("/").pop();
    keys.splice(keys.findIndex((k) => k.id === id), 1);
    return route.fulfill({ status: 204 });
  });

  await signIn(page, "maker");
  await page.goto("/account#signing-keys");
  const section = page.locator("#signing-keys");
  await expect(section.getByRole("heading", { name: "Signing keys" })).toBeVisible();
  await expect(section.getByRole("listitem").filter({ hasText: "Old laptop" })).toContainText(/works since Sep 4, 2026/);
  const add = section.getByRole("button", { name: "Add signing key" });
  await expect(add).toBeDisabled();
  await section.getByRole("textbox", { name: "Name" }).fill("Build box");
  await section.getByRole("textbox", { name: /Public key/ }).fill("ssh-rsa AAAAB3NzaC1yc2E build@box");

  // An RSA key is refused (and nothing asks to confirm it's you first).
  await add.click();
  const alert = section.getByRole("alert");
  await expect(alert).toHaveText("paste an Ed25519 public key (the line in id_ed25519.pub, starting with ssh-ed25519)");
  await section.getByRole("textbox", { name: /Public key/ }).fill("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHt6 build@box");
  await add.click();
  const row = section.getByRole("listitem").filter({ hasText: "Build box" });
  await expect(row).toContainText("works from Oct 26, 2026");
  await expect(row).toContainText("SHA256:new0");
  expect(added).toEqual({ name: "Build box", public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHt6 build@box" });
  await expect(section.getByRole("textbox", { name: "Name" })).toHaveValue("");
  await expect(alert).toHaveCount(0);

  await row.getByRole("button", { name: "Remove" }).click();
  await expect(row).toHaveCount(0);
  await expect(section.getByRole("listitem").filter({ hasText: "Old laptop" })).toBeVisible();
});

test("the publishing guide: an AI prompt to copy, and a message when it can't load", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/publish/guide");
  await expect(page.getByRole("heading", { name: "Publishing on SLAPorSLOP", level: 1 })).toBeVisible();
  const notice = page.locator(".ai-notice");
  await notice.getByRole("button", { name: "Copy" }).click();
  await expect(notice.getByRole("button", { name: "Copied" })).toBeVisible();
  const origin = new URL(page.url()).origin;
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    `Read ${origin}/publishing.md. Tell me what's required and what's optional for publishing on SLAPorSLOP, then set up my project so it's ready to deploy there.`,
  );
  await expect(notice.getByRole("link", { name: "/publishing.md" })).toHaveAttribute("href", "/publishing.md");

  await page.route("**/publishing.md", (route) => route.fulfill({ status: 503, body: "down" }));
  await page.reload();
  await expect(page.getByText("Couldn't load the guide.")).toBeVisible();
  // The prompt is still there to copy.
  await expect(notice.getByRole("button", { name: "Copy" })).toBeVisible();
});

test("the download page: a button per system when there's a build, and says when there isn't", async ({ page }) => {
  // This server has published no desktop builds.
  await page.goto("/download");
  await expect(page.getByRole("heading", { name: "Get the SLAPorSLOP app", level: 1 })).toBeVisible();
  for (const name of ["Windows", "Ubuntu / Debian", "RHEL / Rocky / Alma 10"]) {
    await expect(page.locator("section.download-card").filter({ has: page.getByRole("heading", { name }) })).toContainText(
      "Not available right now.",
    );
  }
  await expect(page.getByRole("link", { name: "Download", exact: true })).toHaveCount(0);

  await answer(page, "/v1/desktop/latest/windows", { version: "1.4.0", url: "https://downloads.example/slop-1.4.0-setup.exe" });
  await answer(page, "/v1/desktop/latest/linux-deb", { version: "1.4.0", url: "https://downloads.example/slop_1.4.0_amd64.deb" });
  await page.reload();
  // The browser here says it's on Windows.
  const windows = page.locator("section.download-card").filter({ has: page.getByRole("heading", { name: "Windows" }) });
  await expect(windows.getByText("YOUR SYSTEM")).toBeVisible();
  await expect(windows.getByRole("link", { name: "Download" })).toHaveAttribute("href", "https://downloads.example/slop-1.4.0-setup.exe");
  await expect(windows).toContainText("Version 1.4.0");
  const deb = page.locator("section.download-card").filter({ has: page.getByRole("heading", { name: "Ubuntu / Debian" }) });
  await expect(deb.getByRole("link", { name: "Download" })).toHaveAttribute("href", "https://downloads.example/slop_1.4.0_amd64.deb");
  await expect(deb.getByText("YOUR SYSTEM")).toHaveCount(0);
  const rpm = page.locator("section.download-card").filter({ has: page.getByRole("heading", { name: "RHEL / Rocky / Alma 10" }) });
  await expect(rpm).toContainText("Not available right now.");
  await expect(rpm.locator("code")).toHaveText("sudo dnf install epel-release && sudo dnf config-manager --set-enabled crb");
});
