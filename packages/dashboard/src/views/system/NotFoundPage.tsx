export function NotFoundPage() {
  return (
    <div className="mx-auto max-w-2xl">
      <div className="rounded-2xl border border-white/10 bg-ink-900/60 p-6 shadow-glow-soft backdrop-blur">
        <div className="text-sm font-semibold text-slate-100">Not found</div>
        <div className="mt-2 text-sm text-slate-300">
          This route doesn’t exist (or failed to load).
        </div>
      </div>
    </div>
  );
}

