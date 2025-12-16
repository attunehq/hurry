import React from "react";
import clsx from "clsx";

type Variant = "primary" | "secondary" | "danger" | "warn" | "ghost";
type Size = "sm" | "md";

export function Button(
  props: React.ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: Variant;
    size?: Size;
  },
) {
  const { className, variant = "primary", size = "md", ...rest } = props;

  return (
    <button
      {...rest}
      className={clsx(
        "inline-flex cursor-pointer items-center justify-center gap-2 rounded-xl border px-3 font-medium transition",
        "disabled:cursor-not-allowed disabled:opacity-50",
        size === "sm" ? "h-9 text-sm" : "h-10 text-sm",
        variant === "primary"
          ? "border-border-accent bg-accent-subtle text-content-primary shadow-glow hover:bg-accent-subtle/80"
          : "",
        variant === "secondary"
          ? "border-border bg-surface-subtle text-content-primary hover:bg-surface-subtle-hover"
          : "",
        variant === "danger"
          ? "border-danger-border bg-danger-bg text-danger-text hover:bg-danger-bg-hover"
          : "",
        variant === "warn"
          ? "border-warn-border bg-warn-bg text-warn-text hover:bg-warn-bg-hover"
          : "",
        variant === "ghost"
          ? "border-transparent bg-transparent text-content-secondary hover:bg-surface-subtle"
          : "",
        className,
      )}
    />
  );
}
