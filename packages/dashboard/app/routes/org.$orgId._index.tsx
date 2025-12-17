import clsx from "clsx";
import { KeyRound, Rocket } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";

import type { OrgApiKeyListResponse } from "../api/types";
import { useApi } from "../api/useApi";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { CodeBlock } from "../ui/primitives/CodeBlock";
import { useOrgContext } from "./org.$orgId";

type Platform = "unix" | "windows";

function detectPlatform(): Platform {
  if (typeof window === "undefined") return "unix";
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  return "unix";
}

export default function OrgIndexPage() {
  const nav = useNavigate();
  const { request, signedIn } = useApi();
  const { orgId } = useOrgContext();
  const [apiKeys, setApiKeys] = useState<OrgApiKeyListResponse | null>(null);

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

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Rocket className="h-5 w-5 text-accent-text" />
            <div className="text-sm font-semibold text-content-primary">Getting Started</div>
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
                  <CodeBlock code='export HURRY_API_TOKEN="your-token-here"' />
                </div>
              </GettingStartedStep>

              <GettingStartedStep
                number={3}
                title="Install Hurry"
              >
                <div className="space-y-2 text-xs text-content-tertiary">
                  <div>Run this in your terminal to install the hurry CLI.</div>
                  <GettingStartedInstallTabs />
                </div>
              </GettingStartedStep>

              <GettingStartedStep
                number={4}
                title="Start using Hurry"
              >
                <div className="space-y-2 text-xs text-content-tertiary">
                  <div>Replace your cargo commands with hurry.</div>
                  <div className="space-y-1.5">
                    <CodeBlock code="hurry cargo build" />
                    <CodeBlock code="hurry cargo test" />
                    <CodeBlock code="hurry cargo check" />
                  </div>
                </div>
              </GettingStartedStep>
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
        className={clsx(
          "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold",
          props.done
            ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
            : "bg-accent-bg text-accent-text",
        )}
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

function GettingStartedInstallTabs() {
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
          className={clsx(
            "flex-1 cursor-pointer rounded px-2 py-1 text-xs font-medium transition",
            platform === "unix"
              ? "bg-surface-raised text-content-primary shadow-sm"
              : "text-content-tertiary hover:text-content-secondary",
          )}
        >
          macOS / Linux
        </button>
        <button
          type="button"
          onClick={() => setPlatform("windows")}
          className={clsx(
            "flex-1 cursor-pointer rounded px-2 py-1 text-xs font-medium transition",
            platform === "windows"
              ? "bg-surface-raised text-content-primary shadow-sm"
              : "text-content-tertiary hover:text-content-secondary",
          )}
        >
          Windows
        </button>
      </div>
      <CodeBlock code={commands[platform]} />
    </div>
  );
}

