import type { ReactNode } from "react";

const TAB_STYLES = `
.tab-content-wrapper {
  view-transition-name: tab-content;
}

:root[data-tab-direction="right"]::view-transition-old(tab-content) {
  animation: tab-slide-out-left 0.15s ease-out forwards;
}

:root[data-tab-direction="right"]::view-transition-new(tab-content) {
  animation: tab-slide-in-right 0.15s ease-out forwards;
}

:root[data-tab-direction="left"]::view-transition-old(tab-content) {
  animation: tab-slide-out-right 0.15s ease-out forwards;
}

:root[data-tab-direction="left"]::view-transition-new(tab-content) {
  animation: tab-slide-in-left 0.15s ease-out forwards;
}

@keyframes tab-slide-out-left {
  to {
    opacity: 0;
    transform: translateX(-20px);
  }
}

@keyframes tab-slide-in-right {
  from {
    opacity: 0;
    transform: translateX(20px);
  }
}

@keyframes tab-slide-out-right {
  to {
    opacity: 0;
    transform: translateX(20px);
  }
}

@keyframes tab-slide-in-left {
  from {
    opacity: 0;
    transform: translateX(-20px);
  }
}
`;

export function TabContent({ children }: { children: ReactNode }) {
  return (
    <>
      <style>{TAB_STYLES}</style>
      <div className="tab-content-wrapper">{children}</div>
    </>
  );
}
