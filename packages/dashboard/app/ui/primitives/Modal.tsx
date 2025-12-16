import React, { useCallback, useEffect, useRef } from "react";
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
  const onCloseRef = useRef(props.onClose);
  onCloseRef.current = props.onClose;

  const handleClose = useCallback(() => {
    onCloseRef.current();
  }, []);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;

    // showModal() puts the dialog in the top layer, making it interactive
    if (!dialog.open) {
      dialog.showModal();
    }

    // Focus the first input if present
    const input = contentRef.current?.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>(
      "input, textarea, select"
    );
    input?.focus();

    // Handle cancel (Escape key) - prevent default browser close and call our handler
    const onCancel = (e: Event) => {
      e.preventDefault();
      handleClose();
    };
    // Handle backdrop click - close modal when clicking outside the dialog content
    const onBackdropClick = (e: MouseEvent) => {
      // Only close if clicking directly on the dialog (the backdrop area)
      // not on its children
      if (e.target === dialog) {
        handleClose();
      }
    };
    dialog.addEventListener("cancel", onCancel);
    dialog.addEventListener("click", onBackdropClick);
    return () => {
      dialog.removeEventListener("cancel", onCancel);
      dialog.removeEventListener("click", onBackdropClick);
    };
  }, [handleClose]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && props.onSubmit) {
      e.preventDefault();
      props.onSubmit();
    }
  }

  // Don't render the dialog at all when closed
  if (!props.open) return null;

  return (
    <dialog
      ref={ref}
      className="m-auto mt-[20vh] w-160 max-w-[92vw] rounded-2xl border border-border bg-surface-overlay p-0 text-content-primary shadow-glow-soft backdrop:bg-backdrop"
      onClose={handleClose}
      onKeyDown={handleKeyDown}
    >
      <div className="flex items-center justify-between border-b border-border px-5 py-4">
        <div className="text-sm font-semibold">{props.title}</div>
        <button
          type="button"
          className="cursor-pointer rounded-md p-1 text-content-muted hover:bg-surface-subtle hover:text-content-secondary"
          onClick={handleClose}
          aria-label="Close"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div ref={contentRef} className="p-5">{props.children}</div>
    </dialog>
  );
}
