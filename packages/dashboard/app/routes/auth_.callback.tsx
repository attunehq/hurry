import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { exchangeAuthCode } from "../api/client";
import { useSession } from "../auth/session";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody } from "../ui/primitives/Card";
import { PageLayout } from "../ui/shell/PageLayout";

export default function AuthCallbackPage() {
  const nav = useNavigate();
  const { setSessionToken } = useSession();
  const [params] = useSearchParams();
  const [status, setStatus] = useState<"working" | "error" | "done">("working");
  const [detail, setDetail] = useState<string | null>(null);

  // Track whether we've already attempted to exchange this code.
  // This prevents double-exchange in React StrictMode (which runs effects twice).
  const attemptedRef = useRef(false);

  const authCode = useMemo(() => params.get("auth_code"), [params]);

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
        nav("/");
      } catch (e) {
        const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
        setStatus("error");
        setDetail(msg || "Failed to exchange auth code.");
      }
    }
    void run();
  }, [authCode, nav, setSessionToken]);

  return (
    <PageLayout
      title="Signing you in…"
      subtitle="Exchanging OAuth callback code for a session token."
    >
      <Card>
        <CardBody>
          {status === "working" ? (
            <div className="text-sm text-content-tertiary">Working…</div>
          ) : null}
          {status === "error" ? (
            <div className="space-y-3">
              <div className="text-sm text-red-600 dark:text-red-200">Sign-in failed.</div>
              {detail ? <div className="text-xs text-content-tertiary">{detail}</div> : null}
              <div className="flex gap-2">
                <Button variant="secondary" onClick={() => nav("/auth")}>
                  Back to auth
                </Button>
              </div>
            </div>
          ) : null}
        </CardBody>
      </Card>
    </PageLayout>
  );
}
