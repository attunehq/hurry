import clsx from "clsx";
import { Pencil } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useNavigate, useOutletContext, useParams } from "react-router";

import type { OrganizationEntry, OrganizationListResponse, OrgRole } from "../api/types";
import { useApi } from "../api/useApi";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody } from "../ui/primitives/Card";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { Modal } from "../ui/primitives/Modal";
import { PageLayout } from "../ui/shell/PageLayout";
import { useToast } from "../ui/toast/ToastProvider";

export type OrgOutletContext = {
  orgId: number;
  role: OrgRole | null;
};

export function useOrgContext() {
  return useOutletContext<OrgOutletContext>();
}

export default function OrgLayout() {
  const nav = useNavigate();
  const toast = useToast();
  const { orgId } = useParams();
  const { request, signedIn } = useApi();
  const [org, setOrg] = useState<OrganizationEntry | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [newName, setNewName] = useState("");

  const id = useMemo(() => Number(orgId ?? "0"), [orgId]);

  const refresh = useCallback(async () => {
    if (!signedIn || !id) return;
    try {
      const out = await request<OrganizationListResponse>({
        path: "/api/v1/me/organizations",
      });
      const found = out.organizations.find((o) => o.id === id) ?? null;
      setOrg(found);
      if (!found) toast.push({ kind: "error", title: "Org not found (or no access)" });
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load org", detail: msg });
    }
  }, [signedIn, id, request, toast]);

  const canAdmin = org?.role === "admin";

  function openRename() {
    setNewName(org?.name ?? "");
    setRenameOpen(true);
  }

  async function rename() {
    if (!signedIn || !id) return;
    const trimmed = newName.trim();
    if (!trimmed) {
      toast.push({ kind: "error", title: "Name cannot be empty" });
      return;
    }
    setRenameOpen(false);
    try {
      await request<void>({
        path: `/api/v1/organizations/${id}`,
        method: "PATCH",
        body: { name: trimmed },
      });
      toast.push({ kind: "success", title: "Organization renamed" });
      await refresh();
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Rename failed", detail: msg });
    }
  }

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!signedIn) {
    return (
      <PageLayout title="Organization">
        <Card>
          <CardBody>
            <div className="flex items-center justify-between">
              <div className="text-sm text-content-tertiary">Sign in to view this organization.</div>
              <Button onClick={() => nav("/auth")} variant="secondary">
                Sign in
              </Button>
            </div>
          </CardBody>
        </Card>
      </PageLayout>
    );
  }

  return (
    <PageLayout
      title={
        <span className="flex items-center gap-2">
          {org ? org.name : "Organization"}
          {canAdmin && (
            <button
              type="button"
              onClick={openRename}
              className="cursor-pointer text-content-muted hover:text-content-primary"
              title="Rename organization"
            >
              <Pencil className="h-4 w-4" />
            </button>
          )}
        </span>
      }
    >
      <div className="mt-6 rounded-2xl border border-border bg-surface-raised p-2 shadow-glow-soft backdrop-blur">
        <div className="flex flex-wrap gap-1">
          <Tab to="" label="Overview" end />
          <Tab to="members" label="Members" />
          <Tab to="api-keys" label="API Keys" />
          <Tab to="invitations" label="Invitations" />
          <Tab to="bots" label="Bots" />
          <Tab to="billing" label="Billing" />
        </div>
      </div>

      <div className="tab-content">
        <Outlet context={{ orgId: id, role: org?.role ?? null }} />
      </div>

      <Modal open={renameOpen} title="Rename organization" onClose={() => setRenameOpen(false)} onSubmit={rename}>
        <div className="space-y-4">
          <div>
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
    </PageLayout>
  );
}

function Tab(props: { to: string; label: string; end?: boolean }) {
  return (
    <NavLink
      to={props.to}
      end={props.end}
      viewTransition
      className={({ isActive }) =>
        clsx(
          "cursor-pointer rounded-xl px-3 py-2 text-sm transition",
          isActive ? "bg-surface-subtle text-content-primary" : "text-content-tertiary hover:bg-surface-subtle hover:text-content-primary",
        )
      }
    >
      {props.label}
    </NavLink>
  );
}
