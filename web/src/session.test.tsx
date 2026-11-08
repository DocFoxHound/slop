// SPDX-License-Identifier: AGPL-3.0-or-later
// The desktop app's session (session.tsx): when SLAPorSLOP can't be reached at launch (just after an
// update, say) while the app still has its sign-in, the store shows the viewer signed out but keeps
// asking, and shows them signed in once it answers. A 401 means signed out: nothing more is asked.
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const me = vi.fn();
const sessionStatus = vi.fn();

vi.mock("./platform", () => ({
  isDesktop: true,
  desktop: {
    sessionStatus: () => sessionStatus(),
    onSessionChanged: () => Promise.resolve(() => {}),
    signOut: () => Promise.resolve(),
  },
}));

vi.mock("./api", async (original) => ({
  ...(await original<typeof import("./api")>()),
  api: { me: () => me() },
}));

const { ApiError } = await import("./api");
const { SessionProvider, retryDelay, useSession } = await import("./session");

function Who() {
  const { me } = useSession();
  return <p>{me === undefined ? "loading" : me === null ? "signed out" : `signed in as ${me.display_name}`}</p>;
}

const offline = () => new ApiError(0, "network", "can't reach SLAPorSLOP: connection refused");

/** Lets the pending promises settle, then moves the clock on. */
async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  me.mockReset();
  sessionStatus.mockReset().mockResolvedValue({ signed_in: true });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("the desktop app's session", () => {
  it("keeps asking while SLAPorSLOP can't be reached, then shows the viewer signed in", async () => {
    me.mockRejectedValueOnce(offline()).mockRejectedValueOnce(offline()).mockResolvedValue({ display_name: "Pat" });
    render(
      <SessionProvider>
        <Who />
      </SessionProvider>,
    );
    await wait(0);
    expect(screen.getByText("signed out")).toBeTruthy();
    await wait(retryDelay(0));
    expect(me).toHaveBeenCalledTimes(2);
    expect(screen.getByText("signed out")).toBeTruthy();
    await wait(retryDelay(1));
    expect(me).toHaveBeenCalledTimes(3);
    expect(screen.getByText("signed in as Pat")).toBeTruthy();
    // Once answered, it stops asking.
    await wait(10 * 60_000);
    expect(me).toHaveBeenCalledTimes(3);
  });

  it("stops at a 401: the sign-in was refused", async () => {
    me.mockRejectedValue(new ApiError(401, "unauthorized", "Sign in first."));
    render(
      <SessionProvider>
        <Who />
      </SessionProvider>,
    );
    await wait(10 * 60_000);
    expect(screen.getByText("signed out")).toBeTruthy();
    expect(me).toHaveBeenCalledTimes(1);
  });

  it("doesn't ask at all when the app has no sign-in", async () => {
    sessionStatus.mockResolvedValue({ signed_in: false });
    render(
      <SessionProvider>
        <Who />
      </SessionProvider>,
    );
    await wait(10 * 60_000);
    expect(screen.getByText("signed out")).toBeTruthy();
    expect(me).not.toHaveBeenCalled();
  });

  it("waits 2 s, doubling, at most a minute", () => {
    expect([0, 1, 2, 3, 4, 5, 9].map(retryDelay)).toEqual([2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000]);
  });
});
