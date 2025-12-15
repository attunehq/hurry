import { Bot, Copy, KeyRound, Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { apiRequest } from "../../api/client";
import type { CreateOrgApiKeyResponse, OrgApiKeyListResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { Modal } from "../../ui/primitives/Modal";
import { useToast } from "../../ui/toast/ToastProvider";
import { useOrgContext } from "./orgContext";

export function OrgApiKeysPage() {
  const toast = useToast();
  const { sessionToken } = useSession();
  const { orgId } = useOrgContext();
  const [data, setData] = useState<OrgApiKeyListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [created, setCreated] = useState<CreateOrgApiKeyResponse | null>(null);

  const keys = useMemo(() => data?.api_keys ?? [], [data]);

  async function load() {
    if (!sessionToken) return;
    setLoading(true);
    try {
      const out = await apiRequest<OrgApiKeyListResponse>({
        path: `/api/v1/organizations/${orgId}/api-keys`,
        sessionToken,
      });
      setData(out);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load API keys", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }

  async function createKey() {
    if (!sessionToken) return;
    const n = name.trim();
    if (!n) {
      toast.push({ kind: "error", title: "Name required" });
      return;
    }
    try {
      const out = await apiRequest<CreateOrgApiKeyResponse>({
        path: `/api/v1/organizations/${orgId}/api-keys`,
        method: "POST",
        sessionToken,
        body: { name: n },
      });
      setCreated(out);
      setName("");
      await load();
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  async function revoke(keyId: number) {
    if (!sessionToken) return;
    if (!confirm(`Revoke API key ${keyId}?`)) return;
    try {
      await apiRequest<void>({
        path: `/api/v1/organizations/${orgId}/api-keys/${keyId}`,
        method: "DELETE",
        sessionToken,
      });
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

  useEffect(() => {
    void load();
  }, [sessionToken, orgId]);

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <div className="text-sm font-semibold text-content-primary">API Keys</div>
              <div className="mt-1 text-sm text-content-tertiary">
                Keys authenticate builds and automation against Hurry.
              </div>
            </div>
            <Button onClick={() => setCreateOpen(true)} disabled={!sessionToken}>
              <Plus className="h-4 w-4" />
              New key
            </Button>
          </div>
        </CardHeader>
        <CardBody>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-content-muted">
                <tr className="border-b border-border">
                  <th className="py-2 pr-3">Key</th>
                  <th className="py-2 pr-3">Owner</th>
                  <th className="py-2 pr-3">Last used</th>
                  <th className="py-2 pr-3"></th>
                </tr>
              </thead>
              <tbody>
                {keys.map((k) => (
                  <tr key={k.id} className="border-b border-border-subtle">
                    <td className="py-3 pr-3">
                      <div className="flex items-center gap-2 font-medium text-content-primary">
                        <KeyRound className="h-4 w-4 text-accent-text" />
                        {k.name}
                      </div>
                    </td>
                    <td className="py-3 pr-3 text-content-secondary">
                      <div className="flex items-center gap-2">
                        {k.bot ? <Bot className="h-4 w-4 text-content-muted" /> : null}
                        {k.account_email}
                      </div>
                    </td>
                    <td className="py-3 pr-3 text-xs text-content-tertiary">{k.accessed_at}</td>
                    <td className="py-3 pr-3">
                      <div className="flex justify-end">
                        <Button variant="danger" size="sm" onClick={() => revoke(k.id)}>
                          <Trash2 className="h-4 w-4" />
                          Revoke
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
                {keys.length === 0 && !loading ? (
                  <tr>
                    <td colSpan={4} className="py-6 text-center text-sm text-content-muted">
                      No API keys yet.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-xs text-content-muted">
            Note: API key tokens are only shown at creation time.
          </div>
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create API key" onClose={() => setCreateOpen(false)} onSubmit={createKey}>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="keyName">Name</Label>
            <Input
              id="keyName"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="ci-key"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button onClick={createKey}>Create</Button>
          </div>
        </div>
      </Modal>

      <Modal open={Boolean(created)} title="API key token (save now)" onClose={() => setCreated(null)}>
        {created ? (
          <div className="space-y-3">
            <div className="text-sm text-content-tertiary">
              This token is only shown once. Copy it somewhere safe.
            </div>
            <div className="rounded-2xl border border-border bg-surface-subtle p-4">
              <div className="text-xs text-content-muted">Token</div>
              <div className="mt-1 break-all font-mono text-xs text-content-primary">
                {created.token}
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => copy(created.token)}>
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
