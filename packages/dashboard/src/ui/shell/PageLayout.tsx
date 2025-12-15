import type { ReactNode } from "react";

interface PageLayoutProps {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}

export function PageLayout({ title, subtitle, actions, children }: PageLayoutProps) {
  return (
    <>
      {/* Header row - rendered in grid-area: header */}
      <div className="contents">
        <div className="flex flex-col items-start justify-between gap-4 md:flex-row md:items-center [grid-area:header]">
          <div>
            <h1 className="text-2xl font-semibold text-content-primary">{title}</h1>
            {subtitle ? (
              <p className="mt-1.5 text-sm text-content-tertiary">{subtitle}</p>
            ) : null}
          </div>
          {actions}
        </div>
      </div>
      {/* Content row - rendered in grid-area: content */}
      <div className="space-y-8 [grid-area:content]">{children}</div>
    </>
  );
}
