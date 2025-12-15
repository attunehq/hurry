import React, { useEffect, useRef } from "react";
import { X } from "lucide-react";

export function Modal(props: {
  open: boolean;
  title: string;
  children: React.ReactNode;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (props.open && !dialog.open) dialog.showModal();
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
  }, [props]);

  return (
    <dialog
      ref={ref}
      className="w-[640px] max-w-[92vw] rounded-2xl border border-white/10 bg-ink-900/90 p-0 text-slate-100 shadow-glow-soft backdrop:bg-black/60"
      onClose={props.onClose}
    >
      <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
        <div className="text-sm font-semibold">{props.title}</div>
        <button
          className="rounded-md p-1 text-slate-400 hover:bg-white/5 hover:text-slate-200"
          onClick={props.onClose}
          aria-label="Close"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div className="p-5">{props.children}</div>
    </dialog>
  );
}

