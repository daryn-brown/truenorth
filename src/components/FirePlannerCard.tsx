import { useEffect, useState } from "react";
import type { FireInputs, FirePlan } from "../types/finance";

interface Props {
  plan: FirePlan | null;
  loading: boolean;
  onUpdate: (inputs: FireInputs) => Promise<void> | void;
}

type FormState = Record<keyof FireInputs, string>;

const usd = (value: number, maximumFractionDigits = 0) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);

const longDate = (iso: string) => {
  const d = new Date(`${iso}T00:00:00`);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString("en-US", { month: "short", year: "numeric" });
};

const toForm = (i: FireInputs): FormState => ({
  current_age: String(i.current_age),
  annual_expenses_usd: String(i.annual_expenses_usd),
  swr_pct: String(i.swr_pct),
  annual_return_pct: String(i.annual_return_pct),
  retirement_age: String(i.retirement_age),
  monthly_contribution_usd: String(i.monthly_contribution_usd),
});

const parseForm = (f: FormState): FireInputs | null => {
  const n = (s: string) => Number(s);
  const vals = {
    current_age: n(f.current_age),
    annual_expenses_usd: n(f.annual_expenses_usd),
    swr_pct: n(f.swr_pct),
    annual_return_pct: n(f.annual_return_pct),
    retirement_age: n(f.retirement_age),
    monthly_contribution_usd: n(f.monthly_contribution_usd),
  };
  return Object.values(vals).every((v) => Number.isFinite(v)) ? vals : null;
};

/**
 * A generic FIRE planner: FIRE number (expenses / SWR), CoastFIRE, and projected ages/dates from
 * net worth + a monthly contribution that defaults to your own pace. Ships neutral so anyone can
 * make it theirs; values persist locally.
 */
export default function FirePlannerCard({ plan, loading, onUpdate }: Props) {
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (plan && !editing) setForm(toForm(plan.inputs));
  }, [plan, editing]);

  if (loading && !plan) {
    return (
      <div className="rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-40 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!plan) return null;

  const setField = (key: keyof FireInputs, value: string) =>
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
    setForm(toForm(plan.inputs));
    setError(null);
    setEditing(false);
  };

  return (
    <div className="rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium uppercase tracking-widest text-slate-400">
            FIRE Planner
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            Financial independence on the {plan.inputs.swr_pct}% rule ·{" "}
            {plan.inputs.annual_return_pct}% return
          </p>
        </div>
        <span className="rounded-full border border-emerald-700/50 bg-emerald-900/30 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-emerald-300">
          FIRE
        </span>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3">
        <Milestone
          label="CoastFIRE"
          target={plan.coast_number}
          progress={plan.coast_progress}
          met={plan.already_coast}
          age={plan.coast_age}
          date={plan.coast_date}
          accent="indigo"
        />
        <Milestone
          label="Full FIRE"
          target={plan.fire_number}
          progress={plan.fire_progress}
          met={plan.already_fire}
          age={plan.fire_age}
          date={plan.fire_date}
          accent="emerald"
        />
      </div>

      <p className="mt-3 text-sm text-slate-300">
        At {usd(plan.current_usd)} invested and{" "}
        <span className="font-semibold text-emerald-400">
          {usd(plan.monthly_contribution_usd)}/mo
        </span>
        {plan.contribution_is_derived ? " (your current pace)" : ""}, you reach{" "}
        <span className="font-semibold text-white">{usd(plan.fire_number)}</span>
        {plan.fire_age !== null ? ` around age ${plan.fire_age}` : " — set a positive pace to see when"}
        . 🧭
      </p>

      <div className="mt-4 border-t border-slate-700/60 pt-3">
        <button
          onClick={() => (editing ? cancel() : setEditing(true))}
          className="text-xs font-medium text-slate-400 hover:text-slate-200"
        >
          {editing ? "× Close goals" : "⚙ Set my goals"}
        </button>

        {editing && form && (
          <div className="mt-3 space-y-3">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              <NumField
                label="Current age"
                suffix="yr"
                value={form.current_age}
                onChange={(v) => setField("current_age", v)}
              />
              <NumField
                label="Annual expenses"
                suffix="$/yr"
                value={form.annual_expenses_usd}
                onChange={(v) => setField("annual_expenses_usd", v)}
              />
              <NumField
                label="Withdrawal rate"
                suffix="%"
                value={form.swr_pct}
                onChange={(v) => setField("swr_pct", v)}
              />
              <NumField
                label="Return"
                suffix="%/yr"
                value={form.annual_return_pct}
                onChange={(v) => setField("annual_return_pct", v)}
              />
              <NumField
                label="Retire age"
                suffix="yr"
                value={form.retirement_age}
                onChange={(v) => setField("retirement_age", v)}
              />
              <NumField
                label="Contribution (0=auto)"
                suffix="$/mo"
                value={form.monthly_contribution_usd}
                onChange={(v) => setField("monthly_contribution_usd", v)}
              />
            </div>

            {error && <p className="text-xs text-red-400">{error}</p>}

            <div className="flex items-center gap-2">
              <button
                onClick={apply}
                disabled={saving}
                className="rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500 disabled:opacity-50"
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

interface MilestoneProps {
  label: string;
  target: number;
  progress: number;
  met: boolean;
  age: number | null;
  date: string | null;
  accent: "indigo" | "emerald";
}

function Milestone({ label, target, progress, met, age, date, accent }: MilestoneProps) {
  const bar = accent === "emerald" ? "bg-emerald-400" : "bg-indigo-400";
  const pct = Math.round(progress * 100);
  return (
    <div className="rounded-xl border border-slate-700/60 bg-slate-900/40 px-4 py-3">
      <p className="text-[11px] font-medium uppercase tracking-wider text-slate-400">{label}</p>
      <p className="mt-1 text-2xl font-bold tracking-tight text-white">{usd(target)}</p>
      <div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-slate-800">
        <div className={`h-full rounded-full ${bar} transition-all`} style={{ width: `${Math.max(2, Math.min(100, pct))}%` }} />
      </div>
      <p className="mt-1 text-xs text-slate-500">
        {met ? "🎉 reached" : age !== null ? `~age ${age} · ${date ? longDate(date) : ""}` : `${pct}%`}
      </p>
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
      <div className="mt-1 flex items-center rounded-lg border border-slate-700 bg-slate-900/60 px-2 focus-within:border-emerald-500">
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
