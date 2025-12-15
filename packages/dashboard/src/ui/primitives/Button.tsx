import React from "react";
import clsx from "clsx";

type Variant = "primary" | "secondary" | "danger" | "ghost";
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
        "inline-flex items-center justify-center gap-2 rounded-xl border px-3 font-medium transition",
        "disabled:cursor-not-allowed disabled:opacity-50",
        size === "sm" ? "h-9 text-sm" : "h-10 text-sm",
        variant === "primary"
          ? "border-neon-500/35 bg-neon-500/15 text-slate-100 shadow-glow hover:bg-neon-500/20"
          : "",
        variant === "secondary"
          ? "border-white/10 bg-white/5 text-slate-100 hover:bg-white/8"
          : "",
        variant === "danger"
          ? "border-red-500/30 bg-red-500/10 text-red-100 hover:bg-red-500/15"
          : "",
        variant === "ghost"
          ? "border-transparent bg-transparent text-slate-200 hover:bg-white/5"
          : "",
        className,
      )}
    />
  );
}
