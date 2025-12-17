import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import type { OrganizationEntry, OrganizationListResponse } from "../api/types";
import { useApi } from "../api/useApi";

const LAST_ORG_KEY = "hurry.lastOrgId";

type OrgContextState = {
  orgs: OrganizationEntry[] | null;
  loading: boolean;
  refresh: () => Promise<void>;
  lastOrgId: number | null;
  setLastOrgId: (id: number) => void;
};

const OrgContext = createContext<OrgContextState | null>(null);

function getStoredLastOrgId(): number | null {
  const raw = localStorage.getItem(LAST_ORG_KEY);
  if (!raw) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function storeLastOrgId(id: number): void {
  localStorage.setItem(LAST_ORG_KEY, String(id));
}

export function OrgProvider({ children }: { children: ReactNode }) {
  const { request, signedIn } = useApi();
  const [orgs, setOrgs] = useState<OrganizationEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [lastOrgId, setLastOrgIdState] = useState<number | null>(getStoredLastOrgId);

  const setLastOrgId = useCallback((id: number) => {
    storeLastOrgId(id);
    setLastOrgIdState(id);
  }, []);

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
    () => ({ orgs, loading, refresh, lastOrgId, setLastOrgId }),
    [orgs, loading, refresh, lastOrgId, setLastOrgId]
  );

  return <OrgContext.Provider value={state}>{children}</OrgContext.Provider>;
}

export function useOrgs() {
  const ctx = useContext(OrgContext);
  if (!ctx) throw new Error("useOrgs must be used within OrgProvider");
  return ctx;
}
