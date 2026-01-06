import type { CSSProperties, ReactNode } from "react";

const NOISE_SVG = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='220' height='220'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='220' height='220' filter='url(%23n)' opacity='.18'/%3E%3C/svg%3E")`;

const noiseOverlayStyle: CSSProperties = {
  content: '""',
  position: "fixed",
  inset: 0,
  pointerEvents: "none",
  backgroundImage: NOISE_SVG,
  mixBlendMode: "overlay",
  opacity: "var(--noise-opacity)",
};

export function Noise({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={className} style={{ position: "relative" }}>
      <div style={noiseOverlayStyle} aria-hidden="true" />
      {children}
    </div>
  );
}
