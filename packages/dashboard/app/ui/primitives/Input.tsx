import React from "react";
import clsx from "clsx";

export function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      className={clsx(
        "h-10 w-full rounded-xl border border-border bg-surface-subtle px-3 text-sm text-content-primary",
        "placeholder:text-content-muted focus:border-border-accent-hover focus:bg-surface-subtle-hover focus:outline-none",
        props.className,
      )}
    />
  );
}
