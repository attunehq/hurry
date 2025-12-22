import { Links, Meta, Outlet, Scripts, ScrollRestoration } from "react-router";

import { AuthGate } from "./auth/AuthGate";
import { SessionProvider } from "./auth/session";
import { OrgProvider } from "./org/OrgContext";
import { UserProvider } from "./user/UserContext";
import { ThemeProvider } from "./ui/theme/ThemeProvider";
import { ToastProvider } from "./ui/toast/ToastProvider";
import { AppShell } from "./ui/shell/AppShell";
import "./styles.css";

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" style={{ overflowY: "scroll" }}>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="icon" type="image/png" href="/attune.png" />
        <title>Hurry Console</title>
        <Meta />
        <Links />
      </head>
      <body>
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}

export default function Root() {
  return (
    <ThemeProvider>
      <SessionProvider>
        <OrgProvider>
          <UserProvider>
            <ToastProvider>
              <AuthGate
                shell={(children) => <AppShell>{children}</AppShell>}
              >
                <Outlet />
              </AuthGate>
            </ToastProvider>
          </UserProvider>
        </OrgProvider>
      </SessionProvider>
    </ThemeProvider>
  );
}

export function HydrateFallback() {
  return (
    <div className="flex min-h-screen items-center justify-center">
      <div className="text-content-tertiary">Loading...</div>
    </div>
  );
}

export function ErrorBoundary({ error }: { error: Error }) {
  console.error("Root ErrorBoundary caught:", error);
  return (
    <div className="flex min-h-screen items-center justify-center">
      <div className="max-w-md p-6">
        <h1 className="text-xl font-bold text-danger-text">Application Error</h1>
        <pre className="mt-4 overflow-auto text-sm text-danger-text">
          {error.message}
          {"\n"}
          {error.stack}
        </pre>
      </div>
    </div>
  );
}
