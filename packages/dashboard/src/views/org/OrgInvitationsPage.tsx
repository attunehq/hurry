import { Copy, Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { apiRequest } from "../../api/client";
import type { CreateInvitationResponse, InvitationListResponse, OrgRole } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { Modal } from "../../ui/primitives/Modal";
import { useToast } from "../../ui/toast/ToastProvider";
import { useOrgContext } from "./orgContext";

export function OrgInvitationsPage() {
  const toast = useToast();
  const { sessionToken } = useSession();
  const { orgId, role } = useOrgContext();
  const [data, setData] = useState<InvitationListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [created, setCreated] = useState<CreateInvitationResponse | null>(null);

  const [inviteRole, setInviteRole] = useState<OrgRole>("member");
  const [maxUses, setMaxUses] = useState<string>("");

  const canAdmin = role === "admin";
  const invites = useMemo(() => data?.invitations ?? [], [data]);

  async function load() {
    if (!sessionToken) return;
    setLoading(true);
    try {
      const out = await apiRequest<InvitationListResponse>({
        path: `/api/v1/organizations/${orgId}/invitations`,
        sessionToken,
      });
      setData(out);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load invitations", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }

  async function createInvite() {
    if (!sessionToken) return;
    const max =
      maxUses.trim().length === 0 ? undefined : Number.isFinite(Number(maxUses)) ? Number(maxUses) : NaN;
    if (max === 0 || Number.isNaN(max)) {
      toast.push({ kind: "error", title: "max_uses must be a number ≥ 1" });
      return;
    }

    try {
      const out = await apiRequest<CreateInvitationResponse>({
        path: `/api/v1/organizations/${orgId}/invitations`,
        method: "POST",
        sessionToken,
        body: { role: inviteRole, ...(max ? { max_uses: max } : {}) },
      });
      setCreated(out);
      setCreateOpen(false);
      setMaxUses("");
      toast.push({ kind: "success", title: "Invitation created" });
      await load();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  async function revoke(invitationId: number) {
    if (!sessionToken) return;
    if (!confirm(`Revoke invitation ${invitationId}?`)) return;
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${orgId}/invitations/${invitationId}`,
        method: "DELETE",
        sessionToken,
      });
      toast.push({ kind: "success", title: "Invitation revoked" });
      await load();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Revoke failed", detail: msg });
    }
  }

  async function copy(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      toast.push({ kind: "success", title: "Copied" });
    } catch {
      toast.push({ kind: "error", title: "Copy failed" });
    }
  }

  function inviteLink(token: string) {
    return `${window.location.origin}/invite/${token}`;
  }

  useEffect(() => {
    void load();
  }, [sessionToken, orgId]);

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <div className="text-sm font-semibold text-slate-100">Invitations</div>
              <div className="mt-1 text-sm text-slate-300">
                Admins can generate shareable links for members to join.
              </div>
            </div>
            <Button onClick={() => setCreateOpen(true)} disabled={!canAdmin}>
              <Plus className="h-4 w-4" />
              New invite
            </Button>
          </div>
        </CardHeader>
        <CardBody>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-slate-400">
                <tr className="border-b border-white/10">
                  <th className="py-2 pr-3">ID</th>
                  <th className="py-2 pr-3">Role</th>
                  <th className="py-2 pr-3">Uses</th>
                  <th className="py-2 pr-3">Status</th>
                  <th className="py-2 pr-3"></th>
                </tr>
              </thead>
              <tbody>
                {invites.map((i) => (
                  <tr key={i.id} className="border-b border-white/5">
                    <td className="py-3 pr-3 font-medium text-slate-100">{i.id}</td>
                    <td className="py-3 pr-3">
                      <Badge tone={i.role === "admin" ? "neon" : "muted"}>{i.role}</Badge>
                    </td>
                    <td className="py-3 pr-3 text-slate-200">
                      {i.use_count}
                      <span className="text-slate-500">
                        {i.max_uses ? ` / ${i.max_uses}` : " / ∞"}
                      </span>
                    </td>
                    <td className="py-3 pr-3">
                      {i.revoked ? <Badge tone="warn">revoked</Badge> : <Badge>active</Badge>}
                    </td>
                    <td className="py-3 pr-3">
                      <div className="flex justify-end gap-2">
                        <Button
                          variant="danger"
                          size="sm"
                          disabled={!canAdmin || i.revoked}
                          onClick={() => revoke(i.id)}
                        >
                          <Trash2 className="h-4 w-4" />
                          Revoke
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
                {invites.length === 0 && !loading ? (
                  <tr>
                    <td colSpan={5} className="py-6 text-center text-sm text-slate-400">
                      No invitations yet.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-xs text-slate-400">
            Note: listing invitations intentionally does not reveal tokens; you only get tokens at
            creation time.
          </div>
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create invitation" onClose={() => setCreateOpen(false)}>
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="role">Role</Label>
              <select
                id="role"
                className="h-10 w-full rounded-xl border border-white/10 bg-white/5 px-3 text-sm text-slate-100"
                value={inviteRole}
                onChange={(e) => setInviteRole(e.target.value as OrgRole)}
              >
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="maxUses">Max uses (optional)</Label>
              <Input
                id="maxUses"
                value={maxUses}
                onChange={(e) => setMaxUses(e.target.value)}
                placeholder="e.g. 5"
              />
            </div>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button onClick={createInvite} disabled={!canAdmin}>
              Create
            </Button>
          </div>
        </div>
      </Modal>

      <Modal
        open={Boolean(created)}
        title="Invitation token (shareable link)"
        onClose={() => setCreated(null)}
      >
        {created ? (
          <div className="space-y-3">
            <div className="text-sm text-slate-300">
              Share this link to invite someone. The token is embedded.
            </div>
            <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
              <div className="text-xs text-slate-400">Invite link</div>
              <div className="mt-1 break-all font-mono text-xs text-slate-100">
                {inviteLink(created.token)}
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => copy(inviteLink(created.token))}>
                <Copy className="h-4 w-4" />
                Copy
              </Button>
              <Button onClick={() => setCreated(null)}>Done</Button>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}
