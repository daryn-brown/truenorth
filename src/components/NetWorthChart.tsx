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
}

const fmt = (value: number, currency: Currency) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);

export default function NetWorthChart({ data, currency }: Props) {
  if (data.length < 2) {
    return (
      <div className="tn-card tn-card--chart flex h-40 flex-col items-center justify-center gap-3 rounded-2xl border border-slate-700 bg-slate-800/40 px-4 text-center text-sm text-slate-500">
        <span>
          Add balance snapshots over time — by syncing a connection or importing history — to see
          your net worth chart.
        </span>
      </div>
    );
  }

  return (
    <div className="tn-card tn-card--chart net-worth-chart rounded-2xl border border-slate-700 bg-slate-800/40 p-5">
      <p className="mb-3 text-xs font-semibold uppercase tracking-widest text-slate-400">
        Net Worth Over Time ({currency})
      </p>
      <ResponsiveContainer width="100%" height={180}>
        <AreaChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
          <defs>
            <linearGradient id="nwGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor="#9d7cff" stopOpacity={0.38} />
              <stop offset="60%" stopColor="#6f65db" stopOpacity={0.1} />
              <stop offset="95%" stopColor="#5578ff" stopOpacity={0} />
            </linearGradient>
            <linearGradient id="nwLine" x1="0" y1="0" x2="1" y2="0">
              <stop offset="0%" stopColor="#65e9c5" />
              <stop offset="54%" stopColor="#9d7cff" />
              <stop offset="100%" stopColor="#e378d3" />
            </linearGradient>
          </defs>
          <XAxis
            dataKey="date"
            tick={{ fill: "#746d80", fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: string) => v.slice(5)}
          />
          <YAxis
            domain={["dataMin", "dataMax"]}
            tick={{ fill: "#746d80", fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: number) => fmt(v, currency)}
            width={70}
          />
          <Tooltip
            contentStyle={{
              background: "#0d0b1a",
              border: "1px solid rgba(198,168,255,.18)",
              borderRadius: 12,
              fontSize: 12,
            }}
            formatter={(v: number) => [fmt(v, currency), "Net Worth"]}
          />
          <Area
            type="monotone"
            dataKey="value"
            stroke="url(#nwLine)"
            strokeWidth={3}
            fill="url(#nwGrad)"
            dot={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
