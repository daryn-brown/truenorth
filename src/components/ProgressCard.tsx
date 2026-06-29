import { useEffect, useState } from "react";
import type { ProgressInputs, ProgressMetrics } from "../types/finance";

interface Props {
  metrics: ProgressMetrics | null;
  loading: boolean;
  onUpdate: (inputs: ProgressInputs) => Promise<void> | void;
}

type FormState = Record<keyof ProgressInputs, string>;

const usd = (value: number, maximumFractionDigits = 0) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);

const fmtMultiple = (m: number) => (m < 1 ? `${m}×` : `${m}×`);

const toForm = (i: ProgressInputs): FormState => ({
  base_salary_usd: String(i.base_salary_usd),
  monthly_expenses_usd: String(i.monthly_expenses_usd),
  years_earning: String(i.years_earning),
});

const parseForm = (f: FormState): ProgressInputs | null => {
  const vals = {
    base_salary_usd: Number(f.base_salary_usd),
    monthly_expenses_usd: Number(f.monthly_expenses_usd),
    years_earning: Number(f.years_earning),
  };
  return Object.values(vals).every((v) => Number.isFinite(v)) ? vals : null;
};

const runway = (months: number | null, years: number | null): string => {
  if (months === null) return "—";
  if (months < 24) return `${months.toFixed(months < 10 ? 1 : 0)} mo`;
  return `${(years ?? months / 12).toFixed(1)} yr`;
};

/**
 * Forward-looking progress: freedom runway (months/years your net worth buys) + salary milestones
 * (0.5×–5×). Neutral defaults; the user enters their own salary/expenses.
 */
export default function ProgressCard({ metrics, loading, onUpdate }: Props) {
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (metrics && !editing) setForm(toForm(metrics.inputs));
  }, [metrics, editing]);

  if (loading && !metrics) {
    return (
      <div className="flex h-full flex-col rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-40 flex-1 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!metrics) return null;

  const nextIdx = metrics.milestones.findIndex((m) => !m.reached);
  const next = nextIdx >= 0 ? metrics.milestones[nextIdx] : null;

  const setField = (key: keyof ProgressInputs, value: string) =>
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
    setForm(toForm(metrics.inputs));
    setError(null);
    setEditing(false);
  };

  return (
    <div className="flex h-full flex-col rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium uppercase tracking-widest text-slate-400">Progress</p>
          <p className="mt-0.5 text-xs text-slate-500">Freedom runway & salary milestones</p>
        </div>
        <span className="rounded-full border border-emerald-700/50 bg-emerald-900/30 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-emerald-300">
          {metrics.salary_multiple.toFixed(1)}× salary
        </span>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-emerald-700/40 bg-emerald-950/20 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-emerald-300">Freedom runway</p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">
            {runway(metrics.freedom_months, metrics.freedom_years)}
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            {metrics.monthly_expenses_usd > 0
              ? `at ${usd(metrics.monthly_expenses_usd)}/mo${metrics.expenses_derived ? " (auto)" : ""}`
              : "set monthly expenses"}
          </p>
        </div>
        <div className="rounded-xl border border-slate-700/60 bg-slate-900/40 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-slate-400">Next milestone</p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">
            {next ? `${fmtMultiple(next.multiple)} = ${usd(next.target_usd)}` : "5× cleared 🎉"}
          </p>
          <p className="mt-0.5 text-xs text-slate-500">{next ? `${Math.round(next.progress * 100)}% there` : "all milestones reached"}</p>
        </div>
      </div>

      <div className="mt-4 flex items-center gap-2">
        {metrics.milestones.map((m) => (
          <div key={m.multiple} className="flex flex-1 flex-col items-center gap-1">
            <span className={`text-xs font-semibold ${m.reached ? "text-emerald-300" : "text-slate-500"}`}>
              {fmtMultiple(m.multiple)}
            </span>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
              <div className={`h-full rounded-full ${m.reached ? "bg-emerald-400" : "bg-indigo-500"}`} style={{ width: `${Math.max(4, Math.round(m.progress * 100))}%` }} />
            </div>
          </div>
        ))}
      </div>

      <p className="mt-3 text-sm text-slate-300">
        At {usd(metrics.current_usd)} you're {metrics.salary_multiple.toFixed(1)}× your base salary
        {metrics.freedom_months !== null && <> — about {runway(metrics.freedom_months, metrics.freedom_years)} of freedom banked</>}. 🧭
      </p>

      <div className="mt-auto border-t border-slate-700/60 pt-3">
        <button onClick={() => (editing ? cancel() : setEditing(true))} className="text-xs font-medium text-slate-400 hover:text-slate-200">
          {editing ? "× Close" : "⚙ My numbers"}
        </button>
        {editing && form && (
          <div className="mt-3 space-y-3">
            <div className="grid grid-cols-3 gap-3">
              <NumField label="Base salary" suffix="$/yr" value={form.base_salary_usd} onChange={(v) => setField("base_salary_usd", v)} />
              <NumField label="Expenses (0=auto)" suffix="$/mo" value={form.monthly_expenses_usd} onChange={(v) => setField("monthly_expenses_usd", v)} />
              <NumField label="Years earning" suffix="yr" value={form.years_earning} onChange={(v) => setField("years_earning", v)} />
            </div>
            {error && <p className="text-xs text-red-400">{error}</p>}
            <div className="flex items-center gap-2">
              <button onClick={apply} disabled={saving} className="rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500 disabled:opacity-50">
                {saving ? "Saving…" : "Apply"}
              </button>
              <button onClick={cancel} disabled={saving} className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-400 hover:bg-slate-800 disabled:opacity-50">
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function NumField({ label, suffix, value, onChange }: { label: string; suffix: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="block">
      <span className="text-[11px] font-medium text-slate-400">{label}</span>
      <div className="mt-1 flex items-center rounded-lg border border-slate-700 bg-slate-900/60 px-2 focus-within:border-emerald-500">
        <input type="number" inputMode="decimal" value={value} onChange={(e) => onChange(e.target.value)} className="w-full bg-transparent py-1.5 text-sm text-white outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none" />
        <span className="pl-1 text-[11px] text-slate-500">{suffix}</span>
      </div>
    </label>
  );
}
