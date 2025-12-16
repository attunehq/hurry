// The functions in this file are pretty cursed, but they do seem to work.
// TODO: we might want to move to SWR or something for less cursed mechanics.

import { useCallback } from "react";
import { useNavigate } from "react-router";

import { useSession } from "../auth/session";
import { apiRequest } from "./client";

/**
 * Hook that provides API utilities with automatic session handling.
 *
 * - `request`: Make authenticated API calls. 401s automatically clear session and redirect.
 * - `logout`: Sign out the current user.
 * - `sessionToken`: The current session token (null if not signed in).
 * - `signedIn`: Whether the user is signed in.
 */
export function useApi() {
  const nav = useNavigate();
  const { sessionToken, setSessionToken, handleUnauthorized } = useSession();

  const request = useCallback(
    <T>(args: {
      path: string;
      method?: "GET" | "POST" | "PATCH" | "DELETE";
      body?: unknown;
    }) => {
      return apiRequest<T>({
        ...args,
        sessionToken,
        onUnauthorized: handleUnauthorized,
      });
    },
    [sessionToken, handleUnauthorized]
  );

  const logout = useCallback(async () => {
    try {
      await apiRequest<void>({
        path: "/api/v1/oauth/logout",
        method: "POST",
        sessionToken,
      });
    } catch {
      // Even if logout fails, clearing local state is still useful.
    } finally {
      setSessionToken(null);
      nav("/auth");
    }
  }, [sessionToken, setSessionToken, nav]);

  return {
    request,
    logout,
    sessionToken,
    signedIn: Boolean(sessionToken),
  };
}
