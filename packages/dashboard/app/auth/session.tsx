import React, { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";

const STORAGE_KEY = "hurry.sessionToken";

type SessionState = {
  sessionToken: string | null;
  setSessionToken: (token: string | null) => void;
  handleUnauthorized: () => void;
  onSessionInvalidated: (callback: () => void) => () => void;
};

const SessionContext = createContext<SessionState | null>(null);

export function SessionProvider(props: { children: React.ReactNode }) {
  const [sessionToken, setSessionTokenState] = useState<string | null>(() => {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw && raw.trim().length > 0 ? raw : null;
  });

  const invalidationCallbacksRef = useRef<Set<() => void>>(new Set());
  const clearSession = useCallback(() => {
    localStorage.removeItem(STORAGE_KEY);
    setSessionTokenState(null);
  }, []);

  const handleUnauthorized = useCallback(() => {
    // Avoids loops when already logged out
    if (!localStorage.getItem(STORAGE_KEY)) return;

    clearSession();
    invalidationCallbacksRef.current.forEach((cb) => cb());
  }, [clearSession]);

  const onSessionInvalidated = useCallback((callback: () => void) => {
    invalidationCallbacksRef.current.add(callback);
    return () => {
      invalidationCallbacksRef.current.delete(callback);
    };
  }, []);

  const state = useMemo<SessionState>(() => {
    return {
      sessionToken,
      setSessionToken: (token) => {
        if (token && token.trim().length > 0) {
          localStorage.setItem(STORAGE_KEY, token);
          setSessionTokenState(token);
          return;
        }
        clearSession();
      },
      handleUnauthorized,
      onSessionInvalidated,
    };
  }, [sessionToken, clearSession, handleUnauthorized, onSessionInvalidated]);

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
