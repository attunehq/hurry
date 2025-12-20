import { ChevronLeft, ChevronRight, ScrollText } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router";

import { isUnauthorizedError } from "../api/client";
import type { AuditLogListResponse } from "../api/types";
import { useApi } from "../api/useApi";
import { Button } from "../ui/primitives/Button";
import { Card, CardBody, CardHeader } from "../ui/primitives/Card";
import { useToast } from "../ui/toast/ToastProvider";
import { useOrgContext } from "./org.$orgId";

const PAGE_SIZE = 25;

type Cursor = { time: string; id: string };

export default function OrgAuditLogPage() {
  const toast = useToast();
  const { request, signedIn } = useApi();
  const { orgId, role } = useOrgContext();
  const [data, setData] = useState<AuditLogListResponse | null>(null);
  const [loading, setLoading] = useState(false);

  // Use URL search params for cursor - enables shareable links and browser back/forward
  const [searchParams, setSearchParams] = useSearchParams();
  const cursorTime = searchParams.get("cursor_time");
  const cursorId = searchParams.get("cursor_id");

  // Track cursor history for "Newer" navigation without relying on browser history
  const [cursorHistory, setCursorHistory] = useState<Cursor[]>([]);

  const canAdmin = role === "admin";
  const entries = useMemo(() => data?.entries ?? [], [data]);
  const hasMore = data?.has_more ?? false;

  const load = useCallback(async () => {
    if (!signedIn) return;
    setLoading(true);
    try {
      let path = `/api/v1/organizations/${orgId}/audit-log?limit=${PAGE_SIZE}`;
      if (cursorTime && cursorId) {
        path += `&cursor_time=${encodeURIComponent(cursorTime)}&cursor_id=${cursorId}`;
      }
      const out = await request<AuditLogListResponse>({ path });
      setData(out);
    } catch (e) {
      if (isUnauthorizedError(e)) return;
      if (e && typeof e === "object" && "status" in e && (e as { status: number }).status === 403) {
        setData(null);
        return;
      }
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
      toast.push({ kind: "error", title: "Failed to load audit log", detail: msg });
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [signedIn, orgId, cursorTime, cursorId, request, toast]);

  function goNext() {
    if (!hasMore || entries.length === 0) return;
    const last = entries[entries.length - 1];
    // Save current cursor to history before navigating forward
    if (cursorTime && cursorId) {
      setCursorHistory((prev) => [...prev, { time: cursorTime, id: cursorId }]);
    } else {
      // First page has no cursor, use empty marker
      setCursorHistory((prev) => [...prev, { time: "", id: "" }]);
    }
    setSearchParams({ cursor_time: last.created_at, cursor_id: String(last.id) });
  }

  function goPrev() {
    if (cursorHistory.length === 0) return;
    const prev = cursorHistory[cursorHistory.length - 1];
    setCursorHistory((h) => h.slice(0, -1));
    if (prev.time === "" && prev.id === "") {
      // Go back to first page
      setSearchParams({});
    } else {
      setSearchParams({ cursor_time: prev.time, cursor_id: prev.id });
    }
  }

  function goFirst() {
    setCursorHistory([]);
    setSearchParams({});
  }

  useEffect(() => {
    void load();
  }, [load]);

  if (!canAdmin) {
    return (
      <Card>
        <CardBody>
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <ScrollText className="mb-4 h-12 w-12 text-content-muted" />
            <div className="text-sm font-medium text-content-primary">Admin access required</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Only organization admins can view the audit log.
            </div>
          </div>
        </CardBody>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm font-semibold text-content-primary">Audit Log</div>
            <div className="mt-1 text-sm text-content-tertiary">
              Track organization activity and security events.
            </div>
          </div>
          <div className="text-xs text-content-muted">
            {loading ? "Loading..." : `${entries.length} events`}
          </div>
        </div>
      </CardHeader>
      <CardBody>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="text-xs text-content-muted">
              <tr className="border-b border-border">
                <th className="py-2 pr-3">Time</th>
                <th className="py-2 pr-3">Actor</th>
                <th className="py-2 pr-3">Action</th>
                <th className="py-2 pr-3">Details</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.id} className="border-b border-border-subtle">
                  <td className="py-3 pr-3 text-content-secondary whitespace-nowrap">
                    {formatTimestamp(entry.created_at)}
                  </td>
                  <td className="py-3 pr-3">
                    <div className="flex flex-col">
                      <span className="font-medium text-content-primary">
                        {entry.account_name ?? entry.account_email ?? "System"}
                      </span>
                      {entry.account_email && entry.account_name ? (
                        <span className="text-xs text-content-muted">{entry.account_email}</span>
                      ) : null}
                    </div>
                  </td>
                  <td className="py-3 pr-3">
                    <span className="rounded bg-surface-subtle px-1.5 py-0.5 text-xs text-content-secondary">
                      {entry.action}
                    </span>
                  </td>
                  <td className="py-3 pr-3 text-content-tertiary">
                    {entry.details ? (
                      <DetailsDisplay details={entry.details} />
                    ) : (
                      <span className="text-content-muted">-</span>
                    )}
                  </td>
                </tr>
              ))}
              {entries.length === 0 && !loading ? (
                <tr>
                  <td colSpan={4} className="py-6 text-center text-sm text-content-muted">
                    No audit events yet.
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>

        <div className="mt-4 flex items-center justify-between border-t border-border-subtle pt-4">
          <div className="text-xs text-content-muted">
            {cursorHistory.length > 0 ? (
              <button
                type="button"
                className="cursor-pointer text-accent-text hover:underline"
                onClick={goFirst}
              >
                Back to latest
              </button>
            ) : (
              "Showing latest events"
            )}
          </div>
          <div className="flex gap-2">
            <Button
              variant="secondary"
              size="sm"
              disabled={cursorHistory.length === 0}
              onClick={goPrev}
            >
              <ChevronLeft className="h-4 w-4" />
              Newer
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={!hasMore}
              onClick={goNext}
            >
              Older
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </CardBody>
    </Card>
  );
}

function formatTimestamp(iso: string): string {
  const date = new Date(iso);
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const MAX_VISIBLE_DETAILS = 3;

function DetailsDisplay({ details }: { details: Record<string, unknown> }) {
  const items = Object.entries(details).filter(([, v]) => v != null);
  if (items.length === 0) return <span className="text-content-muted">-</span>;

  return (
    <div className="flex flex-wrap gap-1">
      {items.slice(0, MAX_VISIBLE_DETAILS).map(([key, value]) => (
        <span key={key} className="rounded bg-surface-subtle px-1.5 py-0.5 text-xs">
          <span className="text-content-muted">{key}:</span>{" "}
          <span className="text-content-secondary">{String(value)}</span>
        </span>
      ))}
      {items.length > MAX_VISIBLE_DETAILS ? (
        <span className="text-xs text-content-muted">+{items.length - MAX_VISIBLE_DETAILS} more</span>
      ) : null}
    </div>
  );
}
