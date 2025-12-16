import { ArrowRight, Github } from "lucide-react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router";

import { apiUrl } from "../api/client";
import { useSession } from "../auth/session";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody } from "../ui/primitives/Card";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { PageLayout } from "../ui/shell/PageLayout";
import { useToast } from "../ui/toast/ToastProvider";

type LocationState = { from?: string } | null;

export default function AuthPage() {
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
    nav(from);
  }

  return (
    <PageLayout
      title="Authentication"
      subtitle="Sign in to manage orgs, invitations, API keys, and bots."
      actions={
        sessionToken ? (
          <div className="text-xs text-content-muted">Session token stored locally</div>
        ) : null
      }
    >
      <Card>
        <CardBody>
          <div className="rounded-2xl border border-border bg-surface-subtle p-5">
            <div className="flex items-center gap-2 text-sm font-semibold text-content-primary">
              <Github className="h-4 w-4 text-content-secondary" />
              Continue with GitHub
            </div>
            <div className="mt-2 text-sm text-content-tertiary">
              Sign in with your GitHub account.
            </div>
            <div className="mt-4">
              <Button onClick={startOAuth}>
                Sign in with GitHub
                <ArrowRight className="h-4 w-4" />
              </Button>
            </div>
          </div>

          {import.meta.env.DEV && (
            <div className="mt-4 rounded-2xl border border-border bg-surface-subtle p-5">
              <div className="flex items-center gap-2 text-sm font-semibold text-content-primary">
                Dev: Use a session token
              </div>
              <div className="mt-2 text-sm text-content-tertiary">
                Paste a session token for local development.
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
              </div>
            </div>
          )}
        </CardBody>
      </Card>
    </PageLayout>
  );
}
