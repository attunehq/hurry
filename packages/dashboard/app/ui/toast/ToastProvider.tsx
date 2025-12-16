import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { X } from "lucide-react";

type ToastKind = "info" | "success" | "error";

type ToastItem = {
  id: string;
  kind: ToastKind;
  title: string;
  detail?: string;
};

type ToastApi = {
  push: (toast: Omit<ToastItem, "id">) => void;
};

const ToastContext = createContext<ToastApi | null>(null);

export function ToastProvider(props: { children: React.ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const timeoutsRef = useRef<Map<string, number>>(new Map());

  const remove = useCallback((id: string) => {
    setItems((prev) => prev.filter((t) => t.id !== id));
    const timeoutId = timeoutsRef.current.get(id);
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
      timeoutsRef.current.delete(id);
    }
  }, []);

  const push = useCallback((toast: Omit<ToastItem, "id">) => {
    const id = crypto.randomUUID();
    setItems((prev) => [...prev, { ...toast, id }]);
    const timeoutId = window.setTimeout(() => {
      timeoutsRef.current.delete(id);
      remove(id);
    }, toast.kind === "error" ? 7000 : 4500);
    timeoutsRef.current.set(id, timeoutId);
  }, [remove]);

  // Cleanup all timeouts on unmount
  useEffect(() => {
    const timeouts = timeoutsRef.current;
    return () => {
      timeouts.forEach((timeoutId) => window.clearTimeout(timeoutId));
      timeouts.clear();
    };
  }, []);

  const api = useMemo(() => ({ push }), [push]);

  return (
    <ToastContext.Provider value={api}>
      {props.children}
      <div className="fixed right-4 top-4 z-50 flex w-90 max-w-[92vw] flex-col gap-2">
        {items.map((t) => (
          <div
            key={t.id}
            className={[
              "rounded-xl border border-border bg-surface-overlay p-4 shadow-glow-soft backdrop-blur",
              t.kind === "success" ? "ring-1 ring-accent-bold/30" : "",
              t.kind === "error" ? "ring-1 ring-red-500/30" : "",
            ].join(" ")}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="text-sm font-semibold text-content-primary">{t.title}</div>
                {t.detail ? (
                  <div className="mt-1 wrap-break-word text-xs text-content-tertiary">{t.detail}</div>
                ) : null}
              </div>
              <button
                className="rounded-md p-1 text-content-muted hover:bg-surface-subtle hover:text-content-secondary"
                onClick={() => remove(t.id)}
                aria-label="Dismiss"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}
