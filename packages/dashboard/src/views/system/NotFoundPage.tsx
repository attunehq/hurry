import { ArrowLeft } from "lucide-react";
import { useNavigate } from "react-router-dom";

export function NotFoundPage() {
  const nav = useNavigate();

  return (
    <div className="flex min-h-screen items-center justify-center px-6">
      <div className="w-full max-w-md rounded-2xl border border-white/10 bg-ink-900/60 p-6 shadow-glow-soft backdrop-blur">
        <div className="text-sm font-semibold text-slate-100">Not found</div>
        <div className="mt-2 text-sm text-slate-300">
          Even at top speed, we couldn't find this one.
        </div>
        <button
          onClick={() => nav(-1)}
          className="mt-4 flex items-center gap-2 rounded-xl px-3 py-2 text-sm text-slate-300 transition hover:bg-white/5 hover:text-slate-100"
        >
          <ArrowLeft className="h-4 w-4" />
          Go back
        </button>
      </div>
    </div>
  );
}

