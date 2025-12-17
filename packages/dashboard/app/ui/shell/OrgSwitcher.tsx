import { Building2, Check, ChevronDown, Plus } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import type { CreateOrganizationResponse, OrganizationEntry } from "../../api/types";
import { useApi } from "../../api/useApi";
import { useOrgs } from "../../org/OrgContext";
import { Button } from "../primitives/Button";
import { Input } from "../primitives/Input";
import { Label } from "../primitives/Label";
import { Modal } from "../primitives/Modal";
import { useToast } from "../toast/ToastProvider";

export function OrgSwitcher() {
  const nav = useNavigate();
  const toast = useToast();
  const { orgId } = useParams();
  const { orgs, loading, refresh } = useOrgs();
  const { request, signedIn } = useApi();
  const [open, setOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [orgName, setOrgName] = useState("");
  const dropdownRef = useRef<HTMLDivElement>(null);

  const currentOrgId = orgId ? Number(orgId) : null;
  const currentOrg = orgs?.find((o) => o.id === currentOrgId) ?? null;

  const sortedOrgs = orgs
    ? [...orgs].sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime())
    : null;

  const handleClickOutside = useCallback((event: MouseEvent) => {
    if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
      setOpen(false);
    }
  }, []);

  useEffect(() => {
    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [open, handleClickOutside]);

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

  return (
    <div className="relative inline-block" ref={dropdownRef}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface-raised px-3 py-2 text-sm text-content-secondary hover:border-border-accent-hover hover:text-content-primary"
      >
        <Building2 className="h-4 w-4" />
        <span className="max-w-48 truncate">
          {loading ? "Loading…" : currentOrg ? currentOrg.name : "Select org"}
        </span>
        <ChevronDown className={`h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 min-w-48 rounded-xl border border-border bg-surface-overlay p-1 shadow-lg shadow-black/50 backdrop-blur-xl">
          {sortedOrgs && sortedOrgs.length > 0 ? (
            <div className="max-h-64 overflow-y-auto">
              {sortedOrgs.map((org) => (
                <Link
                  key={org.id}
                  to={`/org/${org.id}`}
                  onClick={() => setOpen(false)}
                  className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-content-tertiary hover:bg-surface-subtle hover:text-content-primary"
                >
                  <span className="flex-1 truncate">{org.name}</span>
                  {org.id === currentOrgId && <Check className="h-4 w-4 text-accent-text" />}
                </Link>
              ))}
            </div>
          ) : (
            <div className="px-3 py-2 text-sm text-content-muted">
              {loading ? "Loading…" : "No organizations"}
            </div>
          )}

          <div className="mt-1 border-t border-border pt-1">
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                setCreateOpen(true);
              }}
              className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-content-tertiary hover:bg-surface-subtle hover:text-content-primary"
            >
              <Plus className="h-4 w-4" />
              New organization
            </button>
          </div>
        </div>
      )}

      <Modal
        open={createOpen}
        title="Create organization"
        onClose={() => setCreateOpen(false)}
        onSubmit={createOrg}
      >
        <div className="space-y-4">
          <div>
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
