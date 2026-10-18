// The signed-in user, shared across the app.
//
// - Website: a cookie session; signing in is a full-page trip to GitHub (see pages/Auth.tsx).
// - Desktop app: tokens held by the Rust side; signing in opens the system browser, where the
//   user approves this device (docs/SECURITY.md §4.1).

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";

import { api, type Me } from "./api";
import { desktop, isDesktop } from "./platform";

interface Session {
  /** undefined while loading, null when signed out. */
  me: Me | null | undefined;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
  /** Desktop only: open the browser and wait for the user to approve this device. */
  desktopSignIn: () => Promise<void>;
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

  const desktopSignIn = useCallback(async () => {
    await desktop.signIn();
    await refresh();
  }, [refresh]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <SessionContext.Provider value={{ me, refresh, signOut, desktopSignIn }}>{children}</SessionContext.Provider>
  );
}

export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) throw new Error("useSession must be used inside SessionProvider");
  return session;
}
