import { beforeEach, describe, expect, it, vi } from "vitest";

const installed = [
  { slug: "old", title: "Old", maker: "M", version: "1.0", release_id: "r1", kind: "archive", installed_at: "" },
  { slug: "playing", title: "Playing", maker: "M", version: "1.0", release_id: "p1", kind: "archive", installed_at: "" },
  { slug: "setup", title: "Setup", maker: "M", version: "1.0", release_id: "s1", kind: "installer", installed_at: "" },
  { slug: "current", title: "Current", maker: "M", version: "2.0", release_id: "c2", kind: "archive", installed_at: "" },
];
const install = vi.fn(() => Promise.resolve());

vi.mock("../platform", () => ({
  isDesktop: true,
  desktop: {
    installed: () => Promise.resolve(installed),
    running: () => Promise.resolve(["playing"]),
    install: (slug: string) => install(slug),
  },
}));
vi.mock("../api", () => ({
  api: {
    libraryUpdates: () =>
      Promise.resolve(
        ["old", "playing", "setup", "current"].map((slug) => ({
          slug,
          latest: { release_id: slug === "current" ? "c2" : `${slug}-new`, version: "2.0" },
          pending: null,
          builds: [],
        })),
      ),
  },
}));

import { checkUpdates, installUpdates } from "./AppUpdates";

describe("app updates", () => {
  beforeEach(() => install.mockClear());

  it("marks an update available when the release players get differs from the installed one", async () => {
    const apps = await checkUpdates();
    expect(apps.filter((a) => a.available).map((a) => a.installed.slug)).toEqual(["old", "playing", "setup"]);
  });

  it("never updates a running app or one with its own installer", async () => {
    expect(await installUpdates()).toBe(1);
    expect(install).toHaveBeenCalledTimes(1);
    expect(install).toHaveBeenCalledWith("old");
  });
});
