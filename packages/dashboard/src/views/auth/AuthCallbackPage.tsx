import { useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";

import { exchangeAuthCode } from "../../api/client";
import { useSession } from "../../auth/session";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";

export function AuthCallbackPage() {
  const nav = useNavigate();
  const { setSessionToken } = useSession();
  const [params] = useSearchParams();
  const [status, setStatus] = useState<"working" | "error" | "done">("working");
  const [detail, setDetail] = useState<string | null>(null);

  const authCode = useMemo(() => params.get("auth_code"), [params]);

  useEffect(() => {
    let cancelled = false;
    async function run() {
      if (!authCode) {
        setStatus("error");
        setDetail("Missing auth_code in callback URL.");
        return;
      }

      try {
        const out = await exchangeAuthCode(authCode);
        if (cancelled) return;
        setSessionToken(out.session_token);
        setStatus("done");
        nav("/");
      } catch (e) {
        if (cancelled) return;
        const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
        setStatus("error");
        setDetail(msg || "Failed to exchange auth code.");
      }
    }
    void run();
    return () => {
      cancelled = true;
    };
  }, [authCode, nav, setSessionToken]);

  return (
    <div className="mx-auto max-w-xl">
      <Card>
        <CardHeader>
          <div className="text-sm font-semibold text-slate-100">Signing you in…</div>
          <div className="mt-1 text-sm text-slate-300">
            Exchanging OAuth callback code for a session token.
          </div>
        </CardHeader>
        <CardBody>
          {status === "working" ? (
            <div className="text-sm text-slate-300">Working…</div>
          ) : null}
          {status === "error" ? (
            <div className="space-y-3">
              <div className="text-sm text-red-200">Sign-in failed.</div>
              {detail ? <div className="text-xs text-slate-300">{detail}</div> : null}
              <div className="flex gap-2">
                <Button variant="secondary" onClick={() => nav("/auth")}>
                  Back to auth
                </Button>
              </div>
            </div>
          ) : null}
        </CardBody>
      </Card>
    </div>
  );
}

