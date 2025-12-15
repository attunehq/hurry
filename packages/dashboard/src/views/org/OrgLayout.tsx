import { Pencil } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useNavigate, useParams } from "react-router-dom";

import { apiRequest } from "../../api/client";
import type { OrganizationEntry, OrganizationListResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { Modal } from "../../ui/primitives/Modal";
import { useToast } from "../../ui/toast/ToastProvider";

export function OrgLayout() {
  const nav = useNavigate();
  const toast = useToast();
  const { orgId } = useParams();
  const { sessionToken } = useSession();
  const [org, setOrg] = useState<OrganizationEntry | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [newName, setNewName] = useState("");

  const id = useMemo(() => Number(orgId ?? "0"), [orgId]);

  async function refresh() {
    if (!sessionToken || !id) return;
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
    }
  }

  const canAdmin = org?.role === "admin";

  function openRename() {
    setNewName(org?.name ?? "");
    setRenameOpen(true);
  }

  async function rename() {
    if (!sessionToken || !id) return;
    const trimmed = newName.trim();
    if (!trimmed) {
      toast.push({ kind: "error", title: "Name cannot be empty" });
      return;
    }
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${id}`,
        method: "PATCH",
        sessionToken,
        body: { name: trimmed },
      });
      toast.push({ kind: "success", title: "Organization renamed" });
      setRenameOpen(false);
      await refresh();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Rename failed", detail: msg });
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
            <div className="text-sm text-content-tertiary">Sign in to view this organization.</div>
            <Button onClick={() => nav("/auth")} variant="secondary">
              Go to auth
            </Button>
          </div>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-start justify-between gap-3 md:flex-row md:items-center">
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-semibold text-content-primary">
            {org ? org.name : "Organization"}
          </h1>
          {org ? (
            <Badge tone={org.role === "admin" ? "neon" : "muted"}>{org.role}</Badge>
          ) : null}
        </div>
        <Button variant="secondary" onClick={openRename} disabled={!canAdmin}>
          <Pencil className="h-4 w-4" />
          Rename
        </Button>
      </div>

      <div className="rounded-2xl border border-border bg-surface-raised p-2 shadow-glow-soft backdrop-blur">
        <div className="flex flex-wrap gap-1">
          <Tab to="members" label="Members" />
          <Tab to="api-keys" label="API Keys" />
          <Tab to="invitations" label="Invitations" />
          <Tab to="bots" label="Bots" />
        </div>
      </div>

      <Outlet context={{ orgId: id, role: org?.role ?? null }} />

      <Modal open={renameOpen} title="Rename organization" onClose={() => setRenameOpen(false)} onSubmit={rename}>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="org-name">Organization name</Label>
            <Input
              id="org-name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="Enter new name"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setRenameOpen(false)}>
              Cancel
            </Button>
            <Button onClick={rename}>
              Rename
            </Button>
          </div>
        </div>
      </Modal>
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
          isActive ? "bg-surface-subtle text-content-primary" : "text-content-tertiary hover:bg-surface-subtle hover:text-content-primary",
        ].join(" ")
      }
    >
      {props.label}
    </NavLink>
  );
}
