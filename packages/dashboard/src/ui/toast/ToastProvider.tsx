import React, { createContext, useCallback, useContext, useMemo, useState } from "react";
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

  const remove = useCallback((id: string) => {
    setItems((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const push = useCallback((toast: Omit<ToastItem, "id">) => {
    const id = crypto.randomUUID();
    setItems((prev) => [...prev, { ...toast, id }]);
    window.setTimeout(() => remove(id), toast.kind === "error" ? 7000 : 4500);
  }, [remove]);

  const api = useMemo(() => ({ push }), [push]);

  return (
    <ToastContext.Provider value={api}>
      {props.children}
      <div className="fixed right-4 top-4 z-50 flex w-[360px] max-w-[92vw] flex-col gap-2">
        {items.map((t) => (
          <div
            key={t.id}
            className={[
              "rounded-xl border border-white/10 bg-ink-900/80 p-4 shadow-glow-soft backdrop-blur",
              t.kind === "success" ? "ring-1 ring-aurora-400/30" : "",
              t.kind === "error" ? "ring-1 ring-red-500/30" : "",
            ].join(" ")}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="text-sm font-semibold text-slate-100">{t.title}</div>
                {t.detail ? (
                  <div className="mt-1 break-words text-xs text-slate-300">{t.detail}</div>
                ) : null}
              </div>
              <button
                className="rounded-md p-1 text-slate-400 hover:bg-white/5 hover:text-slate-200"
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

