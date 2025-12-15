import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

import { apiRequest } from "../../api/client";
import type { AcceptInvitationResponse, InvitationPreviewResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Badge } from "../../ui/primitives/Badge";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { useToast } from "../../ui/toast/ToastProvider";

export function InvitePage() {
  const nav = useNavigate();
  const toast = useToast();
  const { token } = useParams();
  const { sessionToken } = useSession();
  const [preview, setPreview] = useState<InvitationPreviewResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [accepting, setAccepting] = useState(false);

  const inviteToken = useMemo(() => token ?? "", [token]);

  async function load() {
    if (!inviteToken) return;
    setLoading(true);
    try {
      const out = await apiRequest<InvitationPreviewResponse>({
        path: `/api/v1/invitations/${encodeURIComponent(inviteToken)}`,
      });
      setPreview(out);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Invite not found", detail: msg });
      setPreview(null);
    } finally {
      setLoading(false);
    }
  }

  async function accept() {
    if (!sessionToken) {
      nav("/auth", { state: { from: `/invite/${inviteToken}` } });
      return;
    }
    setAccepting(true);
    try {
      const out = await apiRequest<AcceptInvitationResponse>({
        path: `/api/v1/invitations/${encodeURIComponent(inviteToken)}/accept`,
        method: "POST",
        sessionToken,
      });
      nav(`/org/${out.organization_id}`);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Accept failed", detail: msg });
    } finally {
      setAccepting(false);
    }
  }

  useEffect(() => {
    void load();
  }, [inviteToken]);

  return (
    <div className="mx-auto max-w-2xl">
      <Card>
        <CardHeader>
          <div className="text-sm font-semibold text-content-primary">Invitation</div>
          <div className="mt-1 text-sm text-content-tertiary">
            Preview what you're joining before you accept.
          </div>
        </CardHeader>
        <CardBody>
          {loading ? <div className="text-sm text-content-tertiary">Loading…</div> : null}
          {preview ? (
            <div className="space-y-4">
              <div className="rounded-2xl border border-border bg-surface-subtle p-4">
                <div className="text-xs text-content-muted">Organization</div>
                <div className="mt-1 text-sm font-semibold text-content-primary">
                  {preview.organization_name}
                </div>
                <div className="mt-2 flex items-center gap-2">
                  <Badge tone="muted">Role</Badge>
                  <Badge tone={preview.role === "admin" ? "neon" : "muted"}>{preview.role}</Badge>
                  {!preview.valid ? <Badge tone="warn">invalid</Badge> : null}
                </div>
              </div>

              <div className="flex gap-2">
                <Button onClick={accept} disabled={!preview.valid || accepting}>
                  Accept invite
                </Button>
                <Button variant="secondary" onClick={() => nav("/")}>
                  Back
                </Button>
              </div>
            </div>
          ) : !loading ? (
            <div className="text-sm text-content-tertiary">No preview available.</div>
          ) : null}
        </CardBody>
      </Card>
    </div>
  );
}
