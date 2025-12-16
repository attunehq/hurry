import { Check, Copy } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router";

import { Button } from "../ui/primitives/Button";
import { useToast } from "../ui/toast/ToastProvider";

export default function OnboardingPage() {
  const nav = useNavigate();
  const toast = useToast();
  const [searchParams] = useSearchParams();
  const token = searchParams.get("token");
  const orgId = searchParams.get("org");

  async function copy(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      toast.push({ kind: "success", title: "Copied" });
    } catch {
      toast.push({ kind: "error", title: "Copy failed" });
    }
  }

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
        <div className="w-full max-w-xl">
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
              <div className="text-lg font-semibold text-content-primary">Welcome to Hurry!</div>
              <div className="mt-1 text-sm text-content-tertiary">
                Your API key has been created. Follow these steps to start using Hurry:
              </div>
            </div>

            <div className="space-y-6 p-6">
              <OnboardingStep
                number={1}
                title="Copy your API token"
                description="This token is only shown once. Save it somewhere safe."
                subdescription="You can create more tokens later on your API Keys page."
              >
                <CodeBlock code={token} onCopy={copy} wrap />
              </OnboardingStep>

              <OnboardingStep
                number={2}
                title="Set up your environment"
                description="You may want to add this to your shell config for persistence:"
              >
                <CodeBlock code={`export HURRY_API_TOKEN="${token}"`} onCopy={copy} />
              </OnboardingStep>

              <OnboardingStep
                number={3}
                title="Start using Hurry"
                description="Replace your cargo commands with hurry:"
              >
                <div className="space-y-2">
                  <CodeBlock code="hurry cargo build" onCopy={copy} label="instead of: cargo build" />
                  <CodeBlock code="hurry cargo test" onCopy={copy} label="instead of: cargo test" />
                  <CodeBlock code="hurry cargo check" onCopy={copy} label="instead of: cargo check" />
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
  description: string;
  subdescription?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <div className="flex h-6 w-6 items-center justify-center rounded-full bg-accent-bg text-xs font-semibold text-accent-text">
          {props.number}
        </div>
        <div className="text-sm font-semibold text-content-primary">{props.title}</div>
      </div>
      <div className="ml-8 space-y-2">
        <div className="text-xs text-content-tertiary">{props.description}</div>
        {props.subdescription ? (
          <div className="text-xs text-content-muted">{props.subdescription}</div>
        ) : null}
        {props.children}
      </div>
    </div>
  );
}

function CodeBlock(props: { code: string; onCopy: (value: string) => void; label?: string; wrap?: boolean }) {
  return (
    <div className="group flex items-start justify-between gap-2 rounded-xl border border-border bg-surface-subtle px-3 py-2">
      <div className="flex-1 overflow-x-auto">
        <code className={`font-mono text-xs text-content-primary ${props.wrap ? "break-all" : "whitespace-nowrap"}`}>
          {props.code}
        </code>
        {props.label ? <span className="ml-2 text-xs text-content-muted">{props.label}</span> : null}
      </div>
      <button
        type="button"
        onClick={() => props.onCopy(props.code)}
        className="shrink-0 cursor-pointer rounded p-1 text-content-muted opacity-0 transition hover:bg-surface-raised hover:text-content-secondary group-hover:opacity-100"
        title="Copy"
      >
        <Copy className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
