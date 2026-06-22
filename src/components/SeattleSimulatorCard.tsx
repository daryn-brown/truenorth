import { useEffect, useState } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { SeattleAssumptions, SeattleProjection } from "../types/finance";

interface Props {
  projection: SeattleProjection | null;
  loading: boolean;
  onUpdate: (assumptions: SeattleAssumptions) => Promise<void> | void;
}

type FocusMode = "both" | "current" | "seattle";
type FormState = Record<keyof SeattleAssumptions, string>;

const usd = (value: number, maximumFractionDigits = 0) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);

const usdCompact = (value: number) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);

const monthLabel = (iso: string) => {
  const d = new Date(`${iso}T00:00:00`);
  if (Number.isNaN(d.getTime())) return iso;
  const mon = d.toLocaleDateString("en-US", { month: "short" });
  return `${mon} '${String(d.getFullYear()).slice(2)}`;
};

const humanizeMonths = (m: number) =>
  m % 12 === 0 ? `${m / 12} yr${m / 12 === 1 ? "" : "s"}` : `${m} mo`;

const toForm = (a: SeattleAssumptions): FormState => ({
  current_net_monthly_usd: String(a.current_net_monthly_usd),
  current_expenses_monthly_usd: String(a.current_expenses_monthly_usd),
  seattle_net_monthly_usd: String(a.seattle_net_monthly_usd),
  seattle_expenses_monthly_usd: String(a.seattle_expenses_monthly_usd),
  transition_months: String(a.transition_months),
  horizon_months: String(a.horizon_months),
  annual_return_pct: String(a.annual_return_pct),
});

const parseForm = (f: FormState): SeattleAssumptions | null => {
  const n = (s: string) => Number(s);
  const vals = {
    current_net_monthly_usd: n(f.current_net_monthly_usd),
    current_expenses_monthly_usd: n(f.current_expenses_monthly_usd),
    seattle_net_monthly_usd: n(f.seattle_net_monthly_usd),
    seattle_expenses_monthly_usd: n(f.seattle_expenses_monthly_usd),
    transition_months: Math.round(n(f.transition_months)),
    horizon_months: Math.round(n(f.horizon_months)),
    annual_return_pct: n(f.annual_return_pct),
  };
  return Object.values(vals).every((v) => Number.isFinite(v)) ? vals : null;
};

/**
 * The "Seattle Transition" simulator — projects net worth forward under two scenarios so the user
 * can see the cost of dropping Job 2 and localizing the Microsoft salary to USD, while staying
 * reassured the trajectory keeps climbing.
 */
export default function SeattleSimulatorCard({ projection, loading, onUpdate }: Props) {
  const [focus, setFocus] = useState<FocusMode>("both");
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Keep the editor seeded with the latest persisted assumptions while it's closed; don't clobber
  // in-progress edits.
  useEffect(() => {
    if (projection && !editing) setForm(toForm(projection.assumptions));
  }, [projection, editing]);

  if (loading && !projection) {
    return (
      <div className="rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-56 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!projection) return null;

  const a = projection.assumptions;
  const horizonLabel = humanizeMonths(a.horizon_months);
  const endDate =
    projection.points[projection.points.length - 1]?.date ?? projection.start_date;
  const showCurrent = focus !== "seattle";
  const showSeattle = focus !== "current";

  const chartData = projection.points.map((p) => ({
    date: p.date,
    current: p.current_usd,
    seattle: p.seattle_usd,
  }));
  const tickInterval = Math.max(0, Math.floor(projection.points.length / 6));

  const setField = (key: keyof SeattleAssumptions, value: string) =>
    setForm((f) => (f ? { ...f, [key]: value } : f));

  const apply = async () => {
    if (!form) return;
    const parsed = parseForm(form);
    if (!parsed) {
      setError("Enter valid numbers for every field.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onUpdate(parsed);
      setEditing(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const cancel = () => {
    setForm(toForm(a));
    setError(null);
    setEditing(false);
  };

  return (
    <div className="rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      {/* Header */}
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium uppercase tracking-widest text-slate-400">
            Seattle Transition
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            What happens to net worth when Job 2 ends and Microsoft localizes to USD
          </p>
        </div>
        <span className="rounded-full border border-amber-700/50 bg-amber-900/30 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-amber-300">
          Simulator
        </span>
      </div>

      {/* Headline scenarios at the horizon */}
      <div className="mt-4 grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-indigo-700/40 bg-indigo-950/30 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-indigo-300">
            Keep both jobs · {horizonLabel}
          </p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">
            {usd(projection.current_end_usd)}
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            +{usd(projection.current_monthly_contribution_usd)}/mo saved
          </p>
        </div>
        <div className="rounded-xl border border-amber-700/40 bg-amber-950/20 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-amber-300">
            After Seattle move · {horizonLabel}
          </p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">
            {usd(projection.seattle_end_usd)}
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            {projection.end_gap_usd < 0
              ? `${usd(projection.end_gap_usd)} vs. both jobs`
              : "on par with both jobs"}
          </p>
        </div>
      </div>

      {/* Scenario focus toggle */}
      <div className="mt-4 flex items-center gap-1 rounded-lg border border-slate-700 bg-slate-900/40 p-1 text-xs">
        {(
          [
            ["both", "Both"],
            ["current", "Keep 2 jobs"],
            ["seattle", "Seattle only"],
          ] as [FocusMode, string][]
        ).map(([mode, label]) => (
          <button
            key={mode}
            onClick={() => setFocus(mode)}
            className={`flex-1 rounded-md px-2 py-1 font-medium transition-colors ${
              focus === mode
                ? "bg-slate-700 text-white"
                : "text-slate-400 hover:text-slate-200"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Projection chart */}
      <div className="mt-4">
        <ResponsiveContainer width="100%" height={200}>
          <LineChart data={chartData} margin={{ top: 6, right: 8, bottom: 0, left: 0 }}>
            <CartesianGrid stroke="#1e293b" vertical={false} />
            <XAxis
              dataKey="date"
              tick={{ fill: "#64748b", fontSize: 11 }}
              tickLine={false}
              axisLine={false}
              interval={tickInterval}
              tickFormatter={monthLabel}
            />
            <YAxis
              tick={{ fill: "#64748b", fontSize: 11 }}
              tickLine={false}
              axisLine={false}
              width={52}
              tickFormatter={(v: number) => usdCompact(v)}
            />
            <Tooltip
              contentStyle={{
                background: "#1e293b",
                border: "1px solid #334155",
                borderRadius: 8,
                fontSize: 12,
              }}
              labelFormatter={(v: string) => monthLabel(v)}
              formatter={(v: number, name: string) => [
                usd(v),
                name === "current" ? "Both jobs" : "Seattle",
              ]}
            />
            <ReferenceLine
              x={projection.transition_date}
              stroke="#f59e0b"
              strokeDasharray="4 4"
              label={{ value: "Move", fill: "#f59e0b", fontSize: 11, position: "top" }}
            />
            {showCurrent && (
              <Line
                type="monotone"
                dataKey="current"
                stroke="#6366f1"
                strokeWidth={2}
                dot={false}
              />
            )}
            {showSeattle && (
              <Line
                type="monotone"
                dataKey="seattle"
                stroke="#f59e0b"
                strokeWidth={2}
                dot={false}
              />
            )}
          </LineChart>
        </ResponsiveContainer>
      </div>

      {/* Reassurance */}
      <p className="mt-3 text-sm text-slate-300">
        Even after Job 2 ends around{" "}
        <span className="font-semibold text-amber-300">
          {monthLabel(projection.transition_date)}
        </span>
        , you keep saving{" "}
        <span className="font-semibold text-emerald-400">
          {usd(projection.seattle_monthly_contribution_usd)}/mo
        </span>{" "}
        and still climb to{" "}
        <span className="font-semibold text-white">{usd(projection.seattle_end_usd)}</span> by{" "}
        {monthLabel(endDate)}. 🧭
      </p>

      {/* Assumptions editor */}
      <div className="mt-4 border-t border-slate-700/60 pt-3">
        <button
          onClick={() => (editing ? cancel() : setEditing(true))}
          className="text-xs font-medium text-slate-400 hover:text-slate-200"
        >
          {editing ? "× Close assumptions" : "⚙ Adjust assumptions"}
        </button>

        {editing && form && (
          <div className="mt-3 space-y-3">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              <NumField
                label="Net income now"
                suffix="$/mo"
                value={form.current_net_monthly_usd}
                onChange={(v) => setField("current_net_monthly_usd", v)}
              />
              <NumField
                label="Expenses now"
                suffix="$/mo"
                value={form.current_expenses_monthly_usd}
                onChange={(v) => setField("current_expenses_monthly_usd", v)}
              />
              <NumField
                label="Return"
                suffix="%/yr"
                value={form.annual_return_pct}
                onChange={(v) => setField("annual_return_pct", v)}
              />
              <NumField
                label="Net income (Seattle)"
                suffix="$/mo"
                value={form.seattle_net_monthly_usd}
                onChange={(v) => setField("seattle_net_monthly_usd", v)}
              />
              <NumField
                label="Expenses (Seattle)"
                suffix="$/mo"
                value={form.seattle_expenses_monthly_usd}
                onChange={(v) => setField("seattle_expenses_monthly_usd", v)}
              />
              <div className="grid grid-cols-2 gap-2">
                <NumField
                  label="Move in"
                  suffix="mo"
                  value={form.transition_months}
                  onChange={(v) => setField("transition_months", v)}
                />
                <NumField
                  label="Horizon"
                  suffix="mo"
                  value={form.horizon_months}
                  onChange={(v) => setField("horizon_months", v)}
                />
              </div>
            </div>

            {error && <p className="text-xs text-red-400">{error}</p>}

            <div className="flex items-center gap-2">
              <button
                onClick={apply}
                disabled={saving}
                className="rounded-lg bg-indigo-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-indigo-500 disabled:opacity-50"
              >
                {saving ? "Saving…" : "Apply"}
              </button>
              <button
                onClick={cancel}
                disabled={saving}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-400 hover:bg-slate-800 disabled:opacity-50"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

interface NumFieldProps {
  label: string;
  suffix: string;
  value: string;
  onChange: (value: string) => void;
}

function NumField({ label, suffix, value, onChange }: NumFieldProps) {
  return (
    <label className="block">
      <span className="text-[11px] font-medium text-slate-400">{label}</span>
      <div className="mt-1 flex items-center rounded-lg border border-slate-700 bg-slate-900/60 px-2 focus-within:border-indigo-500">
        <input
          type="number"
          inputMode="decimal"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full bg-transparent py-1.5 text-sm text-white outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none"
        />
        <span className="pl-1 text-[11px] text-slate-500">{suffix}</span>
      </div>
    </label>
  );
}
