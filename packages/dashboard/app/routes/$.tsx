import { ArrowLeft } from "lucide-react";
import { useNavigate } from "react-router";

export default function NotFoundPage() {
  const nav = useNavigate();

  return (
    <div className="noise fixed inset-0 z-50 flex items-start justify-center bg-surface-base px-6 pt-[20vh]">
      <div className="w-full max-w-md rounded-2xl border border-border bg-surface-raised p-6 shadow-glow-soft backdrop-blur">
        <div className="text-sm font-semibold text-content-primary">Not found</div>
        <div className="mt-2 text-sm text-content-tertiary">
          Even at top speed, we couldn't find this one.
        </div>
        <button
          onClick={() => nav(-1)}
          className="mt-4 flex cursor-pointer items-center gap-2 rounded-xl px-3 py-2 text-sm text-content-tertiary transition hover:bg-surface-subtle hover:text-content-primary"
        >
          <ArrowLeft className="h-4 w-4" />
          Go back
        </button>
      </div>
    </div>
  );
}
