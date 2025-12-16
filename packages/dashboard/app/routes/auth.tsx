import { useEffect } from "react";
import { useNavigate } from "react-router";

import { LoginCard } from "../auth/LoginCard";
import { useSession } from "../auth/session";

/**
 * Auth page route. Shows the login card for unauthenticated users.
 * Redirects authenticated users to home.
 */
export default function AuthPage() {
  const nav = useNavigate();
  const { sessionToken } = useSession();

  useEffect(() => {
    if (sessionToken) {
      nav("/", { replace: true });
    }
  }, [sessionToken, nav]);

  // If already authenticated, show nothing while redirecting
  if (sessionToken) {
    return null;
  }

  return <LoginCard />;
}
