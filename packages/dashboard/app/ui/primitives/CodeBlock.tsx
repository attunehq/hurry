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
      <div className="group flex items-start justify-between gap-2 px-3 py-2">
        <code className={`flex-1 overflow-x-auto font-mono text-xs text-content-primary ${wrap ? "break-all" : "whitespace-nowrap"}`}>
          {code}
        </code>
        <button
          type="button"
          onClick={copy}
          className="shrink-0 cursor-pointer rounded p-1 text-content-muted opacity-0 transition hover:bg-surface-raised hover:text-content-secondary group-hover:opacity-100"
          title="Copy"
        >
          <Copy className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
