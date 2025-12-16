import type { ReactNode } from "react";
import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router";

import { useSession } from "./session";

/**
 * Routes that don't require authentication.
 * These are matched as prefixes against the current pathname.
 */
const PUBLIC_ROUTES = ["/auth", "/invite"];

function isPublicRoute(pathname: string): boolean {
  return PUBLIC_ROUTES.some((route) => pathname.startsWith(route));
}

type AuthGateProps = {
  children: ReactNode;
  /** Render function for the app shell, only applied for authenticated users on protected routes */
  shell: (children: ReactNode) => ReactNode;
};

/**
 * Authentication gate that redirects unauthenticated users to the login page.
 * Public routes (like /auth/callback) render without the shell.
 * Authenticated users on protected routes get the shell.
 */
export function AuthGate({ children, shell }: AuthGateProps) {
  const nav = useNavigate();
  const { sessionToken } = useSession();
  const { pathname } = useLocation();

  const isAuthenticated = Boolean(sessionToken);
  const isPublic = isPublicRoute(pathname);

  useEffect(() => {
    if (!isAuthenticated && !isPublic) {
      nav("/auth", { replace: true });
    }
  }, [isAuthenticated, isPublic, nav]);

  // Public routes render without shell
  if (isPublic) {
    return <>{children}</>;
  }

  // Protected routes require authentication - show nothing while redirecting
  if (!isAuthenticated) {
    return null;
  }

  // Authenticated users on protected routes get the shell
  return <>{shell(children)}</>;
}
