import { Gift, Sparkles } from "lucide-react";

import { Card, CardBody } from "../../ui/primitives/Card";

export function BillingPage() {
  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-2xl font-semibold text-slate-100">Billing</h1>
        <p className="mt-1.5 text-sm text-slate-300">
          Manage your subscription and payment details.
        </p>
      </div>

      <Card>
        <CardBody>
          <div className="flex flex-col items-center py-8 text-center">
            <div className="mb-4 grid h-16 w-16 place-items-center rounded-2xl border border-neon-500/20 bg-neon-500/10">
              <Gift className="h-8 w-8 text-neon-300" />
            </div>
            <h2 className="text-lg font-semibold text-slate-100">
              Free During Early Access
            </h2>
            <p className="mt-2 max-w-md text-sm text-slate-300">
              Hurry is free to use while we're in our early access period. We'll
              give you plenty of notice before introducing any paid plans.
            </p>
            <div className="mt-6 flex items-center gap-2 rounded-full border border-neon-500/20 bg-neon-500/10 px-4 py-2 text-sm text-neon-300">
              <Sparkles className="h-4 w-4" />
              No payment required
            </div>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}
