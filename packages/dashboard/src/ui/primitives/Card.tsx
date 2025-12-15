import React from "react";
import clsx from "clsx";

export function Card(props: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      {...props}
      className={clsx(
        "rounded-2xl border border-white/10 bg-ink-900/65 shadow-glow-soft backdrop-blur",
        props.className,
      )}
    />
  );
}

export function CardHeader(props: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div {...props} className={clsx("border-b border-white/10 p-5", props.className)} />
  );
}

export function CardBody(props: React.HTMLAttributes<HTMLDivElement>) {
  return <div {...props} className={clsx("p-5", props.className)} />;
}
