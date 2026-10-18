// The signed-in user, shared across the app.
//
// - Website: a cookie session; signing in is a full-page trip to GitHub (see pages/Auth.tsx).
// - Desktop app: tokens held by the Rust side. Signing in happens in the app's own small sign-in
//   window (login.html / src/login.tsx) before the store opens, like Steam.

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";

import { api, type Me } from "./api";
import { desktop, isDesktop } from "./platform";

interface Session {
  /** undefined while loading, null when signed out. */
  me: Me | null | undefined;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
}

const SessionContext = createContext<Session | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null | undefined>(undefined);

  const refresh = useCallback(async () => {
    if (isDesktop && !(await desktop.sessionStatus()).signed_in) {
      setMe(null);
      return;
    }
    try {
      setMe(await api.me());
    } catch {
      // 401 means signed out; if the API is unreachable, treat the viewer as signed out too.
      setMe(null);
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
    return () => unlisten?.();
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
