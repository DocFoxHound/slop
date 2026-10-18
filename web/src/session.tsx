// The signed-in user, shared across the app.

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";

import { api, type Me } from "./api";
import { isDesktop } from "./platform";

interface Session {
  /** undefined while loading, null when signed out. */
  me: Me | null | undefined;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
  /** Desktop sign-in (system browser + loopback, docs/SECURITY.md §4.1) isn't built yet. */
  canSignIn: boolean;
}

const SessionContext = createContext<Session | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null | undefined>(isDesktop ? null : undefined);

  const refresh = useCallback(async () => {
    if (isDesktop) return;
    try {
      setMe(await api.me());
    } catch {
      // 401 means signed out; if the API is unreachable, treat the viewer as signed out too.
      setMe(null);
    }
  }, []);

  const signOut = useCallback(async () => {
    await api.signout();
    setMe(null);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <SessionContext.Provider value={{ me, refresh, signOut, canSignIn: !isDesktop }}>{children}</SessionContext.Provider>
  );
}

export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) throw new Error("useSession must be used inside SessionProvider");
  return session;
}
