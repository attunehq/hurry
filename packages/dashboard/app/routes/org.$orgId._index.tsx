import { Copy, KeyRound, Rocket, Terminal, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";

import type { OrgApiKeyListResponse } from "../api/types";
import { useApi } from "../api/useApi";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { useToast } from "../ui/toast/ToastProvider";
import { useOrgContext } from "./org.$orgId";

const GETTING_STARTED_DISMISSED_KEY = "hurry.gettingStartedDismissed";

type Platform = "unix" | "windows";

function detectPlatform(): Platform {
  if (typeof window === "undefined") return "unix";
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  return "unix";
}

export default function OrgIndexPage() {
  const nav = useNavigate();
  const toast = useToast();
  const { request, signedIn } = useApi();
  const { orgId } = useOrgContext();
  const [apiKeys, setApiKeys] = useState<OrgApiKeyListResponse | null>(null);

  // TODO: Move dismissed state to server-side user preferences
  const [dismissed, setDismissed] = useState(() => {
    if (typeof window === "undefined") return false;
    return localStorage.getItem(GETTING_STARTED_DISMISSED_KEY) === "true";
  });

  const hasApiKeys = useMemo(() => (apiKeys?.api_keys.length ?? 0) > 0, [apiKeys]);

  const loadApiKeys = useCallback(async () => {
    if (!signedIn) return;
    try {
      const out = await request<OrgApiKeyListResponse>({
        path: `/api/v1/organizations/${orgId}/api-keys`,
      });
      setApiKeys(out);
    } catch {
      // Ignore errors, just won't show key count
    }
  }, [signedIn, orgId, request]);

  useEffect(() => {
    void loadApiKeys();
  }, [loadApiKeys]);

  function dismissGettingStarted() {
    setDismissed(true);
    localStorage.setItem(GETTING_STARTED_DISMISSED_KEY, "true");
  }

  async function copyToClipboard(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      toast.push({ kind: "success", title: "Copied" });
    } catch {
      toast.push({ kind: "error", title: "Copy failed" });
    }
  }

  return (
    <div className="space-y-4">
      {!dismissed ? (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Rocket className="h-5 w-5 text-accent-text" />
                <div className="text-sm font-semibold text-content-primary">Getting Started</div>
              </div>
              <button
                type="button"
                onClick={dismissGettingStarted}
                className="rounded p-1 text-content-muted transition hover:bg-surface-subtle hover:text-content-secondary"
                title="Dismiss"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </CardHeader>
          <CardBody>
            <div className="space-y-4">
              <GettingStartedStep
                number={1}
                title="Get your API key"
                done={hasApiKeys}
              >
                {hasApiKeys ? (
                  (() => {
                    const count = apiKeys?.api_keys.length ?? 0;
                    return (
                      <div className="text-xs text-content-tertiary">
                        You have {count} API key{count === 1 ? "" : "s"}.{" "}
                        <Link to="api-keys" className="text-accent-text hover:underline">
                          View keys
                        </Link>
                      </div>
                    );
                  })()
                ) : (
                  <div className="flex items-center gap-2">
                    <Button size="sm" onClick={() => nav("api-keys")}>
                      <KeyRound className="h-4 w-4" />
                      Create API key
                    </Button>
                  </div>
                )}
              </GettingStartedStep>

              <GettingStartedStep
                number={2}
                title="Set up your environment"
              >
                <div className="space-y-2 text-xs text-content-tertiary">
                  <div>Add your API token to your shell config.</div>
                  <GettingStartedCodeBlock
                    code='export HURRY_API_TOKEN="your-token-here"'
                    onCopy={copyToClipboard}
                  />
                </div>
              </GettingStartedStep>

              <GettingStartedStep
                number={3}
                title="Install Hurry"
              >
                <div className="space-y-2 text-xs text-content-tertiary">
                  <div>Run this in your terminal to install the hurry CLI.</div>
                  <GettingStartedInstallTabs onCopy={copyToClipboard} />
                </div>
              </GettingStartedStep>

              <GettingStartedStep
                number={4}
                title="Start using Hurry"
              >
                <div className="space-y-2 text-xs text-content-tertiary">
                  <div>Replace your cargo commands with hurry.</div>
                  <div className="space-y-1.5">
                    <GettingStartedCodeBlock code="hurry cargo build" onCopy={copyToClipboard} />
                    <GettingStartedCodeBlock code="hurry cargo test" onCopy={copyToClipboard} />
                    <GettingStartedCodeBlock code="hurry cargo check" onCopy={copyToClipboard} />
                  </div>
                </div>
              </GettingStartedStep>
            </div>
          </CardBody>
        </Card>
      ) : null}

      <Card>
        <CardHeader>
          <div className="text-sm font-semibold text-content-primary">Quick Links</div>
        </CardHeader>
        <CardBody>
          <div className="grid gap-3 sm:grid-cols-2">
            <QuickLinkCard
              to="api-keys"
              icon={<KeyRound className="h-5 w-5" />}
              title="API Keys"
              description="Manage authentication tokens"
            />
            <QuickLinkCard
              to="members"
              icon={<Terminal className="h-5 w-5" />}
              title="Members"
              description="View and manage team members"
            />
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

function GettingStartedStep(props: {
  number: number;
  title: string;
  done?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3">
      <div
        className={[
          "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold",
          props.done
            ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
            : "bg-accent-bg text-accent-text",
        ].join(" ")}
      >
        {props.done ? "\u2713" : props.number}
      </div>
      <div className="flex-1 space-y-1.5">
        <div className="text-sm font-medium text-content-primary">{props.title}</div>
        {props.children}
      </div>
    </div>
  );
}

function GettingStartedCodeBlock(props: { code: string; onCopy: (value: string) => void }) {
  return (
    <div className="group flex items-center justify-between gap-2 rounded-lg border border-border bg-surface-subtle px-3 py-2">
      <code className="flex-1 overflow-x-auto whitespace-nowrap font-mono text-xs text-content-primary">
        {props.code}
      </code>
      <button
        type="button"
        onClick={() => props.onCopy(props.code)}
        className="shrink-0 rounded p-1 text-content-muted opacity-0 transition hover:bg-surface-raised hover:text-content-secondary group-hover:opacity-100"
        title="Copy"
      >
        <Copy className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function GettingStartedInstallTabs(props: { onCopy: (value: string) => void }) {
  const [platform, setPlatform] = useState<Platform>(detectPlatform);

  const commands = {
    unix: "curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash",
    windows: "irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex",
  };

  return (
    <div className="space-y-1.5">
      <div className="flex gap-1 rounded-md border border-border bg-surface-subtle p-0.5">
        <button
          type="button"
          onClick={() => setPlatform("unix")}
          className={[
            "flex-1 rounded px-2 py-1 text-xs font-medium transition",
            platform === "unix"
              ? "bg-surface-raised text-content-primary shadow-sm"
              : "text-content-tertiary hover:text-content-secondary",
          ].join(" ")}
        >
          macOS / Linux
        </button>
        <button
          type="button"
          onClick={() => setPlatform("windows")}
          className={[
            "flex-1 rounded px-2 py-1 text-xs font-medium transition",
            platform === "windows"
              ? "bg-surface-raised text-content-primary shadow-sm"
              : "text-content-tertiary hover:text-content-secondary",
          ].join(" ")}
        >
          Windows
        </button>
      </div>
      <GettingStartedCodeBlock code={commands[platform]} onCopy={props.onCopy} />
    </div>
  );
}

function QuickLinkCard(props: {
  to: string;
  icon: React.ReactNode;
  title: string;
  description: string;
}) {
  return (
    <Link
      to={props.to}
      className="flex items-center gap-3 rounded-xl border border-border bg-surface-subtle p-4 transition hover:border-border-accent-hover hover:bg-surface-subtle-hover"
    >
      <div className="text-accent-text">{props.icon}</div>
      <div>
        <div className="text-sm font-medium text-content-primary">{props.title}</div>
        <div className="text-xs text-content-muted">{props.description}</div>
      </div>
    </Link>
  );
}
