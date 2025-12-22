import clsx from "clsx";
import { AlertTriangle, Check } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { useOrgs } from "../org/OrgContext";
import { Button } from "../ui/primitives/Button";
import { CodeBlock } from "../ui/primitives/CodeBlock";

type Platform = "unix" | "windows";

function detectPlatform(): Platform {
  if (typeof window === "undefined") return "unix";
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  return "unix";
}

export default function OnboardingPage() {
  const nav = useNavigate();
  const [searchParams] = useSearchParams();
  const { refresh: refreshOrgs } = useOrgs();
  const token = searchParams.get("token");
  const orgId = searchParams.get("org");

  // When users arrive here, they might have just accepted an invitation for a
  // new organization. In order to handle the org dropdown properly, we refresh
  // the orgs list pre-emptively.
  useEffect(() => {
    void refreshOrgs();
  }, [refreshOrgs]);

  if (!token || !orgId) {
    return (
      <div className="noise fixed inset-0 flex items-center justify-center">
        <div className="w-full max-w-md px-6">
          <div className="rounded-2xl border border-border bg-surface-raised p-6 text-center shadow-glow-soft">
            <div className="text-content-muted">
              No API token found. Please create an API key from the API Keys page.
            </div>
            <div className="mt-4">
              <Button onClick={() => nav("/")}>Go to Dashboard</Button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="noise fixed inset-0 overflow-y-auto">
      <div className="flex min-h-full items-center justify-center px-6 py-12">
        <div className="w-full max-w-2xl">
          {/* Brand */}
          <div className="mb-8 flex items-center justify-center gap-3">
            <div className="grid h-11 w-11 place-items-center rounded-xl border border-border bg-surface-subtle shadow-glow-soft">
              <span className="bg-linear-to-br from-attune-300 to-attune-500 bg-clip-text text-2xl font-bold text-transparent">
                A
              </span>
            </div>
            <div className="text-xl font-semibold text-content-primary">Hurry</div>
          </div>

          {/* Welcome card */}
          <div className="rounded-2xl border border-border bg-surface-raised shadow-glow-soft backdrop-blur">
            <div className="border-b border-border px-6 py-4 text-center">
              <div className="text-sm text-content-tertiary">
                An initial API key for you to use has been created. Get ready for faster builds!
              </div>
            </div>

            <div className="space-y-6 p-6">
              <OnboardingStep
                number={1}
                title="Copy your API token"
                warning="This token is only shown once. Save it somewhere safe."
                subdescription="You can create more tokens later on your API Keys page."
              >
                <CodeBlock code={token} wrap />
              </OnboardingStep>

              <OnboardingStep
                number={2}
                title="Set up your environment"
                description="You may want to add this to your shell config for persistence."
              >
                <CodeBlock code={`export HURRY_API_TOKEN="${token}"`} />
              </OnboardingStep>

              <OnboardingStep
                number={3}
                title="Install Hurry"
                description="Run this in your terminal to install the hurry CLI."
              >
                <InstallTabs />
              </OnboardingStep>

              <OnboardingStep
                number={4}
                title="Start using Hurry"
                description="Replace your cargo commands with hurry."
              >
                <div className="space-y-2">
                  <CodeBlock code="hurry cargo build" />
                  <CodeBlock code="hurry cargo test" />
                  <CodeBlock code="hurry cargo check" />
                </div>
              </OnboardingStep>
            </div>

            <div className="border-t border-border px-6 py-4">
              <Button className="w-full" onClick={() => nav(`/org/${orgId}`)}>
                <Check className="h-4 w-4" />
                Got it, take me to my organization
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function OnboardingStep(props: {
  number: number;
  title: string;
  description?: string;
  warning?: string;
  subdescription?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <div className="flex h-6 w-6 items-center justify-center rounded-full bg-accent-subtle text-xs font-semibold text-accent-text">
          {props.number}
        </div>
        <div className="text-sm font-semibold text-content-primary">{props.title}</div>
      </div>
      <div className="ml-8 space-y-2">
        {props.description ? (
          <div className="text-xs text-content-tertiary">{props.description}</div>
        ) : null}
        {props.warning ? (
          <div className="flex items-center gap-1.5 text-xs text-warn-text">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            <span>{props.warning}</span>
          </div>
        ) : null}
        {props.subdescription ? (
          <div className="text-xs text-content-muted">{props.subdescription}</div>
        ) : null}
        {props.children}
      </div>
    </div>
  );
}

function InstallTabs() {
  const [platform, setPlatform] = useState<Platform>(detectPlatform);

  const commands = {
    unix: "curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash",
    windows: "irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex",
  };

  return (
    <div className="space-y-2">
      <div className="flex gap-1 rounded-lg border border-border bg-surface-subtle p-1">
        <button
          type="button"
          onClick={() => setPlatform("unix")}
          className={clsx(
            "flex-1 cursor-pointer rounded-md px-3 py-1.5 text-xs font-medium transition",
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
            "flex-1 cursor-pointer rounded-md px-3 py-1.5 text-xs font-medium transition",
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
