import { useId, type ReactNode } from "react";

const NOISE_SVG = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='220' height='220'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='220' height='220' filter='url(%23n)' opacity='.18'/%3E%3C/svg%3E")`;

export function Noise({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  const id = useId();

  const styles = `
#${id}::before {
  content: "";
  position: fixed;
  inset: 0;
  pointer-events: none;
  background-image: ${NOISE_SVG};
  mix-blend-mode: overlay;
  opacity: var(--noise-opacity);
}
`;

  return (
    <>
      <style href={`Noise-${id}`} precedence="medium">{styles}</style>
      <div id={id} className={className}>{children}</div>
    </>
  );
}
