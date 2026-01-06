import type { CSSProperties, ReactNode } from "react";

type DropdownProps = {
  open: boolean;
  children: ReactNode;
  className?: string;
};

const baseStyle: CSSProperties = {
  transformOrigin: "top",
  transition: "opacity 0.1s ease-out, transform 0.1s ease-out",
};

const openStyle: CSSProperties = {
  ...baseStyle,
  opacity: 1,
  transform: "scale(1) translateY(0)",
  pointerEvents: "auto",
};

const closedStyle: CSSProperties = {
  ...baseStyle,
  opacity: 0,
  transform: "scale(0.95) translateY(-4px)",
  pointerEvents: "none",
};

export function Dropdown({ open, children, className }: DropdownProps) {
  return (
    <div className={className} style={open ? openStyle : closedStyle}>
      {children}
    </div>
  );
}
