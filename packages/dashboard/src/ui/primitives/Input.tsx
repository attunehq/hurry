import React from "react";
import clsx from "clsx";

export function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      className={clsx(
        "h-10 w-full rounded-xl border border-white/10 bg-white/5 px-3 text-sm text-slate-100",
        "placeholder:text-slate-500 focus:border-neon-500/50 focus:bg-white/6 focus:outline-none",
        props.className,
      )}
    />
  );
}
