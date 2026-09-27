import type { BalanceStatus } from "../types/finance";

const statuses: Record<BalanceStatus, { label: string; style: string }> = {
  current: {
    label: "Recent balance",
    style: "border-emerald-700/50 bg-emerald-900/30 text-emerald-300",
  },
  stale: {
    label: "Stale balance",
    style: "border-amber-700/50 bg-amber-900/30 text-amber-300",
  },
  reauth_required: {
    label: "Authentication needed",
    style: "border-amber-700/50 bg-amber-900/30 text-amber-300",
  },
  error: {
    label: "Check needed",
    style: "border-red-700/50 bg-red-900/30 text-red-300",
  },
  missing: {
    label: "Data unavailable",
    style: "border-amber-700/50 bg-amber-900/30 text-amber-300",
  },
  unknown: {
    label: "Not verified",
    style: "border-slate-600 bg-slate-800 text-slate-300",
  },
};

export function formatConnectionTime(value: string | null): string {
  if (!value) return "Not available";
  const dateOnly = /^\d{4}-\d{2}-\d{2}$/.test(value);
  const date = new Date(dateOnly ? `${value}T00:00:00` : value);
  if (Number.isNaN(date.getTime())) return "Date unavailable";
  return new Intl.DateTimeFormat("en-CA", {
    dateStyle: "medium",
    ...(dateOnly ? {} : { timeStyle: "short" as const }),
  }).format(date);
}

export default function ConnectionStatus({ status }: { status: BalanceStatus }) {
  const { label, style } = statuses[status];
  return (
    <span className={`inline-flex rounded-full border px-2 py-0.5 text-[11px] font-medium ${style}`}>
      {label}
    </span>
  );
}
