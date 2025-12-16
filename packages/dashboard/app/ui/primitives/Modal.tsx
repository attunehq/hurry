import React, { useEffect, useRef } from "react";
import { X } from "lucide-react";

export function Modal(props: {
  open: boolean;
  title: string;
  children: React.ReactNode;
  onClose: () => void;
  onSubmit?: () => void;
}) {
  const ref = useRef<HTMLDialogElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (props.open && !dialog.open) {
      dialog.showModal();
      const input = contentRef.current?.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>(
        "input, textarea, select"
      );
      input?.focus();
    }
    if (!props.open && dialog.open) dialog.close();
  }, [props.open]);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const onCancel = (e: Event) => {
      e.preventDefault();
      props.onClose();
    };
    dialog.addEventListener("cancel", onCancel);
    return () => dialog.removeEventListener("cancel", onCancel);
  }, [props.onClose]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && props.onSubmit) {
      e.preventDefault();
      props.onSubmit();
    }
  }

  return (
    <dialog
      ref={ref}
      className="fixed left-1/2 top-[20%] w-160 max-w-[92vw] -translate-x-1/2 rounded-2xl border border-border bg-surface-overlay p-0 text-content-primary shadow-glow-soft backdrop:bg-backdrop"
      onClose={props.onClose}
      onKeyDown={handleKeyDown}
    >
      <div className="flex items-center justify-between border-b border-border px-5 py-4">
        <div className="text-sm font-semibold">{props.title}</div>
        <button
          className="rounded-md p-1 text-content-muted hover:bg-surface-subtle hover:text-content-secondary"
          onClick={props.onClose}
          aria-label="Close"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div ref={contentRef} className="p-5">{props.children}</div>
    </dialog>
  );
}
