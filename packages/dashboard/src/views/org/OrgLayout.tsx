import { ArrowLeft, DoorOpen, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useNavigate, useParams } from "react-router-dom";

import { apiRequest } from "../../api/client";
import type { OrganizationEntry, OrganizationListResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody } from "../../ui/primitives/Card";
import { useToast } from "../../ui/toast/ToastProvider";

export function OrgLayout() {
  const nav = useNavigate();
  const toast = useToast();
  const { orgId } = useParams();
  const { sessionToken } = useSession();
  const [org, setOrg] = useState<OrganizationEntry | null>(null);
  const [loading, setLoading] = useState(false);

  const id = useMemo(() => Number(orgId ?? "0"), [orgId]);

  async function refresh() {
    if (!sessionToken || !id) return;
    setLoading(true);
    try {
      const out = await apiRequest<OrganizationListResponse>({
        path: "/api/v1/me/organizations",
        sessionToken,
      });
      const found = out.organizations.find((o) => o.id === id) ?? null;
      setOrg(found);
      if (!found) toast.push({ kind: "error", title: "Org not found (or no access)" });
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load org", detail: msg });
    } finally {
      setLoading(false);
    }
  }

  async function leave() {
    if (!sessionToken || !id) return;
    if (!confirm("Leave this organization?")) return;
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${id}/leave`,
        method: "POST",
        sessionToken,
      });
      toast.push({ kind: "success", title: "Left organization" });
      nav("/");
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Leave failed", detail: msg });
    }
  }

  useEffect(() => {
    void refresh();
  }, [sessionToken, id]);

  if (!sessionToken) {
    return (
      <Card>
        <CardBody>
          <div className="flex items-center justify-between">
            <div className="text-sm text-slate-300">Sign in to view this organization.</div>
            <Button onClick={() => nav("/auth")} variant="secondary">
              Go to auth
            </Button>
          </div>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-col items-start justify-between gap-3 md:flex-row md:items-center">
        <div className="flex items-center gap-3">
          <Button variant="ghost" onClick={() => nav("/")}>
            <ArrowLeft className="h-4 w-4" />
            Back
          </Button>
          <div>
            <div className="text-lg font-semibold text-slate-100">
              {org ? org.name : `Org ${id}`}
            </div>
            <div className="mt-1 flex items-center gap-2 text-xs text-slate-400">
              <span>Org ID: {id}</span>
              {org ? <Badge tone={org.role === "admin" ? "neon" : "muted"}>{org.role}</Badge> : null}
            </div>
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={refresh} disabled={loading}>
            <RefreshCw className="h-4 w-4" />
            Refresh
          </Button>
          <Button variant="danger" onClick={leave}>
            <DoorOpen className="h-4 w-4" />
            Leave
          </Button>
        </div>
      </div>

      <div className="rounded-2xl border border-white/10 bg-ink-900/55 p-2 shadow-glow-soft backdrop-blur">
        <div className="flex flex-wrap gap-1">
          <Tab to="members" label="Members" />
          <Tab to="api-keys" label="API Keys" />
          <Tab to="invitations" label="Invitations" />
          <Tab to="bots" label="Bots" />
        </div>
      </div>

      <Outlet context={{ orgId: id, role: org?.role ?? null }} />
    </div>
  );
}

function Tab(props: { to: string; label: string }) {
  return (
    <NavLink
      to={props.to}
      className={({ isActive }) =>
        [
          "rounded-xl px-3 py-2 text-sm transition",
          isActive ? "bg-white/6 text-slate-100" : "text-slate-300 hover:bg-white/5 hover:text-slate-100",
        ].join(" ")
      }
    >
      {props.label}
    </NavLink>
  );
}

