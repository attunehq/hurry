import { Building2, ExternalLink, Plus } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";

import type {
  CreateOrganizationResponse,
  MeResponse,
  OrganizationEntry,
  OrganizationListResponse,
} from "../api/types";
import { useApi } from "../api/useApi";
import { Badge } from "../ui/primitives/Badge";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { Modal } from "../ui/primitives/Modal";
import { PageLayout } from "../ui/shell/PageLayout";
import { useToast } from "../ui/toast/ToastProvider";

export default function DashboardHome() {
  const nav = useNavigate();
  const toast = useToast();
  const { request, signedIn } = useApi();
  const [me, setMe] = useState<MeResponse | null>(null);
  const [orgs, setOrgs] = useState<OrganizationEntry[] | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [orgName, setOrgName] = useState("");

  const headerLine = useMemo(() => {
    if (!me) return "Hurry Console";
    const who = me.name?.trim() ? me.name.trim() : me.github_username ?? me.email;
    return `Welcome, ${who}`;
  }, [me]);

  const sortedOrgs = useMemo(() => {
    if (!orgs) return null;
    return [...orgs].sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime());
  }, [orgs]);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setMe(null);
      setOrgs(null);
      return;
    }
    try {
      const meOut = await request<MeResponse>({ path: "/api/v1/me" });
      const orgsOut = await request<OrganizationListResponse>({ path: "/api/v1/me/organizations" });
      setMe(meOut);
      setOrgs(orgsOut.organizations);
    } catch (e) {
      // Don't show error toast for 401 - handled by session invalidation
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      setMe(null);
      setOrgs(null);
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load", detail: msg });
    }
  }, [signedIn, request, toast]);

  async function createOrg() {
    if (!signedIn) {
      toast.push({ kind: "error", title: "Sign in first" });
      nav("/auth");
      return;
    }
    const name = orgName.trim();
    if (!name) {
      toast.push({ kind: "error", title: "Organization name required" });
      return;
    }
    setCreateOpen(false);
    try {
      const created = await request<CreateOrganizationResponse>({
        path: "/api/v1/organizations",
        method: "POST",
        body: { name },
      });
      setOrgName("");
      await refresh();
      nav(`/org/${created.id}`);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <PageLayout
      title={headerLine}
      subtitle="Manage organizations, invitations, API keys, and bot identities."
      actions={
        <Button onClick={() => setCreateOpen(true)} disabled={!signedIn}>
          <Plus className="h-4 w-4" />
          New org
        </Button>
      }
    >
      {!signedIn ? (
        <Card>
          <CardBody>
            <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
              <div>
                <div className="text-sm font-semibold text-content-primary">Sign in required</div>
                <div className="mt-1 text-sm text-content-tertiary">
                  Sign in with GitHub to continue.
                </div>
              </div>
              <Button onClick={() => nav("/auth")}>Sign in</Button>
            </div>
          </CardBody>
        </Card>
      ) : null}

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="text-sm font-semibold text-content-primary">Organizations</div>
            <div className="text-xs text-content-muted">
              {orgs ? `${orgs.length} total` : signedIn ? "Loading…" : "—"}
            </div>
          </div>
        </CardHeader>
        <CardBody>
          {sortedOrgs && sortedOrgs.length === 0 ? (
            <div className="text-sm text-content-tertiary">
              No organizations yet. Create one to get started.
            </div>
          ) : null}

          {sortedOrgs ? (
            <div className="space-y-3">
              {sortedOrgs.map((o) => (
                <Link
                  key={o.id}
                  to={`/org/${o.id}`}
                  className="group flex items-center justify-between rounded-2xl border border-border bg-surface-subtle p-5 transition hover:border-border-accent-hover hover:bg-surface-subtle-hover"
                >
                  <div className="flex items-center gap-3">
                    <Building2 className="h-5 w-5 text-accent-text" />
                    <div>
                      <div className="text-base font-semibold text-content-primary">{o.name}</div>
                      <div className="mt-0.5 text-xs text-content-muted">
                        Created {new Date(o.created_at).toLocaleDateString()}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center gap-3">
                    <Badge tone={o.role === "admin" ? "neon" : "muted"}>{o.role}</Badge>
                    <ExternalLink className="h-4 w-4 text-content-muted transition group-hover:text-content-tertiary" />
                  </div>
                </Link>
              ))}
            </div>
          ) : (
            <div className="text-sm text-content-tertiary">{signedIn ? "Loading…" : "—"}</div>
          )}
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create organization" onClose={() => setCreateOpen(false)} onSubmit={createOrg}>
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
    </PageLayout>
  );
}
