import { Bot, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { isUnauthorizedError } from "../api/client";
import type { BotListResponse, CreateBotResponse } from "../api/types";
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

export default function OrgBotsPage() {
  const toast = useToast();
  const { request, signedIn } = useApi();
  const { orgId, role } = useOrgContext();
  const [data, setData] = useState<BotListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);

  const [botName, setBotName] = useState("");
  const [responsibleEmail, setResponsibleEmail] = useState("");
  const [created, setCreated] = useState<CreateBotResponse | null>(null);

  const bots = useMemo(() => data?.bots ?? [], [data]);
  const canAdmin = role === "admin";

  const load = useCallback(async () => {
    if (!signedIn) return;
    setLoading(true);
    try {
      const out = await request<BotListResponse>({
        path: `/api/v1/organizations/${orgId}/bots`,
      });
      setData(out);
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 403) {
        setData(null);
        return;
      }
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load bots", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [signedIn, orgId, request, toast]);

  async function createBot() {
    if (!signedIn) return;
    const n = botName.trim();
    const e = responsibleEmail.trim();
    if (!n || !e) {
      toast.push({ kind: "error", title: "Name and responsible email required" });
      return;
    }
    setCreateOpen(false);
    try {
      const out = await request<CreateBotResponse>({
        path: `/api/v1/organizations/${orgId}/bots`,
        method: "POST",
        body: { name: n, responsible_email: e },
      });
      setCreated(out);
      setBotName("");
      setResponsibleEmail("");
      await load();
    } catch (err) {
      if (isUnauthorizedError(err)) return;
      const msg =
        err && typeof err === "object" && "message" in err ? String((err as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
    }
  }

  async function revokeBot(accountId: number, name: string | null) {
    if (!signedIn) return;
    if (!confirm(`Revoke bot "${name ?? "Unnamed bot"}"? This removes the bot from the organization.`))
      return;
    try {
      await request<void>({
        path: `/api/v1/organizations/${orgId}/members/${accountId}`,
        method: "DELETE",
      });
      await load();
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Revoke failed", detail: msg });
    }
  }

  useEffect(() => {
    void load();
  }, [load]);

  if (!canAdmin) {
    return (
      <Card>
        <CardBody>
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <Bot className="mb-4 h-12 w-12 text-content-muted" />
            <div className="text-sm font-medium text-content-primary">Admin access required</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Only organization admins can manage bots.
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
              <div className="text-sm font-semibold text-content-primary">Bots</div>
              <div className="mt-1 text-sm text-content-tertiary">
                Machine accounts for automated workflows.
              </div>
            </div>
            <Button onClick={() => setCreateOpen(true)} disabled={!canAdmin}>
              <Plus className="h-4 w-4" />
              New bot
            </Button>
          </div>
        </CardHeader>
        <CardBody>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-content-muted">
                <tr className="border-b border-border">
                  <th className="py-2 pr-3">Bot</th>
                  <th className="py-2 pr-3">Responsible</th>
                  <th className="py-2 pr-3">Created</th>
                  <th className="py-2 pr-3"></th>
                </tr>
              </thead>
              <tbody>
                {bots.map((b) => (
                  <tr key={b.account_id} className="border-b border-border-subtle">
                    <td className="py-3 pr-3">
                      <div className="flex items-center gap-2 font-medium text-content-primary">
                        <Bot className="h-4 w-4 text-accent-text" />
                        {b.name ?? "Unnamed bot"}
                      </div>
                    </td>
                    <td className="py-3 pr-3 text-content-secondary">{b.responsible_email}</td>
                    <td className="py-3 pr-3 text-xs text-content-tertiary">{b.created_at}</td>
                    <td className="py-3 pr-3">
                      <div className="flex justify-end">
                        <Button
                          variant="danger"
                          size="sm"
                          disabled={!canAdmin}
                          onClick={() => revokeBot(b.account_id, b.name ?? null)}
                        >
                          <Trash2 className="h-4 w-4" />
                          Revoke
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
                {bots.length === 0 && !loading ? (
                  <tr>
                    <td colSpan={4} className="py-6 text-center text-sm text-content-muted">
                      No bots yet.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-xs text-content-muted">
            Note: Bot API keys are only shown at creation time.
          </div>
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create bot" onClose={() => setCreateOpen(false)} onSubmit={createBot}>
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <Label htmlFor="botName">Name</Label>
              <Input
                id="botName"
                value={botName}
                onChange={(e) => setBotName(e.target.value)}
                placeholder="CI Bot"
              />
            </div>
            <div>
              <Label htmlFor="email">Responsible email</Label>
              <Input
                id="email"
                value={responsibleEmail}
                onChange={(e) => setResponsibleEmail(e.target.value)}
                placeholder="ops@example.com"
              />
            </div>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button onClick={createBot} disabled={!canAdmin}>
              Create
            </Button>
          </div>
        </div>
      </Modal>

      <Modal
        open={Boolean(created)}
        title="Bot API key (save now)"
        onClose={() => setCreated(null)}
      >
        {created ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Badge tone="neon">bot</Badge>
              <div className="text-sm font-semibold text-content-primary">{created.name}</div>
            </div>
            <div className="text-sm text-content-tertiary">
              This API key is shown once. Copy it somewhere safe.
            </div>
            <CodeBlock code={created.api_key} label="API key" />
            <div className="flex justify-end">
              <Button onClick={() => setCreated(null)}>Done</Button>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}
