import { Copy } from "lucide-react";

import { useToast } from "../toast/ToastProvider";

type CodeBlockProps = {
  code: string;
  label?: string;
  wrap?: boolean;
};

export function CodeBlock({ code, label, wrap }: CodeBlockProps) {
  const toast = useToast();

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      toast.push({ kind: "success", title: "Copied" });
    } catch {
      toast.push({ kind: "error", title: "Copy failed" });
    }
  }

  return (
    <div className="rounded-lg border border-border bg-surface-subtle">
      {label && (
        <div className="border-b border-border px-3 py-1.5 text-xs text-content-muted">
          {label}
        </div>
      )}
      <div className="group flex items-center justify-between gap-2 px-3 py-2">
        <code className={`flex-1 overflow-x-auto font-mono text-xs text-content-primary ${wrap ? "break-all" : "whitespace-nowrap"}`}>
          {code}
        </code>
        <button
          type="button"
          onClick={copy}
          className="shrink-0 cursor-pointer text-content-muted opacity-0 transition hover:text-content-secondary group-hover:opacity-100"
          title="Copy"
          aria-label="Copy to clipboard"
        >
          <Copy className="h-3 w-3" />
        </button>
      </div>
    </div>
  );
}
