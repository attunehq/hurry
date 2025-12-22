import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { apiRequest, exchangeAuthCode } from "../api/client";
import type { AcceptInvitationResponse, CreateOrgApiKeyResponse, OrganizationListResponse } from "../api/types";
import { useSession } from "../auth/session";
import { Button } from "../ui/primitives/Button";

type Status = "working" | "error" | "done";

const STATUS_COPY: Record<Status, { title: string; subtitle: string }> = {
  working: { title: "Signing you in…", subtitle: "Exchanging OAuth callback code." },
  error: { title: "Sign-in failed", subtitle: "There was a problem signing you in." },
  done: { title: "Success", subtitle: "Redirecting to dashboard…" },
};

export default function AuthCallbackPage() {
  const nav = useNavigate();
  const { setSessionToken } = useSession();
  const [params] = useSearchParams();
  const [status, setStatus] = useState<Status>("working");
  const [detail, setDetail] = useState<string | null>(null);

  // Track whether we've already attempted to exchange this code.
  // This prevents double-exchange in React StrictMode (which runs effects twice).
  const attemptedRef = useRef(false);

  const authCode = useMemo(() => params.get("auth_code"), [params]);
  const isNewUser = useMemo(() => params.get("new_user") === "true", [params]);
  const inviteToken = useMemo(() => params.get("invite"), [params]);

  useEffect(() => {
    // Prevent double-exchange in StrictMode.
    // StrictMode mounts, unmounts, then remounts - the ref persists across this.
    if (attemptedRef.current) return;
    attemptedRef.current = true;

    async function run() {
      if (!authCode) {
        setStatus("error");
        setDetail("Missing auth_code in callback URL.");
        return;
      }

      try {
        const out = await exchangeAuthCode(authCode);
        setSessionToken(out.session_token);
        setStatus("done");

        if (inviteToken) {
          await handleInvitationFlow(out.session_token, inviteToken);
        } else if (isNewUser) {
          // New user onboarding: create API key and redirect to onboarding flow
          await handleNewUserOnboarding(out.session_token);
        } else {
          nav("/");
        }
      } catch (e) {
        const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
        setStatus("error");
        setDetail(msg || "Failed to exchange auth code.");
      }
    }

    async function handleInvitationFlow(sessionToken: string, token: string) {
      try {
        const invitation = await apiRequest<AcceptInvitationResponse>({
          path: `/api/v1/invitations/${encodeURIComponent(token)}/accept`,
          method: "POST",
          sessionToken,
        });

        const apiKey = await apiRequest<CreateOrgApiKeyResponse>({
          path: `/api/v1/organizations/${invitation.organization_id}/api-keys`,
          method: "POST",
          sessionToken,
          body: { name: "Default" },
        });

        nav(`/onboarding?token=${encodeURIComponent(apiKey.token)}&org=${invitation.organization_id}`);
      } catch {
        // If invitation acceptance fails (expired, already used, etc.), fall back to normal flow
        if (isNewUser) {
          await handleNewUserOnboarding(sessionToken);
        } else {
          nav("/");
        }
      }
    }

    async function handleNewUserOnboarding(sessionToken: string) {
      try {
        // Fetch user's organizations
        const orgsResponse = await apiRequest<OrganizationListResponse>({
          path: "/api/v1/me/organizations",
          sessionToken,
        });

        const orgs = orgsResponse.organizations;
        if (orgs.length === 0) {
          // No org found, fall back to home
          nav("/");
          return;
        }

        // Use the first org (auto-created by Courier)
        const org = orgs[0];

        // Create a default API key for onboarding
        const keyName = "Default";
        const apiKey = await apiRequest<CreateOrgApiKeyResponse>({
          path: `/api/v1/organizations/${org.id}/api-keys`,
          method: "POST",
          sessionToken,
          body: { name: keyName },
        });

        // Redirect to onboarding page with token and org
        nav(`/onboarding?token=${encodeURIComponent(apiKey.token)}&org=${org.id}`);
      } catch {
        // If onboarding setup fails, just go to home page
        nav("/");
      }
    }

    void run();
  }, [authCode, isNewUser, inviteToken, nav, setSessionToken]);

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

        {/* Status card */}
        <div className="rounded-2xl border border-border bg-surface-raised shadow-glow-soft backdrop-blur">
          <div className="border-b border-border px-6 py-4">
            <div className="text-base font-semibold text-content-primary">
              {STATUS_COPY[status].title}
            </div>
            <div className="mt-1 text-sm text-content-tertiary">
              {STATUS_COPY[status].subtitle}
            </div>
          </div>

          <div className="p-6">
            {status === "working" && (
              <div className="text-sm text-content-tertiary">Working…</div>
            )}
            {status === "error" && (
              <div className="space-y-4">
                {detail && (
                  <div className="rounded-xl border border-border bg-surface-subtle p-4 text-sm text-content-tertiary">
                    {detail}
                  </div>
                )}
                <Button variant="secondary" onClick={() => nav("/auth")}>
                  Back to sign in
                </Button>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
