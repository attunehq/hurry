import { Crown, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { apiRequest } from "../../api/client";
import type { MemberListResponse, OrgRole } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { useToast } from "../../ui/toast/ToastProvider";
import { useOrgContext } from "./orgContext";

export function OrgMembersPage() {
  const toast = useToast();
  const { sessionToken } = useSession();
  const { orgId, role } = useOrgContext();
  const [members, setMembers] = useState<MemberListResponse | null>(null);
  const [loading, setLoading] = useState(false);

  const canAdmin = role === "admin";

  async function load() {
    if (!sessionToken) return;
    setLoading(true);
    try {
      const out = await apiRequest<MemberListResponse>({
        path: `/api/v1/organizations/${orgId}/members`,
        sessionToken,
      });
      setMembers(out);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load members", detail: msg });
      setMembers(null);
    } finally {
      setLoading(false);
    }
  }

  async function setRole(accountId: number, newRole: OrgRole) {
    if (!sessionToken) return;
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${orgId}/members/${accountId}`,
        method: "PATCH",
        sessionToken,
        body: { role: newRole },
      });
      toast.push({ kind: "success", title: "Role updated", detail: `${accountId} → ${newRole}` });
      await load();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Update failed", detail: msg });
    }
  }

  async function remove(accountId: number) {
    if (!sessionToken) return;
    if (!confirm(`Remove member ${accountId}?`)) return;
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${orgId}/members/${accountId}`,
        method: "DELETE",
        sessionToken,
      });
      toast.push({ kind: "success", title: "Member removed" });
      await load();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Remove failed", detail: msg });
    }
  }

  const rows = useMemo(() => members?.members ?? [], [members]);

  useEffect(() => {
    void load();
  }, [sessionToken, orgId]);

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm font-semibold text-slate-100">Members</div>
            <div className="mt-1 text-sm text-slate-300">
              Membership + roles are managed inside Courier.
            </div>
          </div>
          <div className="text-xs text-slate-400">{loading ? "Loading…" : `${rows.length} total`}</div>
        </div>
      </CardHeader>
      <CardBody>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="text-xs text-slate-400">
              <tr className="border-b border-white/10">
                <th className="py-2 pr-3">Account</th>
                <th className="py-2 pr-3">Email</th>
                <th className="py-2 pr-3">Role</th>
                <th className="py-2 pr-3"></th>
              </tr>
            </thead>
            <tbody>
              {rows.map((m) => (
                <tr key={m.account_id} className="border-b border-white/5">
                  <td className="py-3 pr-3">
                    <div className="font-medium text-slate-100">{m.account_id}</div>
                    {m.name ? <div className="text-xs text-slate-400">{m.name}</div> : null}
                  </td>
                  <td className="py-3 pr-3 text-slate-200">{m.email}</td>
                  <td className="py-3 pr-3">
                    <Badge tone={m.role === "admin" ? "neon" : "muted"}>{m.role}</Badge>
                  </td>
                  <td className="py-3 pr-3">
                    <div className="flex justify-end gap-2">
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={!canAdmin || m.role === "admin"}
                        onClick={() => setRole(m.account_id, "admin")}
                      >
                        <Crown className="h-4 w-4" />
                        Promote
                      </Button>
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={!canAdmin || m.role === "member"}
                        onClick={() => setRole(m.account_id, "member")}
                      >
                        Demote
                      </Button>
                      <Button
                        variant="danger"
                        size="sm"
                        disabled={!canAdmin}
                        onClick={() => remove(m.account_id)}
                      >
                        <Trash2 className="h-4 w-4" />
                        Remove
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
              {rows.length === 0 && !loading ? (
                <tr>
                  <td colSpan={4} className="py-6 text-center text-sm text-slate-400">
                    No members found.
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      </CardBody>
    </Card>
  );
}

