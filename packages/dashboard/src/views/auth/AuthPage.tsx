import { ArrowRight, Github, KeyRound } from "lucide-react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

import { apiUrl } from "../../api/client";
import { useSession } from "../../auth/session";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { Input } from "../../ui/primitives/Input";
import { Label } from "../../ui/primitives/Label";
import { useToast } from "../../ui/toast/ToastProvider";

type LocationState = { from?: string } | null;

export function AuthPage() {
  const nav = useNavigate();
  const loc = useLocation();
  const toast = useToast();
  const { sessionToken, setSessionToken } = useSession();
  const [token, setToken] = useState("");

  const from = useMemo(() => {
    const s = (loc.state as LocationState) ?? null;
    return s?.from ?? "/";
  }, [loc.state]);

  function startOAuth() {
    const redirectUri = `${window.location.origin}/auth/callback`;
    const url = apiUrl(
      `/api/v1/oauth/github/start?redirect_uri=${encodeURIComponent(redirectUri)}`,
    );
    window.location.assign(url);
  }

  function saveToken() {
    if (!token.trim()) {
      toast.push({ kind: "error", title: "Session token required" });
      return;
    }
    setSessionToken(token.trim());
    toast.push({ kind: "success", title: "Session token saved" });
    nav(from);
  }

  return (
    <div className="mx-auto max-w-3xl">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between gap-4">
            <div>
              <div className="text-sm font-semibold text-slate-100">Authentication</div>
              <div className="mt-1 text-sm text-slate-300">
                Sign in to manage orgs, invitations, API keys, and bots.
              </div>
            </div>
            {sessionToken ? (
              <div className="text-xs text-slate-400">Session token stored locally</div>
            ) : null}
          </div>
        </CardHeader>
        <CardBody>
          <div className="grid gap-6 md:grid-cols-2">
            <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
              <div className="flex items-center gap-2 text-sm font-semibold text-slate-100">
                <Github className="h-4 w-4 text-slate-200" />
                Continue with GitHub
              </div>
              <div className="mt-2 text-sm text-slate-300">
                Uses Courier’s OAuth flow. If your local Courier isn’t configured for OAuth yet,
                use the session token option.
              </div>
              <div className="mt-4">
                <Button onClick={startOAuth}>
                  Start OAuth
                  <ArrowRight className="h-4 w-4" />
                </Button>
              </div>
            </div>

            <div className="rounded-2xl border border-white/10 bg-white/5 p-5">
              <div className="flex items-center gap-2 text-sm font-semibold text-slate-100">
                <KeyRound className="h-4 w-4 text-neon-300" />
                Use a session token
              </div>
              <div className="mt-2 text-sm text-slate-300">
                Paste a Courier session token (dev-friendly; e.g. from `scripts/api/login`).
              </div>

              <div className="mt-4 space-y-2">
                <Label htmlFor="token">Session token</Label>
                <Input
                  id="token"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="Paste token…"
                  autoComplete="off"
                  spellCheck={false}
                />
                <div className="flex gap-2">
                  <Button variant="secondary" onClick={() => setToken("")}>
                    Clear
                  </Button>
                  <Button onClick={saveToken}>Save</Button>
                </div>
                <div className="text-xs text-slate-400">
                  Stored in `localStorage` only; never written to git.
                </div>
              </div>
            </div>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

