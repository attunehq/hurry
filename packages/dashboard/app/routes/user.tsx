import { Calendar, Github, LogOut, Mail, Pencil } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router";

import type { MeResponse } from "../api/types";
import { useApi } from "../api/useApi";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { Input } from "../ui/primitives/Input";
import { Label } from "../ui/primitives/Label";
import { Modal } from "../ui/primitives/Modal";
import { PageLayout } from "../ui/shell/PageLayout";
import { useToast } from "../ui/toast/ToastProvider";

export default function UserPage() {
  const nav = useNavigate();
  const toast = useToast();
  const { request, logout, signedIn } = useApi();
  const [me, setMe] = useState<MeResponse | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [newName, setNewName] = useState("");

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setMe(null);
      return;
    }
    try {
      const meOut = await request<MeResponse>({ path: "/api/v1/me" });
      setMe(meOut);
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      setMe(null);
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load user", detail: msg });
    }
  }, [signedIn, request, toast]);

  function openRename() {
    setNewName(me?.name ?? "");
    setRenameOpen(true);
  }

  async function rename() {
    if (!signedIn) return;
    const trimmed = newName.trim();
    if (!trimmed) {
      toast.push({ kind: "error", title: "Name cannot be empty" });
      return;
    }
    setRenameOpen(false);
    try {
      await request<void>({
        path: "/api/v1/me",
        method: "PATCH",
        body: { name: trimmed },
      });
      toast.push({ kind: "success", title: "Account name updated" });
      await refresh();
    } catch (e) {
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 401) return;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Update failed", detail: msg });
    }
  }

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <PageLayout
      title="Account"
      subtitle="View your account information."
      actions={
        <div className="flex gap-2">
          <Button variant="secondary" onClick={openRename} disabled={!signedIn || !me}>
            <Pencil className="h-4 w-4" />
            Rename
          </Button>
          <Button variant="danger" onClick={logout} disabled={!signedIn}>
            <LogOut className="h-4 w-4" />
            Sign out
          </Button>
        </div>
      }
    >
      {!signedIn ? (
        <Card>
          <CardBody>
            <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center">
              <div>
                <div className="text-sm font-semibold text-content-primary">Sign in required</div>
                <div className="mt-1 text-sm text-content-tertiary">
                  Sign in to view your profile information.
                </div>
              </div>
              <Button onClick={() => nav("/auth")}>Sign in</Button>
            </div>
          </CardBody>
        </Card>
      ) : null}

      {signedIn && me ? (
        <Card>
          <CardHeader>
            <div className="text-sm font-semibold text-content-primary">Account Details</div>
          </CardHeader>
          <CardBody>
            <div className="space-y-4">
              {me.name ? (
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 h-4 w-4 text-center text-accent-text text-xs font-bold">N</div>
                  <div>
                    <div className="text-xs font-medium text-content-muted">Name</div>
                    <div className="mt-0.5 text-sm text-content-primary">{me.name}</div>
                  </div>
                </div>
              ) : null}

              <div className="flex items-start gap-3">
                <Mail className="mt-0.5 h-4 w-4 text-accent-text" />
                <div>
                  <div className="text-xs font-medium text-content-muted">Email</div>
                  <div className="mt-0.5 text-sm text-content-primary">{me.email}</div>
                </div>
              </div>

              {me.github_username ? (
                <div className="flex items-start gap-3">
                  <Github className="mt-0.5 h-4 w-4 text-accent-text" />
                  <div>
                    <div className="text-xs font-medium text-content-muted">GitHub Username</div>
                    <div className="mt-0.5 text-sm text-content-primary">{me.github_username}</div>
                  </div>
                </div>
              ) : null}

              <div className="flex items-start gap-3">
                <Calendar className="mt-0.5 h-4 w-4 text-accent-text" />
                <div>
                  <div className="text-xs font-medium text-content-muted">Member Since</div>
                  <div className="mt-0.5 text-sm text-content-primary">
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
            <div className="text-sm text-content-tertiary">Loading...</div>
          </CardBody>
        </Card>
      ) : null}

      <Modal open={renameOpen} title="Update account name" onClose={() => setRenameOpen(false)} onSubmit={rename}>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="account-name">Name</Label>
            <Input
              id="account-name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="Enter your name"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setRenameOpen(false)}>
              Cancel
            </Button>
            <Button onClick={rename}>
              Save
            </Button>
          </div>
        </div>
      </Modal>
    </PageLayout>
  );
}
