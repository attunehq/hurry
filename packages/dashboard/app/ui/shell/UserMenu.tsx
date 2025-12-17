import { ChevronDown, LogOut, User } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router";

import { useApi } from "../../api/useApi";
import { useUser } from "../../user/UserContext";

export function UserMenu() {
  const { user, loading } = useUser();
  const { logout } = useApi();
  const [open, setOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const handleClickOutside = useCallback((event: MouseEvent) => {
    if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
      setOpen(false);
    }
  }, []);

  useEffect(() => {
    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [open, handleClickOutside]);

  const displayName = user?.name?.trim() || user?.email || "Account";

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface-raised px-3 py-2 text-sm text-content-secondary transition hover:border-border-accent-hover hover:text-content-primary ${open ? "shadow-dropdown" : ""}`}
      >
        <User className="h-4 w-4" />
        <span className="max-w-48 truncate">
          {loading ? "Loading…" : displayName}
        </span>
        <ChevronDown className={`h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <div className="animate-dropdown absolute right-0 top-full z-50 mt-1 min-w-48 rounded-xl border border-border bg-surface-overlay p-1 shadow-dropdown backdrop-blur-xl">
          <Link
            to="/user"
            onClick={() => setOpen(false)}
            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-content-tertiary hover:bg-surface-subtle hover:text-content-primary"
          >
            <User className="h-4 w-4" />
            Account details
          </Link>

          <div className="my-1 border-t border-border" />

          <button
            type="button"
            onClick={() => {
              setOpen(false);
              void logout();
            }}
            className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-content-tertiary hover:bg-surface-subtle hover:text-content-primary"
          >
            <LogOut className="h-4 w-4" />
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
