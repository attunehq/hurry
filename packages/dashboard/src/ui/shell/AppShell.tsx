import { CreditCard, LogIn, LogOut, User, Users } from "lucide-react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";

import { apiRequest } from "../../api/client";
import { useSession } from "../../auth/session";
import { Button } from "../primitives/Button";

function brand() {
  return (
    <div className="flex items-center gap-3">
      <div className="grid h-11 w-11 place-items-center rounded-xl border border-white/10 bg-white/5 shadow-glow-soft">
        <span className="text-2xl font-bold bg-gradient-to-br from-attune-300 to-attune-500 bg-clip-text text-transparent">
          A
        </span>
      </div>
      <div className="text-xl font-semibold text-slate-100">Hurry</div>
    </div>
  );
}

export function AppShell() {
  const nav = useNavigate();
  const loc = useLocation();
  const { sessionToken, setSessionToken } = useSession();

  const signedIn = Boolean(sessionToken);

  async function logout() {
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
  }

  return (
    <div className="noise min-h-screen">
      <div className="mx-auto flex max-w-6xl gap-8 px-6 pb-12 pt-10">
        <aside className="hidden w-64 shrink-0 flex-col gap-5 md:flex">
          {brand()}
          <div className="rounded-2xl border border-white/10 bg-ink-900/55 shadow-glow-soft backdrop-blur">
            <div className="border-b border-white/10 px-4 py-3 text-xs font-semibold text-slate-300">
              Console
            </div>
            <nav className="flex flex-col p-2 text-sm">
              <NavLink
                to="/"
                className={({ isActive }) =>
                  [
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-slate-300 hover:bg-white/5 hover:text-slate-100",
                    isActive ? "bg-white/5 text-slate-100" : "",
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
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-slate-300 hover:bg-white/5 hover:text-slate-100",
                    isActive ? "bg-white/5 text-slate-100" : "",
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
                    "flex items-center gap-2 rounded-xl px-3 py-2 text-slate-300 hover:bg-white/5 hover:text-slate-100",
                    isActive ? "bg-white/5 text-slate-100" : "",
                  ].join(" ")
                }
              >
                <CreditCard className="h-4 w-4" />
                Billing
              </NavLink>
            </nav>
          </div>

          <div className="mt-auto rounded-2xl border border-white/10 bg-ink-900/55 p-4 text-xs text-slate-400 shadow-glow-soft backdrop-blur">
            <div className="font-semibold text-slate-300">Deployment</div>
            <div className="mt-1">Target: app.hurry.attunehq.com</div>
            <div className="mt-3 flex gap-2">
              {signedIn ? (
                <Button variant="secondary" size="sm" onClick={logout}>
                  <LogOut className="h-4 w-4" />
                  Sign out
                </Button>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => nav("/auth", { state: { from: loc.pathname } })}
                >
                  <LogIn className="h-4 w-4" />
                  Sign in
                </Button>
              )}
            </div>
          </div>
        </aside>

        <main className="min-w-0 flex-1">
          <div className="mb-4 flex items-center justify-between md:hidden">
            {brand()}
            {signedIn ? (
              <Button variant="secondary" size="sm" onClick={logout}>
                <LogOut className="h-4 w-4" />
                Sign out
              </Button>
            ) : (
              <Button variant="secondary" size="sm" onClick={() => nav("/auth")}>
                <LogIn className="h-4 w-4" />
                Sign in
              </Button>
            )}
          </div>

          <Outlet />
        </main>
      </div>
    </div>
  );
}

