import { ArrowRight, Github } from "lucide-react";
import { useState } from "react";

import { apiUrl } from "../api/client";
import { Button } from "../ui/primitives/Button";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { useToast } from "../ui/toast/ToastProvider";
import { useSession } from "./session";

/**
 * Centered, modal-style login card shown to unauthenticated users.
 * This is the main authentication UI for the app.
 */
export function LoginCard() {
  const toast = useToast();
  const { setSessionToken } = useSession();
  const [token, setToken] = useState("");

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
  }

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

        {/* Login card */}
        <div className="rounded-2xl border border-border bg-surface-raised shadow-glow-soft backdrop-blur">
          <div className="border-b border-border px-6 py-4">
            <div className="text-base font-semibold text-content-primary">Sign in</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Sign in to manage orgs, invitations, API keys, and bots.
            </div>
          </div>

          <div className="p-6 space-y-4">
            {/* GitHub OAuth */}
            <div className="rounded-xl border border-border bg-surface-subtle p-4">
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

            {/* Dev mode token entry */}
            {import.meta.env.DEV && (
              <div className="rounded-xl border border-border bg-surface-subtle p-4">
                <div className="flex items-center gap-2 text-sm font-semibold text-content-primary">
                  Dev: Use a session token
                </div>
                <div className="mt-2 text-sm text-content-tertiary">
                  Paste a session token for local development.
                </div>

                <div className="mt-4 space-y-2">
                  <Label htmlFor="dev-token">Session token</Label>
                  <Input
                    id="dev-token"
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
          </div>
        </div>
      </div>
    </div>
  );
}
