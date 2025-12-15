import React, { createContext, useContext, useMemo, useState } from "react";

const STORAGE_KEY = "courier.sessionToken";

type SessionState = {
  sessionToken: string | null;
  setSessionToken: (token: string | null) => void;
};

const SessionContext = createContext<SessionState | null>(null);

export function SessionProvider(props: { children: React.ReactNode }) {
  const [sessionToken, setSessionTokenState] = useState<string | null>(() => {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw && raw.trim().length > 0 ? raw : null;
  });

  const state = useMemo<SessionState>(() => {
    return {
      sessionToken,
      setSessionToken: (token) => {
        if (token && token.trim().length > 0) {
          localStorage.setItem(STORAGE_KEY, token);
          setSessionTokenState(token);
          return;
        }
        localStorage.removeItem(STORAGE_KEY);
        setSessionTokenState(null);
      },
    };
  }, [sessionToken]);

  return (
    <SessionContext.Provider value={state}>
      {props.children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error("useSession must be used within SessionProvider");
  return ctx;
}

