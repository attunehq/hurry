import { Gift, Sparkles } from "lucide-react";

import { Card, CardBody } from "../ui/primitives/Card";
import { PageLayout } from "../ui/shell/PageLayout";

export default function BillingPage() {
  return (
    <PageLayout
      title="Billing"
      subtitle="Manage your subscription and payment details."
    >
      <Card>
        <CardBody>
          <div className="flex flex-col items-center py-8 text-center">
            <div className="mb-4 grid h-16 w-16 place-items-center rounded-2xl border border-border-accent bg-accent-subtle">
              <Gift className="h-8 w-8 text-accent-text" />
            </div>
            <h2 className="text-lg font-semibold text-content-primary">
              Free During Early Access
            </h2>
            <p className="mt-2 max-w-md text-sm text-content-tertiary">
              Hurry is free to use while we're in our early access period. We'll
              give you plenty of notice before introducing any paid plans.
            </p>
            <div className="mt-6 flex items-center gap-2 rounded-full border border-border-accent bg-accent-subtle px-4 py-2 text-sm text-accent-text">
              <Sparkles className="h-4 w-4" />
              No payment required
            </div>
          </div>
        </CardBody>
      </Card>
    </PageLayout>
  );
}
