import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router";

import type { AcceptInvitationResponse, InvitationPreviewResponse } from "../api/types";
import { useApi } from "../api/useApi";
import { Badge } from "../ui/primitives/Badge";
import { Button } from "../ui/primitives/Button";
import { useToast } from "../ui/toast/ToastProvider";

export default function InvitePage() {
  const nav = useNavigate();
  const toast = useToast();
  const { token } = useParams();
  const { request, signedIn } = useApi();
  const [preview, setPreview] = useState<InvitationPreviewResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [accepting, setAccepting] = useState(false);

  const inviteToken = useMemo(() => token ?? "", [token]);

  const load = useCallback(async () => {
    if (!inviteToken) return;
    setLoading(true);
    try {
      const out = await request<InvitationPreviewResponse>({
        path: `/api/v1/invitations/${encodeURIComponent(inviteToken)}`,
      });
      setPreview(out);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Invite not found", detail: msg });
      setPreview(null);
    } finally {
      setLoading(false);
    }
  }, [inviteToken, request, toast]);

  async function accept() {
    if (!signedIn) {
      nav("/auth", { state: { from: `/invite/${inviteToken}` } });
      return;
    }
    setAccepting(true);
    try {
      const out = await request<AcceptInvitationResponse>({
        path: `/api/v1/invitations/${encodeURIComponent(inviteToken)}/accept`,
        method: "POST",
      });
      nav(`/org/${out.organization_id}`);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Accept failed", detail: msg });
    } finally {
      setAccepting(false);
    }
  }

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="noise fixed inset-0 flex items-center justify-center">
      <div className="w-full max-w-md px-6">
        {/* Brand */}
        <div className="mb-8 flex items-center justify-center gap-3">
          <div className="grid h-11 w-11 place-items-center rounded-xl border border-border bg-surface-subtle shadow-glow-soft">
            <span className="text-2xl font-bold bg-linear-to-br from-attune-300 to-attune-500 bg-clip-text text-transparent">
              A
            </span>
          </div>
          <div className="text-xl font-semibold text-content-primary">Hurry</div>
        </div>

        {/* Invite card */}
        <div className="rounded-2xl border border-border bg-surface-raised shadow-glow-soft backdrop-blur">
          <div className="border-b border-border px-6 py-4">
            <div className="text-base font-semibold text-content-primary">Invitation</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Preview what you're joining before you accept.
            </div>
          </div>

          <div className="p-6">
            {loading && (
              <div className="text-sm text-content-tertiary">Loading…</div>
            )}
            {preview && (
              <div className="space-y-4">
                <div className="rounded-xl border border-border bg-surface-subtle p-4">
                  <div className="text-xs text-content-muted">Organization</div>
                  <div className="mt-1 text-sm font-semibold text-content-primary">
                    {preview.organization_name}
                  </div>
                  <div className="mt-2 flex items-center gap-2">
                    <Badge tone="muted">Role</Badge>
                    <Badge tone={preview.role === "admin" ? "neon" : "muted"}>{preview.role}</Badge>
                    {!preview.valid && <Badge tone="warn">invalid</Badge>}
                  </div>
                </div>

                {!signedIn && (
                  <div className="rounded-xl border border-amber-400/25 bg-amber-400/10 p-4">
                    <div className="text-sm font-medium text-content-primary">
                      New to Hurry?
                    </div>
                    <div className="mt-1 text-sm text-content-secondary">
                      You'll need to sign in and complete the onboarding flow first.
                      After that, come back to this invite link to accept it.
                    </div>
                  </div>
                )}

                <div className="flex gap-2">
                  <Button onClick={accept} disabled={!preview.valid || accepting}>
                    {signedIn ? "Accept invite" : "Sign in to accept"}
                  </Button>
                  <Button variant="secondary" onClick={() => nav("/")}>
                    Back
                  </Button>
                </div>
              </div>
            )}
            {!loading && !preview && (
              <div className="text-sm text-content-tertiary">No preview available.</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
