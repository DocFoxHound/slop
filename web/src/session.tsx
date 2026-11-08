// SPDX-License-Identifier: AGPL-3.0-or-later
// The signed-in user, shared across the app.
//
// - Website: a cookie session; signing in is a full-page trip to GitHub (see pages/Auth.tsx).
// - Desktop app: tokens held by the Rust side. Signing in happens in the app's own small sign-in
//   window (login.html / src/login.tsx) before the store opens, like Steam.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

import { api, ApiError, type Me } from "./api";
import { desktop, isDesktop } from "./platform";

/** How long the desktop app waits before asking again, after `tries` failed asks: 2 s, doubling, at most a minute. */
export const retryDelay = (tries: number) => Math.min(60_000, 2_000 * 2 ** tries);

interface Session {
  /** undefined while loading, null when signed out. */
  me: Me | null | undefined;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
}

const SessionContext = createContext<Session | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null | undefined>(undefined);
  const retry = useRef<number | undefined>(undefined);
  const tries = useRef(0);

  const refresh = useCallback(async () => {
    window.clearTimeout(retry.current);
    if (isDesktop && !(await desktop.sessionStatus()).signed_in) {
      setMe(null);
      return;
    }
    try {
      setMe(await api.me());
      tries.current = 0;
    } catch (e) {
      // 401 means signed out; if the API is unreachable, treat the viewer as signed out too.
      setMe(null);
      // The desktop app still has its sign-in, though, and has no reload button: SLAPorSLOP may
      // be restarting, or the network not up yet just after an update. Ask again until it
      // answers, instead of showing the store signed out until the app restarts.
      const signedOut = e instanceof ApiError && (e.status === 401 || e.code === "signed_out");
      if (isDesktop && !signedOut) {
        retry.current = window.setTimeout(() => void refresh(), retryDelay(tries.current++));
      }
    }
  }, []);

  const signOut = useCallback(async () => {
    if (isDesktop) await desktop.signOut();
    else await api.signout();
    setMe(null);
  }, []);

  useEffect(() => {
    void refresh();
    if (!isDesktop) return;
    // The sign-in window is a separate page; it tells this one when the session changes.
    let unlisten: (() => void) | undefined;
    void desktop.onSessionChanged(() => void refresh()).then((u) => (unlisten = u));
    return () => {
      unlisten?.();
      window.clearTimeout(retry.current);
    };
  }, [refresh]);

  return (
    <SessionContext.Provider value={{ me, refresh, signOut }}>{children}</SessionContext.Provider>
  );
}

export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) throw new Error("useSession must be used inside SessionProvider");
  return session;
}
