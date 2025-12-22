import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router";

import { LoginCard } from "../auth/LoginCard";
import { useSession } from "../auth/session";

interface LocationState {
  inviteToken?: string;
}

/**
 * Auth page route. Shows the login card for unauthenticated users.
 * Redirects authenticated users to home.
 */
export default function AuthPage() {
  const nav = useNavigate();
  const location = useLocation();
  const { sessionToken } = useSession();

  const state = location.state as LocationState | null;
  const inviteToken = state?.inviteToken;

  useEffect(() => {
    if (sessionToken) {
      nav("/", { replace: true });
    }
  }, [sessionToken, nav]);

  // If already authenticated, show nothing while redirecting
  if (sessionToken) {
    return null;
  }

  return <LoginCard inviteToken={inviteToken} />;
}
