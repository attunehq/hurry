import { Navigate, createBrowserRouter, RouterProvider } from "react-router-dom";

import { AppShell } from "../ui/shell/AppShell";
import { AuthCallbackPage } from "../views/auth/AuthCallbackPage";
import { AuthPage } from "../views/auth/AuthPage";
import { BillingPage } from "../views/billing/BillingPage";
import { DashboardHome } from "../views/home/DashboardHome";
import { InvitePage } from "../views/invite/InvitePage";
import { OrgApiKeysPage } from "../views/org/OrgApiKeysPage";
import { OrgBotsPage } from "../views/org/OrgBotsPage";
import { OrgInvitationsPage } from "../views/org/OrgInvitationsPage";
import { OrgLayout } from "../views/org/OrgLayout";
import { OrgMembersPage } from "../views/org/OrgMembersPage";
import { NotFoundPage } from "../views/system/NotFoundPage";
import { UserPage } from "../views/user/UserPage";
import { SessionProvider } from "../auth/session";
import { ToastProvider } from "../ui/toast/ToastProvider";

const router = createBrowserRouter([
  {
    path: "/",
    element: (
      <SessionProvider>
        <ToastProvider>
          <AppShell />
        </ToastProvider>
      </SessionProvider>
    ),
    errorElement: <NotFoundPage />,
    children: [
      { index: true, element: <DashboardHome /> },
      { path: "auth", element: <AuthPage /> },
      { path: "auth/callback", element: <AuthCallbackPage /> },
      { path: "billing", element: <BillingPage /> },
      { path: "user", element: <UserPage /> },
      { path: "invite/:token", element: <InvitePage /> },
      {
        path: "org/:orgId",
        element: <OrgLayout />,
        children: [
          { index: true, element: <Navigate to="members" replace /> },
          { path: "members", element: <OrgMembersPage /> },
          { path: "api-keys", element: <OrgApiKeysPage /> },
          { path: "invitations", element: <OrgInvitationsPage /> },
          { path: "bots", element: <OrgBotsPage /> },
        ],
      },
    ],
  },
]);

export function AppRouter() {
  return <RouterProvider router={router} />;
}
