import { Github, Mail, RefreshCw, Calendar } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";

import { apiRequest } from "../../api/client";
import type { MeResponse } from "../../api/types";
import { useSession } from "../../auth/session";
import { Button } from "../../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../../ui/primitives/Card";
import { useToast } from "../../ui/toast/ToastProvider";

export function UserPage() {
  const nav = useNavigate();
  const toast = useToast();
  const { sessionToken } = useSession();
  const [me, setMe] = useState<MeResponse | null>(null);
  const [loading, setLoading] = useState(false);

  const signedIn = Boolean(sessionToken);

  async function refresh() {
    if (!sessionToken) {
      setMe(null);
      return;
    }
    setLoading(true);
    try {
      const meOut = await apiRequest<MeResponse>({ path: "/api/v1/me", sessionToken });
      setMe(meOut);
    } catch (e) {
      setMe(null);
      const msg = e && typeof e === "object" && "message" in e ? String((e as any).message) : "";
      toast.push({ kind: "error", title: "Failed to load user", detail: msg });
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
  }, [sessionToken]);

  return (
    <div className="space-y-8">
      <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
        <div>
          <h1 className="text-2xl font-semibold text-slate-100">Account</h1>
          <p className="mt-1.5 text-sm text-slate-300">
            View your account information.
          </p>
        </div>
        <Button variant="secondary" onClick={refresh} disabled={!signedIn || loading}>
          <RefreshCw className="h-4 w-4" />
          Refresh
        </Button>
      </div>

      {!signedIn ? (
        <Card>
          <CardBody>
            <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
              <div>
                <div className="text-sm font-semibold text-slate-100">Sign in required</div>
                <div className="mt-1 text-sm text-slate-300">
                  Sign in to view your profile information.
                </div>
              </div>
              <Button onClick={() => nav("/auth")}>Go to auth</Button>
            </div>
          </CardBody>
        </Card>
      ) : null}

      {signedIn && me ? (
        <Card>
          <CardHeader>
            <div className="text-sm font-semibold text-slate-100">Account Details</div>
          </CardHeader>
          <CardBody>
            <div className="space-y-4">
              {me.name ? (
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 h-4 w-4 text-center text-neon-300 text-xs font-bold">N</div>
                  <div>
                    <div className="text-xs font-medium text-slate-400">Name</div>
                    <div className="mt-0.5 text-sm text-slate-100">{me.name}</div>
                  </div>
                </div>
              ) : null}

              <div className="flex items-start gap-3">
                <Mail className="mt-0.5 h-4 w-4 text-neon-300" />
                <div>
                  <div className="text-xs font-medium text-slate-400">Email</div>
                  <div className="mt-0.5 text-sm text-slate-100">{me.email}</div>
                </div>
              </div>

              {me.github_username ? (
                <div className="flex items-start gap-3">
                  <Github className="mt-0.5 h-4 w-4 text-neon-300" />
                  <div>
                    <div className="text-xs font-medium text-slate-400">GitHub Username</div>
                    <div className="mt-0.5 text-sm text-slate-100">{me.github_username}</div>
                  </div>
                </div>
              ) : null}

              <div className="flex items-start gap-3">
                <Calendar className="mt-0.5 h-4 w-4 text-neon-300" />
                <div>
                  <div className="text-xs font-medium text-slate-400">Member Since</div>
                  <div className="mt-0.5 text-sm text-slate-100">
                    {new Date(me.created_at).toLocaleDateString(undefined, {
                      year: "numeric",
                      month: "long",
                      day: "numeric",
                    })}
                  </div>
                </div>
              </div>

            </div>
          </CardBody>
        </Card>
      ) : signedIn ? (
        <Card>
          <CardBody>
            <div className="text-sm text-slate-300">Loading...</div>
          </CardBody>
        </Card>
      ) : null}
    </div>
  );
}
