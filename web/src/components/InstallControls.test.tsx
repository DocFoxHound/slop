// SPDX-License-Identifier: AGPL-3.0-or-later
// Play for games that play in the browser (InstallControls' BrowserControls), on the website and
// in the desktop app: a real link, the launch recorded alongside (website) or first (desktop),
// and a failed launch never stopping the game.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ desktop: false, me: { id: "pat" } as { id: string } | null }));

vi.mock("../platform", async (actual) => ({
  ...(await actual<typeof import("../platform")>()),
  get isDesktop() {
    return state.desktop;
  },
  openExternal: vi.fn(async () => {}),
  desktop: {
    installed: vi.fn(async () => []),
    install: vi.fn(),
    launch: vi.fn(),
  },
}));
vi.mock("../session", () => ({ useSession: () => ({ me: state.me }) }));
vi.mock("../api", async (actual) => {
  const real = await actual<typeof import("../api")>();
  return {
    ...real,
    api: {
      launch: vi.fn(),
      addToLibrary: vi.fn(async () => {}),
      removeFromLibrary: vi.fn(async () => {}),
      hasBuild: vi.fn(),
    },
  };
});

import { api, ApiError, type Launched } from "../api";
import { desktop, openExternal } from "../platform";
import { APP_LAUNCHED, PLAY_STOPPED } from "./AppUpdates";
import { BrowserPlayLink, InstallControls, type BrowserPlay } from "./InstallControls";

const ADDRESS = "https://play.example.com/orbit/";
const launched = (play_address: string | null = ADDRESS): Launched => ({ token: null, pulse_url: "/v1/pulse", heartbeat_seconds: 60, play_address });

function controls(play: BrowserPlay = { address: ADDRESS }, bar = false, inLibrary = false) {
  return render(
    <MemoryRouter initialEntries={["/app/orbit"]}>
      <InstallControls slug="orbit" title="Orbit" inLibrary={inLibrary} browser={play} bar={bar ? { extra: null } : undefined} />
    </MemoryRouter>,
  );
}

const play = () => screen.getByRole("link", { name: "Play" });

/** Lets the click's promises settle. */
const settle = () => act(async () => {});

describe("Play on a browser game", () => {
  let launches: string[];
  let stopped: string[];
  const onLaunch = (e: Event) => launches.push((e as CustomEvent<string>).detail);
  const onStopped = (e: Event) => stopped.push((e as CustomEvent<string>).detail);

  beforeEach(() => {
    state.desktop = false;
    state.me = { id: "pat" };
    launches = [];
    stopped = [];
    window.addEventListener(APP_LAUNCHED, onLaunch);
    window.addEventListener(PLAY_STOPPED, onStopped);
    vi.mocked(api.launch).mockReset().mockResolvedValue(launched());
    vi.mocked(openExternal).mockClear();
    vi.mocked(api.addToLibrary).mockClear();
  });
  afterEach(() => {
    window.removeEventListener(APP_LAUNCHED, onLaunch);
    window.removeEventListener(PLAY_STOPPED, onStopped);
    cleanup();
    vi.useRealTimers();
  });

  it("on the website, is a link that opens a new tab by itself, with nothing passed to the site", () => {
    controls();
    const link = play();
    expect(link.getAttribute("href")).toBe(ADDRESS);
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    const hint = document.getElementById(link.getAttribute("aria-describedby")!);
    expect(hint?.textContent).toBe("Opens play.example.com in a new tab. Nothing to download or install.");
    // Nothing to install: no Install, no settings, no sandbox switch.
    expect(screen.queryByRole("button", { name: /Install|Settings/ })).toBeNull();
    expect(desktop.installed).not.toHaveBeenCalled();
    expect(api.hasBuild).not.toHaveBeenCalled();
  });

  it("on the website, records the play without holding up the tab, then tells the page", async () => {
    let finish: (l: Launched) => void = () => {};
    vi.mocked(api.launch).mockReturnValue(new Promise((done) => (finish = done)));
    controls();
    // The link isn't stopped: the browser opens the tab inside the click, so no popup blocker stops it.
    expect(fireEvent.click(play())).toBe(true);
    expect(api.launch).toHaveBeenCalledWith("orbit");
    expect(openExternal).not.toHaveBeenCalled();
    expect(launches).toEqual([]);
    await act(async () => finish(launched()));
    expect(launches).toEqual(["orbit"]);
  });

  it("on the website, a middle click records it too; signed out, nothing is recorded", async () => {
    controls();
    fireEvent(play(), new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
    expect(api.launch).toHaveBeenCalledTimes(1);
    cleanup();

    state.me = null;
    controls();
    fireEvent.click(play());
    await settle();
    expect(api.launch).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("link", { name: "Sign in to add to your library" }).getAttribute("href")).toBe("/signin?return_to=%2Fapp%2Forbit");
  });

  it("when the play isn't recorded, says so, and the game is open anyway", async () => {
    vi.mocked(api.launch).mockRejectedValue(new ApiError(0, "network", "Can't reach SLAPorSLOP."));
    controls();
    fireEvent.click(play());
    await settle();
    expect(screen.getByRole("status").textContent).toMatch(/^SLAPorSLOP couldn't count this play.*The game opened anyway\.$/);
    expect(launches).toEqual([]);
  });

  it("on the website, an address withdrawn since the page loaded: the tab is open, so it says to close it, and the page loads again", async () => {
    vi.mocked(api.launch).mockRejectedValue(new ApiError(409, "conflict", "nothing to play yet"));
    controls();
    expect(fireEvent.click(play())).toBe(true);
    await settle();
    expect(screen.getByRole("status").textContent).toBe("SLAPorSLOP no longer opens this game: its address was withdrawn. Close that tab.");
    expect(screen.queryByText(/couldn't count this play/)).toBeNull();
    expect(stopped).toEqual(["orbit"]);
    expect(launches).toEqual([]);
    cleanup();
    // Gone from the store since: the same, without the address.
    vi.mocked(api.launch).mockRejectedValue(new ApiError(404, "not_found", "not found"));
    controls();
    fireEvent.click(play());
    await settle();
    expect(screen.getByRole("status").textContent).toBe("SLAPorSLOP no longer opens this game. Close that tab.");
  });

  it("the rating's Play keeps what SLAPorSLOP said once the page drops the address", async () => {
    vi.mocked(api.launch).mockRejectedValue(new ApiError(409, "conflict", "nothing to play yet"));
    const view = (address: string | null) => (
      <MemoryRouter>
        <BrowserPlayLink slug="orbit" address={address} />
      </MemoryRouter>
    );
    const { rerender } = render(view(ADDRESS));
    fireEvent.click(play());
    await settle();
    // The page loaded again (PLAY_STOPPED): no address now.
    rerender(view(null));
    expect(screen.queryByRole("link", { name: "Play" })).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("SLAPorSLOP no longer opens this game: its address was withdrawn. Close that tab.");
  });

  it("with nothing to play, or an address that isn't a web page, has no Play, in words true whatever the reason", () => {
    controls({ address: null });
    expect(screen.queryByRole("link", { name: "Play" })).toBeNull();
    // Not approved yet, withdrawn, or out of the store: never "yet".
    expect(screen.getByText("Not playable right now.")).toBeTruthy();
    expect(screen.getByText("SLAPorSLOP has no address for it to open right now.")).toBeTruthy();
    expect(screen.queryByText(/approved yet/)).toBeNull();
    cleanup();
    controls({ address: "javascript:alert(1)" });
    expect(screen.queryByRole("link", { name: "Play" })).toBeNull();
    cleanup();
    // The library's bar: a big button that can't be pressed, until the store page has loaded.
    controls({ address: null, loading: true }, true);
    expect((screen.getByRole("button", { name: "Checking…" }) as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    controls({ address: null }, true);
    expect((screen.getByRole("button", { name: "Not playable right now" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Plays in your browser")).toBeTruthy();
    expect(screen.getByText("SLAPorSLOP has no address for it to open right now.")).toBeTruthy();
  });

  it("in the desktop app, records the play first, then opens the address SLAPorSLOP gives in the system browser", async () => {
    state.desktop = true;
    vi.mocked(api.launch).mockResolvedValue(launched("https://play.example.com/orbit/v2/"));
    controls();
    expect(document.getElementById(play().getAttribute("aria-describedby")!)?.textContent).toBe(
      "Opens play.example.com in your browser. Nothing to download or install.",
    );
    // The webview never follows the link itself.
    expect(fireEvent.click(play())).toBe(false);
    await settle();
    expect(api.launch).toHaveBeenCalledWith("orbit");
    expect(openExternal).toHaveBeenCalledWith("https://play.example.com/orbit/v2/");
    expect(launches).toEqual(["orbit"]);
    expect(desktop.launch).not.toHaveBeenCalled();
    expect(desktop.install).not.toHaveBeenCalled();
  });

  it("in the desktop app, a launch that fails or takes too long never stops the game", async () => {
    state.desktop = true;
    vi.mocked(api.launch).mockRejectedValue(new ApiError(503, "unavailable", "SLAPorSLOP is busy."));
    controls();
    fireEvent.click(play());
    await settle();
    expect(openExternal).toHaveBeenCalledWith(ADDRESS);
    expect(screen.getByRole("status").textContent).toMatch(/^SLAPorSLOP couldn't count this play/);
    cleanup();

    vi.useFakeTimers();
    vi.mocked(openExternal).mockClear();
    vi.mocked(api.launch).mockReturnValue(new Promise(() => {}));
    controls();
    fireEvent.click(play());
    await act(async () => vi.advanceTimersByTime(2900));
    expect(openExternal).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTime(200));
    expect(openExternal).toHaveBeenCalledWith(ADDRESS);
  });

  it("in the desktop app, a slow launch goes on being counted: once it is, the note goes and the page is told", async () => {
    state.desktop = true;
    vi.useFakeTimers();
    let finish: (l: Launched) => void = () => {};
    vi.mocked(api.launch).mockReturnValue(new Promise((done) => (finish = done)));
    controls();
    fireEvent.click(play());
    await act(async () => vi.advanceTimersByTime(3100));
    expect(openExternal).toHaveBeenCalledWith(ADDRESS);
    expect(screen.getByRole("status").textContent).toMatch(/^SLAPorSLOP couldn't count this play/);
    expect(launches).toEqual([]);
    // SLAPorSLOP answers after all: counted, so the rating and bug reports can open up.
    await act(async () => finish(launched()));
    expect(launches).toEqual(["orbit"]);
    expect(screen.queryByRole("status")).toBeNull();
    expect(openExternal).toHaveBeenCalledTimes(1);
  });

  it("in the desktop app, a slow launch that turns out withdrawn says to close the game it opened", async () => {
    state.desktop = true;
    vi.useFakeTimers();
    let fail: (e: unknown) => void = () => {};
    vi.mocked(api.launch).mockReturnValue(new Promise((_, no) => (fail = no)));
    controls();
    fireEvent.click(play());
    await act(async () => vi.advanceTimersByTime(3100));
    expect(openExternal).toHaveBeenCalledWith(ADDRESS);
    await act(async () => fail(new ApiError(409, "conflict", "nothing to play yet")));
    expect(screen.getByRole("status").textContent).toBe("SLAPorSLOP no longer opens this game: its address was withdrawn. Close that tab.");
    expect(stopped).toEqual(["orbit"]);
    expect(launches).toEqual([]);
  });

  it("in the desktop app, opens nothing when SLAPorSLOP says there's nothing to play, or the address isn't https", async () => {
    state.desktop = true;
    vi.mocked(api.launch).mockRejectedValue(new ApiError(409, "conflict", "nothing to play yet"));
    controls();
    fireEvent.click(play());
    await settle();
    expect(openExternal).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("Not opened: nothing to play yet.");
    // The page loads again, so Play goes away.
    expect(stopped).toEqual(["orbit"]);
    cleanup();

    vi.mocked(api.launch).mockResolvedValue(launched(null));
    controls({ address: "http://play.example.com/orbit/" });
    fireEvent.click(play());
    await settle();
    expect(openExternal).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("Not opened: the SLAPorSLOP app opens only https addresses.");
  });

  it("in the desktop app, opens nothing when SLAPorSLOP's own answer isn't an https address", async () => {
    state.desktop = true;
    // The store page's address is https; the one the launch answers with decides, and is checked too.
    for (const answer of ["http://evil.example/", "javascript:alert(1)", "file:///etc/passwd"]) {
      vi.mocked(openExternal).mockClear();
      vi.mocked(api.launch).mockResolvedValue(launched(answer));
      controls();
      fireEvent.click(play());
      await settle();
      expect(openExternal, answer).not.toHaveBeenCalled();
      expect(screen.getByRole("alert").textContent).toBe("Not opened: the SLAPorSLOP app opens only https addresses.");
      cleanup();
    }
    // Signed out, nothing is asked of SLAPorSLOP, and a plain http address isn't opened either.
    state.me = null;
    vi.mocked(api.launch).mockClear();
    vi.mocked(openExternal).mockClear();
    controls({ address: "http://play.example.com/orbit/" });
    fireEvent.click(play());
    await settle();
    expect(api.launch).not.toHaveBeenCalled();
    expect(openExternal).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("Not opened: the SLAPorSLOP app opens only https addresses.");
    // Signed out with an https address, it opens straight away.
    cleanup();
    controls();
    fireEvent.click(play());
    await settle();
    expect(api.launch).not.toHaveBeenCalled();
    expect(openExternal).toHaveBeenCalledWith(ADDRESS);
  });

  it("adds it to the library in the desktop app too, where nothing installs it", async () => {
    state.desktop = true;
    controls({ address: ADDRESS }, true);
    const add = screen.getByRole("button", { name: "+ Add to library" });
    expect(add.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(add);
    await settle();
    expect(api.addToLibrary).toHaveBeenCalledWith("orbit");
    expect(screen.getByRole("button", { name: "✓ In your library" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("on its own, for the rating's \"Press Play\", plays the same way", async () => {
    render(
      <MemoryRouter>
        <BrowserPlayLink slug="orbit" address={ADDRESS} />
      </MemoryRouter>,
    );
    fireEvent.click(play());
    await settle();
    expect(api.launch).toHaveBeenCalledWith("orbit");
    expect(launches).toEqual(["orbit"]);
    cleanup();
    const { container } = render(
      <MemoryRouter>
        <BrowserPlayLink slug="orbit" address={null} />
      </MemoryRouter>,
    );
    expect(container.textContent).toBe("");
  });
});
