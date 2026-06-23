import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import type { Currency } from "../types/finance";

interface DataPoint {
  date: string;
  value: number;
}

interface Props {
  data: DataPoint[];
  currency: Currency;
  /** When provided, the empty state offers a button to reconstruct history from transactions. */
  onBackfill?: () => void;
  backfilling?: boolean;
  /** Optional message shown under the empty state (e.g. the result of a backfill attempt). */
  note?: string | null;
}

const fmt = (value: number, currency: Currency) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);

export default function NetWorthChart({
  data,
  currency,
  onBackfill,
  backfilling = false,
  note,
}: Props) {
  if (data.length < 2) {
    return (
      <div className="flex h-40 flex-col items-center justify-center gap-3 rounded-2xl border border-slate-700 bg-slate-800/40 px-4 text-center text-sm text-slate-500">
        <span>Add balance snapshots over time to see your net worth chart.</span>
        {onBackfill && (
          <button
            onClick={onBackfill}
            disabled={backfilling}
            title="Reconstruct past balances from your transaction history"
            className="flex items-center gap-1.5 rounded-lg border border-indigo-700/60 bg-indigo-900/30 px-3 py-1.5 text-xs text-indigo-200 hover:bg-indigo-900/60 disabled:opacity-50 transition-colors"
          >
            {backfilling ? "Reconstructing…" : "↺ Reconstruct from transactions"}
          </button>
        )}
        {note && <span className="text-xs text-slate-500">{note}</span>}
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-slate-700 bg-slate-800/40 p-5">
      <p className="mb-3 text-xs font-semibold uppercase tracking-widest text-slate-400">
        Net Worth Over Time ({currency})
      </p>
      <ResponsiveContainer width="100%" height={180}>
        <AreaChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
          <defs>
            <linearGradient id="nwGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor="#6366f1" stopOpacity={0.3} />
              <stop offset="95%" stopColor="#6366f1" stopOpacity={0} />
            </linearGradient>
          </defs>
          <XAxis
            dataKey="date"
            tick={{ fill: "#64748b", fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: string) => v.slice(5)}
          />
          <YAxis
            tick={{ fill: "#64748b", fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: number) => fmt(v, currency)}
            width={70}
          />
          <Tooltip
            contentStyle={{
              background: "#1e293b",
              border: "1px solid #334155",
              borderRadius: 8,
              fontSize: 12,
            }}
            formatter={(v: number) => [fmt(v, currency), "Net Worth"]}
          />
          <Area
            type="monotone"
            dataKey="value"
            stroke="#6366f1"
            strokeWidth={2}
            fill="url(#nwGrad)"
            dot={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
