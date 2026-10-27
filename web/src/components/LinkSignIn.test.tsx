// SPDX-License-Identifier: AGPL-3.0-or-later
// Linking a sign-in from the website and from the desktop app, whose pages are its own (a server
// address followed inside the app showed its "Page not found").
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { LinkSignIn } from "./LinkSignIn";

const platform = vi.hoisted(() => ({ desktop: false, opened: [] as string[] }));
vi.mock("../platform", () => ({
  get isDesktop() {
    return platform.desktop;
  },
  openExternal: async (url: string) => {
    platform.opened.push(url);
  },
}));

afterEach(() => {
  cleanup();
  platform.desktop = false;
  platform.opened = [];
});

describe("LinkSignIn", () => {
  it("on the website, is a link to the server, which comes back to the page", () => {
    render(
      <LinkSignIn provider="google" returnTo="/account?picture=refresh#picture">
        Refresh picture
      </LinkSignIn>,
    );
    expect(screen.getByRole("link", { name: "Refresh picture" }).getAttribute("href")).toBe(
      "/v1/auth/google/link?return_to=%2Faccount%3Fpicture%3Drefresh%23picture",
    );
  });

  it("in the desktop app, opens the site in the browser and reloads when the app is back in front", () => {
    platform.desktop = true;
    const onReturn = vi.fn();
    render(
      <LinkSignIn provider="discord" returnTo="/account?picture=refresh#picture" onReturn={onReturn}>
        Refresh picture
      </LinkSignIn>,
    );
    expect(screen.queryByRole("link")).toBeNull();
    // Not yet opened: coming back to the app does nothing.
    act(() => void window.dispatchEvent(new Event("focus")));
    expect(onReturn).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Refresh picture" }));
    expect(platform.opened).toEqual([
      `${window.location.origin}/v1/auth/discord/link?return_to=%2Faccount%3Fpicture%3Drefresh%23picture`,
    ]);
    expect(screen.getByRole("status").textContent).toBe("Opened in your browser: finish there, then come back.");
    act(() => void window.dispatchEvent(new Event("focus")));
    expect(onReturn).toHaveBeenCalledTimes(1);
  });
});
