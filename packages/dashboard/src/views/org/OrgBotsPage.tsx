import { Bot, Copy, Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { apiRequest } from "../../api/client";
import type { BotListResponse, CreateBotResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { Modal } from "../../ui/primitives/Modal";
import { useToast } from "../../ui/toast/ToastProvider";
import { useOrgContext } from "./orgContext";

export function OrgBotsPage() {
  const toast = useToast();
  const { sessionToken } = useSession();
  const { orgId, role } = useOrgContext();
  const [data, setData] = useState<BotListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);

  const [botName, setBotName] = useState("");
  const [responsibleEmail, setResponsibleEmail] = useState("");
  const [created, setCreated] = useState<CreateBotResponse | null>(null);

  const bots = useMemo(() => data?.bots ?? [], [data]);
  const canAdmin = role === "admin";

  async function load() {
    if (!sessionToken) return;
    setLoading(true);
    try {
      const out = await apiRequest<BotListResponse>({
        path: `/api/v1/organizations/${orgId}/bots`,
        sessionToken,
      });
      setData(out);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load bots", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }

  async function createBot() {
    if (!sessionToken) return;
    const n = botName.trim();
    const e = responsibleEmail.trim();
    if (!n || !e) {
      toast.push({ kind: "error", title: "Name and responsible email required" });
      return;
    }
    try {
      const out = await apiRequest<CreateBotResponse>({
        path: `/api/v1/organizations/${orgId}/bots`,
        method: "POST",
        sessionToken,
        body: { name: n, responsible_email: e },
      });
      setCreated(out);
      setBotName("");
      setResponsibleEmail("");
      toast.push({ kind: "success", title: "Bot created", detail: out.name });
      setCreateOpen(false);
      await load();
    } catch (err) {
      const msg =
        err && typeof err === "object" && "message" in err ? String((err as any).message) : "";
      toast.push({ kind: "error", title: "Create failed", detail: msg });
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
              <div className="text-sm font-semibold text-slate-100">Bots</div>
              <div className="mt-1 text-sm text-slate-300">
                Bots are org-scoped accounts for CI/automation. Creating a bot returns an API key once.
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
              <thead className="text-xs text-slate-400">
                <tr className="border-b border-white/10">
                  <th className="py-2 pr-3">Bot</th>
                  <th className="py-2 pr-3">Responsible</th>
                  <th className="py-2 pr-3">Created</th>
                </tr>
              </thead>
              <tbody>
                {bots.map((b) => (
                  <tr key={b.account_id} className="border-b border-white/5">
                    <td className="py-3 pr-3">
                      <div className="flex items-center gap-2 font-medium text-slate-100">
                        <Bot className="h-4 w-4 text-neon-300" />
                        {b.name ?? `Bot ${b.account_id}`}
                      </div>
                      <div className="text-xs text-slate-400">Account ID: {b.account_id}</div>
                    </td>
                    <td className="py-3 pr-3 text-slate-200">{b.responsible_email}</td>
                    <td className="py-3 pr-3 text-xs text-slate-300">{b.created_at}</td>
                  </tr>
                ))}
                {bots.length === 0 && !loading ? (
                  <tr>
                    <td colSpan={3} className="py-6 text-center text-sm text-slate-400">
                      No bots yet.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-xs text-slate-400">
            To revoke a bot, disable its account via server-side tooling (bots are accounts).
          </div>
        </CardBody>
      </Card>

      <Modal open={createOpen} title="Create bot" onClose={() => setCreateOpen(false)}>
        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="botName">Name</Label>
              <Input
                id="botName"
                value={botName}
                onChange={(e) => setBotName(e.target.value)}
                placeholder="CI Bot"
              />
            </div>
            <div className="space-y-2">
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
              <div className="text-sm font-semibold text-slate-100">{created.name}</div>
            </div>
            <div className="text-sm text-slate-300">
              This API key is shown once. Copy it somewhere safe.
            </div>
            <div className="rounded-2xl border border-white/10 bg-white/5 p-4">
              <div className="text-xs text-slate-400">API key</div>
              <div className="mt-1 break-all font-mono text-xs text-slate-100">
                {created.api_key}
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => copy(created.api_key)}>
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

