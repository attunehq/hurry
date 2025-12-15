import { Building2, ExternalLink, Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";

import { apiRequest } from "../../api/client";
import type {
  CreateOrganizationResponse,
  MeResponse,
  OrganizationEntry,
  OrganizationListResponse,
} from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { Modal } from "../../ui/primitives/Modal";
import { useToast } from "../../ui/toast/ToastProvider";

export function DashboardHome() {
  const nav = useNavigate();
  const toast = useToast();
  const { sessionToken } = useSession();
  const [me, setMe] = useState<MeResponse | null>(null);
  const [orgs, setOrgs] = useState<OrganizationEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [orgName, setOrgName] = useState("");

  const signedIn = Boolean(sessionToken);

  const headerLine = useMemo(() => {
    if (!me) return "Courier Dashboard";
    const who = me.name?.trim() ? me.name.trim() : me.github_username ?? me.email;
    return `Welcome, ${who}`;
  }, [me]);

  async function refresh() {
    if (!sessionToken) {
      setMe(null);
      setOrgs(null);
      return;
    }
    setLoading(true);
    try {
      const meOut = await apiRequest<MeResponse>({ path: "/api/v1/me", sessionToken });
      const orgsOut = await apiRequest<OrganizationListResponse>({
        path: "/api/v1/me/organizations",
        sessionToken,
      });
      setMe(meOut);
      setOrgs(orgsOut.organizations);
    } catch (e) {
      setMe(null);
      setOrgs(null);
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load", detail: msg });
    } finally {
      setLoading(false);
    }
  }

  async function createOrg() {
    if (!sessionToken) {
      toast.push({ kind: "error", title: "Sign in first" });
      nav("/auth");
      return;
    }
    const name = orgName.trim();
    if (!name) {
      toast.push({ kind: "error", title: "Organization name required" });
      return;
    }
    try {
      const created = await apiRequest<CreateOrganizationResponse>({
        path: "/api/v1/organizations",
        method: "POST",
        sessionToken,
        body: { name },
      });
      toast.push({ kind: "success", title: "Organization created", detail: created.name });
      setCreateOpen(false);
      setOrgName("");
      await refresh();
      nav(`/org/${created.id}`);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  useEffect(() => {
    void refresh();
  }, [sessionToken]);

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
        <div>
          <div className="text-xl font-semibold text-slate-100">{headerLine}</div>
          <div className="mt-1 text-sm text-slate-300">
            Manage organizations, invitations, API keys, and bot identities.
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={refresh} disabled={!signedIn || loading}>
            <RefreshCw className="h-4 w-4" />
            Refresh
          </Button>
          <Button onClick={() => setCreateOpen(true)} disabled={!signedIn}>
            <Plus className="h-4 w-4" />
            New org
          </Button>
        </div>
      </div>

      {!signedIn ? (
        <Card>
          <CardBody>
            <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
              <div>
                <div className="text-sm font-semibold text-slate-100">Sign in required</div>
                <div className="mt-1 text-sm text-slate-300">
                  Paste a session token or use GitHub OAuth to continue.
                </div>
              </div>
              <Button onClick={() => nav("/auth")}>Go to auth</Button>
            </div>
          </CardBody>
        </Card>
      ) : null}

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="text-sm font-semibold text-slate-100">Organizations</div>
            <div className="text-xs text-slate-400">
              {orgs ? `${orgs.length} total` : signedIn ? "Loading…" : "—"}
            </div>
          </div>
        </CardHeader>
        <CardBody>
          {orgs && orgs.length === 0 ? (
            <div className="text-sm text-slate-300">
              No organizations yet. Create one to get started.
            </div>
          ) : null}

          {orgs ? (
            <div className="grid gap-3 md:grid-cols-2">
              {orgs.map((o) => (
                <Link
                  key={o.id}
                  to={`/org/${o.id}`}
                  className="group rounded-2xl border border-white/10 bg-white/5 p-4 transition hover:border-neon-500/30 hover:bg-white/7"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <Building2 className="h-4 w-4 text-neon-300" />
                        <div className="truncate text-sm font-semibold text-slate-100">
                          {o.name}
                        </div>
                      </div>
                      <div className="mt-1 text-xs text-slate-400">Org ID: {o.id}</div>
                    </div>
                    <div className="flex items-center gap-2">
                      <Badge tone={o.role === "admin" ? "neon" : "muted"}>{o.role}</Badge>
                      <ExternalLink className="h-4 w-4 text-slate-500 transition group-hover:text-slate-300" />
                    </div>
                  </div>
                </Link>
              ))}
            </div>
          ) : (
            <div className="text-sm text-slate-300">{signedIn ? "Loading…" : "—"}</div>
          )}
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create organization" onClose={() => setCreateOpen(false)}>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="orgName">Name</Label>
            <Input
              id="orgName"
              value={orgName}
              onChange={(e) => setOrgName(e.target.value)}
              placeholder="Acme Research"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button onClick={createOrg}>Create</Button>
          </div>
        </div>
      </Modal>
    </div>
  );
}

