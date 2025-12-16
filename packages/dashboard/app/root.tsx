import { Links, Meta, Outlet, Scripts, ScrollRestoration } from "react-router";

import { SessionProvider } from "./auth/session";
import { ThemeProvider } from "./ui/theme/ThemeProvider";
import { ToastProvider } from "./ui/toast/ToastProvider";
import { AppShell } from "./ui/shell/AppShell";
import "./styles.css";

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
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
        <ToastProvider>
          <AppShell>
            <Outlet />
          </AppShell>
        </ToastProvider>
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
        <h1 className="text-xl font-bold text-red-500">Application Error</h1>
        <pre className="mt-4 overflow-auto text-sm text-red-400">
          {error.message}
          {"\n"}
          {error.stack}
        </pre>
      </div>
    </div>
  );
}
