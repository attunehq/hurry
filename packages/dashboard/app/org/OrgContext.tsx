import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import type { OrganizationEntry, OrganizationListResponse } from "../api/types";
import { useApi } from "../api/useApi";

type OrgContextState = {
  orgs: OrganizationEntry[] | null;
  loading: boolean;
  refresh: () => Promise<void>;
};

const OrgContext = createContext<OrgContextState | null>(null);

export function OrgProvider({ children }: { children: ReactNode }) {
  const { request, signedIn } = useApi();
  const [orgs, setOrgs] = useState<OrganizationEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setOrgs(null);
      return;
    }
    setLoading(true);
    try {
      const out = await request<OrganizationListResponse>({
        path: "/api/v1/me/organizations",
      });
      setOrgs(out.organizations);
    } catch (e) {
      // Don't clear orgs on 401 - session invalidation handles that
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) {
        return;
      }
      setOrgs(null);
    } finally {
      setLoading(false);
    }
  }, [signedIn, request]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const state = useMemo<OrgContextState>(
    () => ({ orgs, loading, refresh }),
    [orgs, loading, refresh]
  );

  return <OrgContext.Provider value={state}>{children}</OrgContext.Provider>;
}

export function useOrgs() {
  const ctx = useContext(OrgContext);
  if (!ctx) throw new Error("useOrgs must be used within OrgProvider");
  return ctx;
}
