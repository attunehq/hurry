import React from "react";
import clsx from "clsx";

export function Label(props: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return (
    <label
      {...props}
      className={clsx("mb-1 block text-xs font-medium text-content-tertiary", props.className)}
    />
  );
}
