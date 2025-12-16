import React from "react";
import clsx from "clsx";

export function Badge(
  props: React.HTMLAttributes<HTMLSpanElement> & { tone?: "neon" | "accent" | "muted" | "warn" },
) {
  const { className, tone = "muted", ...rest } = props;
  const isAccent = tone === "neon" || tone === "accent";
  return (
    <span
      {...rest}
      className={clsx(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium",
        isAccent ? "border-border-accent bg-accent-subtle text-accent-text" : "",
        tone === "warn" ? "border-amber-400/25 bg-amber-400/10 text-amber-600 dark:text-amber-200" : "",
        tone === "muted" ? "border-border bg-surface-subtle text-content-tertiary" : "",
        className,
      )}
    />
  );
}
