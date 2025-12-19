import type { ReactNode } from "react";
import { useEffect } from "react";
import { useNavigate } from "react-router";

import { useSession } from "../../auth/session";
import { useToast } from "../toast/ToastProvider";
import { OrgSwitcher } from "./OrgSwitcher";
import { UserMenu } from "./UserMenu";

function brand() {
  return (
    <div className="flex items-center gap-3">
      <div className="grid h-11 w-11 place-items-center rounded-xl border border-border bg-surface-subtle shadow-glow-soft">
        <span className="text-2xl font-bold bg-linear-to-br from-attune-300 to-attune-500 bg-clip-text text-transparent">
          A
        </span>
      </div>
      <div className="text-xl font-semibold text-content-primary">Hurry</div>
    </div>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const nav = useNavigate();
  const toast = useToast();
  const { onSessionInvalidated } = useSession();

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
      <div className="mx-auto max-w-6xl px-6 pb-12 pt-10">
        {/* Header: brand, org switcher, user menu */}
        <header className="mb-8 flex items-center gap-4">
          {brand()}
          <div className="flex-1">
            <OrgSwitcher />
          </div>
          <UserMenu />
        </header>

        {/* Page content */}
        {children}
      </div>
    </div>
  );
}
