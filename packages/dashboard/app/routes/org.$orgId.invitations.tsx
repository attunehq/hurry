import { Plus, Ticket, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { isUnauthorizedError } from "../api/client";
import type { CreateInvitationResponse, InvitationListResponse, OrgRole } from "../api/types";
import { useApi } from "../api/useApi";
import { Badge } from "../ui/primitives/Badge";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { CodeBlock } from "../ui/primitives/CodeBlock";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { Modal } from "../ui/primitives/Modal";
import { useToast } from "../ui/toast/ToastProvider";
import { useOrgContext } from "./org.$orgId";

export default function OrgInvitationsPage() {
  const toast = useToast();
  const { request, signedIn } = useApi();
  const { orgId, role } = useOrgContext();
  const [data, setData] = useState<InvitationListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [created, setCreated] = useState<CreateInvitationResponse | null>(null);

  const [inviteRole, setInviteRole] = useState<OrgRole>("member");
  const [maxUses, setMaxUses] = useState<string>("");

  const canAdmin = role === "admin";
  const invites = useMemo(() => data?.invitations ?? [], [data]);

  const load = useCallback(async () => {
    if (!signedIn) return;
    setLoading(true);
    try {
      const out = await request<InvitationListResponse>({
        path: `/api/v1/organizations/${orgId}/invitations`,
      });
      setData(out);
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 403) {
        setData(null);
        return;
      }
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load invitations", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [signedIn, orgId, request, toast]);

  async function createInvite() {
    if (!signedIn) return;
    const max =
      maxUses.trim().length === 0 ? undefined : Number.isFinite(Number(maxUses)) ? Number(maxUses) : NaN;
    if (max === 0 || Number.isNaN(max)) {
      toast.push({ kind: "error", title: "max_uses must be a number ≥ 1" });
      return;
    }
    setCreateOpen(false);
    try {
      const out = await request<CreateInvitationResponse>({
        path: `/api/v1/organizations/${orgId}/invitations`,
        method: "POST",
        body: { role: inviteRole, ...(max ? { max_uses: max } : {}) },
      });
      setCreated(out);
      setMaxUses("");
      await load();
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  async function revoke(invitationId: number) {
    if (!signedIn) return;
    if (!confirm(`Revoke invitation ${invitationId}?`)) return;
    try {
      await request<void>({
        path: `/api/v1/organizations/${orgId}/invitations/${invitationId}`,
        method: "DELETE",
      });
      await load();
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Revoke failed", detail: msg });
    }
  }

  function inviteLink(token: string) {
    return `${window.location.origin}/invite/${token}`;
  }

  useEffect(() => {
    void load();
  }, [load]);

  if (!canAdmin) {
    return (
      <Card>
        <CardBody>
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <Ticket className="mb-4 h-12 w-12 text-content-muted" />
            <div className="text-sm font-medium text-content-primary">Admin access required</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Only organization admins can manage invitations.
            </div>
          </div>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <div className="text-sm font-semibold text-content-primary">Invitations</div>
              <div className="mt-1 text-sm text-content-tertiary">
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
              <thead className="text-xs text-content-muted">
                <tr className="border-b border-border">
                  <th className="py-2 pr-3">ID</th>
                  <th className="py-2 pr-3">Role</th>
                  <th className="py-2 pr-3">Uses</th>
                  <th className="py-2 pr-3">Status</th>
                  <th className="py-2 pr-3"></th>
                </tr>
              </thead>
              <tbody>
                {invites.map((i) => (
                  <tr key={i.id} className="border-b border-border-subtle">
                    <td className="py-3 pr-3 font-medium text-content-primary">{i.id}</td>
                    <td className="py-3 pr-3">
                      <Badge tone={i.role === "admin" ? "neon" : "muted"}>{i.role}</Badge>
                    </td>
                    <td className="py-3 pr-3 text-content-secondary">
                      {i.use_count}
                      <span className="text-content-muted">
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
                    <td colSpan={5} className="py-6 text-center text-sm text-content-muted">
                      No invitations yet.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-xs text-content-muted">
            Note: Invitation tokens are only shown at creation time.
          </div>
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create invitation" onClose={() => setCreateOpen(false)} onSubmit={createInvite}>
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <Label htmlFor="role">Role</Label>
              <select
                id="role"
                className="h-10 w-full cursor-pointer rounded-xl border border-border bg-surface-subtle px-3 text-sm text-content-primary focus:border-border-accent-hover focus:bg-surface-subtle-hover focus:outline-none"
                value={inviteRole}
                onChange={(e) => setInviteRole(e.target.value as OrgRole)}
              >
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
            </div>
            <div>
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
            <div className="text-sm text-content-tertiary">
              Share this link to invite someone. The token is embedded.
            </div>
            <CodeBlock code={inviteLink(created.token)} label="Invite link" />
            <div className="flex justify-end">
              <Button onClick={() => setCreated(null)}>Done</Button>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}
