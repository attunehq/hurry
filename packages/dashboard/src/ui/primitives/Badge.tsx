import React from "react";
import clsx from "clsx";

export function Badge(
  props: React.HTMLAttributes<HTMLSpanElement> & { tone?: "neon" | "muted" | "warn" },
) {
  const { className, tone = "muted", ...rest } = props;
  return (
    <span
      {...rest}
      className={clsx(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium",
        tone === "neon" ? "border-neon-500/35 bg-neon-500/10 text-neon-300" : "",
        tone === "warn" ? "border-amber-400/25 bg-amber-400/10 text-amber-200" : "",
        tone === "muted" ? "border-white/10 bg-white/5 text-slate-300" : "",
        className,
      )}
    />
  );
}
