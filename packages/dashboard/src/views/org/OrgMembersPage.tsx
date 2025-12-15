import { Bot, Crown, DoorOpen, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";

import type { MeResponse, MemberListResponse, OrgRole } from "../../api/types";
import { useApi } from "../../api/useApi";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { useToast } from "../../ui/toast/ToastProvider";
import { useOrgContext } from "./orgContext";

export function OrgMembersPage() {
  const nav = useNavigate();
  const toast = useToast();
  const { request, signedIn } = useApi();
  const { orgId, role } = useOrgContext();
  const [members, setMembers] = useState<MemberListResponse | null>(null);
  const [me, setMe] = useState<MeResponse | null>(null);
  const [loading, setLoading] = useState(false);

  const canAdmin = role === "admin";
  const adminCount = useMemo(
    () => members?.members.filter((m) => m.role === "admin" && !m.bot).length ?? 0,
    [members]
  );
  const isOnlyAdmin = role === "admin" && adminCount === 1;

  async function load() {
    if (!signedIn) return;
    setLoading(true);
    try {
      const [membersOut, meOut] = await Promise.all([
        request<MemberListResponse>({
          path: `/api/v1/organizations/${orgId}/members`,
        }),
        request<MeResponse>({ path: "/api/v1/me" }),
      ]);
      setMembers(membersOut);
      setMe(meOut);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as any).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load members", detail: msg });
      setMembers(null);
    } finally {
      setLoading(false);
    }
  }

  async function setRole(accountId: number, newRole: OrgRole) {
    if (!signedIn) return;
    try {
      await request<void>({
        path: `/api/v1/organizations/${orgId}/members/${accountId}`,
        method: "PATCH",
        body: { role: newRole },
      });
      await load();
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as any).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Update failed", detail: msg });
    }
  }

  async function remove(accountId: number) {
    if (!signedIn) return;
    if (!confirm(`Remove member ${accountId}?`)) return;
    try {
      await request<void>({
        path: `/api/v1/organizations/${orgId}/members/${accountId}`,
        method: "DELETE",
      });
      await load();
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as any).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Remove failed", detail: msg });
    }
  }

  async function leave() {
    if (!signedIn) return;
    if (!confirm("Leave this organization?")) return;
    try {
      await request<void>({
        path: `/api/v1/organizations/${orgId}/leave`,
        method: "POST",
      });
      nav("/");
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as any).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Leave failed", detail: msg });
    }
  }

  const rows = useMemo(() => {
    const list = members?.members ?? [];
    return [...list].sort((a, b) => Number(a.bot) - Number(b.bot));
  }, [members]);

  useEffect(() => {
    void load();
  }, [signedIn, orgId]);

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm font-semibold text-content-primary">Members</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Manage who has access to this organization.
            </div>
          </div>
          <div className="text-xs text-content-muted">{loading ? "Loading…" : `${rows.length} total`}</div>
        </div>
      </CardHeader>
      <CardBody>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="text-xs text-content-muted">
              <tr className="border-b border-border">
                <th className="py-2 pr-3">Name</th>
                <th className="py-2 pr-3">Email</th>
                <th className="py-2 pr-3">Role</th>
                <th className="py-2 pr-3"></th>
              </tr>
            </thead>
            <tbody>
              {rows.map((m) => (
                <tr key={m.account_id} className="border-b border-border-subtle">
                  <td className="py-3 pr-3">
                    <div className="flex items-center gap-2 font-medium text-content-primary">
                      {m.bot ? <Bot className="h-4 w-4 text-accent-text" /> : null}
                      {m.name ?? m.email}
                    </div>
                  </td>
                  <td className="py-3 pr-3 text-content-secondary">{m.email}</td>
                  <td className="py-3 pr-3">
                    <div className="flex items-center gap-2">
                      <Badge tone={m.role === "admin" ? "neon" : "muted"}>{m.role}</Badge>
                      {m.bot ? <Badge tone="muted">bot</Badge> : null}
                    </div>
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
                      {m.account_id === me?.id && isOnlyAdmin ? (
                        <div
                          className="relative"
                          title="You're the only admin. Promote another member before demoting yourself."
                        >
                          <Button variant="secondary" size="sm" disabled>
                            Demote
                          </Button>
                        </div>
                      ) : (
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={!canAdmin || m.role === "member"}
                          onClick={() => setRole(m.account_id, "member")}
                        >
                          Demote
                        </Button>
                      )}
                      {m.account_id === me?.id ? (
                        <div
                          className="relative"
                          title={isOnlyAdmin ? "You're the only admin. Promote another member before leaving." : undefined}
                        >
                          <Button variant="danger" size="sm" disabled={isOnlyAdmin} onClick={leave}>
                            <DoorOpen className="h-4 w-4" />
                            Leave
                          </Button>
                        </div>
                      ) : (
                        <Button
                          variant="danger"
                          size="sm"
                          disabled={!canAdmin}
                          onClick={() => remove(m.account_id)}
                        >
                          <Trash2 className="h-4 w-4" />
                          Remove
                        </Button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
              {rows.length === 0 && !loading ? (
                <tr>
                  <td colSpan={4} className="py-6 text-center text-sm text-content-muted">
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
