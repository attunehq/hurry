import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import type { MeResponse } from "../api/types";
import { useApi } from "../api/useApi";

type UserContextState = {
  user: MeResponse | null;
  loading: boolean;
  refresh: () => Promise<void>;
};

const UserContext = createContext<UserContextState | null>(null);

export function UserProvider({ children }: { children: ReactNode }) {
  const { request, signedIn } = useApi();
  const [user, setUser] = useState<MeResponse | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setUser(null);
      return;
    }
    setLoading(true);
    try {
      const out = await request<MeResponse>({
        path: "/api/v1/me",
      });
      setUser(out);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) {
        return;
      }
      setUser(null);
    } finally {
      setLoading(false);
    }
  }, [signedIn, request]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const state = useMemo<UserContextState>(
    () => ({ user, loading, refresh }),
    [user, loading, refresh]
  );

  return <UserContext.Provider value={state}>{children}</UserContext.Provider>;
}

export function useUser() {
  const ctx = useContext(UserContext);
  if (!ctx) throw new Error("useUser must be used within UserProvider");
  return ctx;
}
