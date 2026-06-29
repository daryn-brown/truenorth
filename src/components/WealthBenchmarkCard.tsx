import { useEffect, useState } from "react";
import type { WealthBenchmark, WealthInputs } from "../types/finance";

interface Props {
  benchmark: WealthBenchmark | null;
  loading: boolean;
  onUpdate: (inputs: WealthInputs) => Promise<void> | void;
}

type FormState = Record<keyof WealthInputs, string>;

const usd = (value: number, maximumFractionDigits = 0) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);

const STATUS: Record<WealthBenchmark["status"], { label: string; cls: string }> = {
  Under: { label: "Under-accumulator", cls: "border-amber-700/50 bg-amber-900/30 text-amber-300" },
  Average: { label: "On track", cls: "border-indigo-700/50 bg-indigo-900/40 text-indigo-300" },
  Prodigious: { label: "Prodigious", cls: "border-emerald-700/50 bg-emerald-900/30 text-emerald-300" },
};

const toForm = (i: WealthInputs): FormState => ({
  current_age: String(i.current_age),
  gross_income_usd: String(i.gross_income_usd),
  years_earning: String(i.years_earning),
});

const parseForm = (f: FormState): WealthInputs | null => {
  const vals = {
    current_age: Number(f.current_age),
    gross_income_usd: Number(f.gross_income_usd),
    years_earning: Number(f.years_earning),
  };
  return Object.values(vals).every((v) => Number.isFinite(v)) ? vals : null;
};

/**
 * Wealth benchmark: net worth vs. the Millionaire Next Door expected figure, an under-40 adjusted
 * target, and accumulation velocity. Neutral defaults; the user enters their own numbers.
 */
export default function WealthBenchmarkCard({ benchmark, loading, onUpdate }: Props) {
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (benchmark && !editing) setForm(toForm(benchmark.inputs));
  }, [benchmark, editing]);

  if (loading && !benchmark) {
    return (
      <div className="rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-40 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!benchmark) return null;

  const status = STATUS[benchmark.status];
  const pct = Math.round(benchmark.adjusted_progress * 100);

  const setField = (key: keyof WealthInputs, value: string) =>
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
    setForm(toForm(benchmark.inputs));
    setError(null);
    setEditing(false);
  };

  return (
    <div className="rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium uppercase tracking-widest text-slate-400">
            Wealth Benchmark
          </p>
          <p className="mt-0.5 text-xs text-slate-500">
            Net worth vs. The Millionaire Next Door formula
          </p>
        </div>
        <span className={`rounded-full border px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider ${status.cls}`}>
          {status.label}
        </span>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-slate-700/60 bg-slate-900/40 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-slate-400">Expected</p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">{usd(benchmark.expected_usd)}</p>
          <p className="mt-0.5 text-xs text-slate-500">you're at {(benchmark.ratio * 100).toFixed(0)}%</p>
        </div>
        <div className="rounded-xl border border-emerald-700/40 bg-emerald-950/20 px-4 py-3">
          <p className="text-[11px] font-medium uppercase tracking-wider text-emerald-300">Adjusted (under-40)</p>
          <p className="mt-1 text-2xl font-bold tracking-tight text-white">{usd(benchmark.adjusted_usd)}</p>
          <p className="mt-0.5 text-xs text-slate-500">{usd(benchmark.velocity_usd)}/yr velocity</p>
        </div>
      </div>

      <div className="mt-3 h-2 w-full overflow-hidden rounded-full bg-slate-800">
        <div className="h-full rounded-full bg-gradient-to-r from-indigo-500 to-emerald-400 transition-all" style={{ width: `${Math.max(2, Math.min(100, pct))}%` }} />
      </div>
      <p className="mt-2 text-sm text-slate-300">
        At {usd(benchmark.current_usd)}, you're {pct}% to the adjusted target — PAW bar is{" "}
        <span className="font-semibold text-white">{usd(benchmark.prodigious_usd)}</span>. 🧭
      </p>

      <div className="mt-4 border-t border-slate-700/60 pt-3">
        <button onClick={() => (editing ? cancel() : setEditing(true))} className="text-xs font-medium text-slate-400 hover:text-slate-200">
          {editing ? "× Close" : "⚙ My numbers"}
        </button>
        {editing && form && (
          <div className="mt-3 space-y-3">
            <div className="grid grid-cols-3 gap-3">
              <NumField label="Current age" suffix="yr" value={form.current_age} onChange={(v) => setField("current_age", v)} />
              <NumField label="Gross income" suffix="$/yr" value={form.gross_income_usd} onChange={(v) => setField("gross_income_usd", v)} />
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
