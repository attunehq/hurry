import { CreditCard, User, Users } from "lucide-react";
import { useEffect } from "react";
import { NavLink, Outlet, useNavigate } from "react-router-dom";

import { useSession } from "../../auth/session";
import { useToast } from "../toast/ToastProvider";

function brand() {
  return (
    <div className="flex items-center gap-3">
      <div className="grid h-11 w-11 place-items-center rounded-xl border border-border bg-surface-subtle shadow-glow-soft">
        <span className="text-2xl font-bold bg-gradient-to-br from-attune-300 to-attune-500 bg-clip-text text-transparent">
          A
        </span>
      </div>
      <div className="text-xl font-semibold text-content-primary">Hurry</div>
    </div>
  );
}

export function AppShell() {
  const nav = useNavigate();
  const toast = useToast();
  const { onSessionInvalidated } = useSession();

  // Handle session invalidation (401 from any API request)
  useEffect(() => {
    return onSessionInvalidated(() => {
      toast.push({
        kind: "error",
        title: "Session expired",
        detail: "Your session is no longer active. Please sign in again.",
      });
      nav("/auth");
    });
  }, [nav, toast, onSessionInvalidated]);

  return (
    <div className="noise min-h-screen">
      <div className="mx-auto flex max-w-6xl gap-8 px-6 pb-12 pt-10">
        <aside className="hidden w-64 shrink-0 flex-col gap-5 md:flex">
          {brand()}
          <div className="rounded-2xl border border-border bg-surface-raised shadow-glow-soft backdrop-blur">
            <div className="border-b border-border px-4 py-3 text-xs font-semibold text-content-tertiary">
              Console
            </div>
            <nav className="flex flex-col p-2 text-sm">
              <NavLink
                to="/"
                className={({ isActive }) =>
                  [
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-content-tertiary hover:bg-surface-subtle hover:text-content-primary",
                    isActive ? "bg-surface-subtle text-content-primary" : "",
                  ].join(" ")
                }
              >
                <Users className="h-4 w-4" />
                Organizations
              </NavLink>
              <NavLink
                to="/user"
                className={({ isActive }) =>
                  [
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-content-tertiary hover:bg-surface-subtle hover:text-content-primary",
                    isActive ? "bg-surface-subtle text-content-primary" : "",
                  ].join(" ")
                }
              >
                <User className="h-4 w-4" />
                Account
              </NavLink>
              <NavLink
                to="/billing"
                className={({ isActive }) =>
                  [
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-content-tertiary hover:bg-surface-subtle hover:text-content-primary",
                    isActive ? "bg-surface-subtle text-content-primary" : "",
                  ].join(" ")
                }
              >
                <CreditCard className="h-4 w-4" />
                Billing
              </NavLink>
            </nav>
          </div>
        </aside>

        <main className="min-w-0 flex-1">
          <div className="mb-4 md:hidden">
            {brand()}
          </div>

          <Outlet />
        </main>
      </div>
    </div>
  );
}
