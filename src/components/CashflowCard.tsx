import { useCallback, useEffect, useState } from "react";
import type {
  CashflowSummary,
  ClassifiedTransaction,
  Currency,
  FlowType,
  MoneyPair,
} from "../types/finance";
import { listRecentTransactions, setTransactionFlow } from "../hooks/useFinanceApi";

interface Props {
  summary: CashflowSummary | null;
  homeCurrency: Currency;
  loading: boolean;
  /** Re-fetch the dashboard summary after a retag changes the totals. */
  onChanged: () => void;
}

const money = (pair: MoneyPair, currency: Currency, maximumFractionDigits = 0) =>
  new Intl.NumberFormat(currency === "CAD" ? "en-CA" : "en-US", {
    style: "currency",
    currency,
    maximumFractionDigits,
  }).format(currency === "CAD" ? pair.cad : pair.usd);

const signedAmount = (amount: number, currency: Currency) =>
  new Intl.NumberFormat(currency === "CAD" ? "en-CA" : "en-US", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
    signDisplay: "exceptZero",
  }).format(amount);

const shortDate = (iso: string) => {
  const d = new Date(`${iso}T00:00:00`);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
};

const FLOW_STYLES: Record<FlowType, string> = {
  income: "border-emerald-700/50 bg-emerald-900/40 text-emerald-300",
  fixed: "border-indigo-700/50 bg-indigo-900/40 text-indigo-300",
  variable: "border-amber-700/50 bg-amber-900/40 text-amber-300",
  transfer: "border-slate-600/60 bg-slate-800/60 text-slate-400",
};

const FLOW_OPTIONS: FlowType[] = ["income", "fixed", "variable", "transfer"];

/**
 * "Monthly Cashflow" — savings rate plus an income / fixed / variable breakdown over the trailing
 * window. Fixed commitments (the mom support transfer) are kept out of the variable "lifestyle"
 * number, and internal transfers are excluded entirely. Rows can be retagged inline.
 */
export default function CashflowCard({
  summary,
  homeCurrency,
  loading,
  onChanged,
}: Props) {
  const [open, setOpen] = useState(false);
  const [txns, setTxns] = useState<ClassifiedTransaction[]>([]);
  const [txnsLoading, setTxnsLoading] = useState(false);
  const [savingId, setSavingId] = useState<number | null>(null);

  const loadTxns = useCallback(async () => {
    setTxnsLoading(true);
    try {
      setTxns(await listRecentTransactions(50));
    } catch (err) {
      console.error("Failed to load transactions:", err);
    } finally {
      setTxnsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open && txns.length === 0) void loadTxns();
  }, [open, txns.length, loadTxns]);

  const handleRetag = async (id: number, value: string) => {
    setSavingId(id);
    try {
      await setTransactionFlow(id, value === "" ? null : (value as FlowType));
      await loadTxns();
      onChanged();
    } catch (err) {
      console.error("Failed to retag transaction:", err);
    } finally {
      setSavingId(null);
    }
  };

  if (loading && !summary) {
    return (
      <div className="rounded-2xl border border-slate-700 bg-slate-900/40 p-6">
        <div className="h-28 animate-pulse rounded-lg bg-slate-800" />
      </div>
    );
  }
  if (!summary) return null;

  const ratePct = Math.round(summary.savings_rate * 100);
  const netNegative = summary.net_savings.usd < 0;
  const expenseTotal =
    (homeCurrency === "CAD" ? summary.fixed.cad : summary.fixed.usd) +
    (homeCurrency === "CAD" ? summary.variable.cad : summary.variable.usd);
  const incomeVal = homeCurrency === "CAD" ? summary.income.cad : summary.income.usd;
  const fixedVal = homeCurrency === "CAD" ? summary.fixed.cad : summary.fixed.usd;
  const variableVal =
    homeCurrency === "CAD" ? summary.variable.cad : summary.variable.usd;
  const denom = Math.max(incomeVal, expenseTotal, 1);

  const Bar = ({
    label,
    value,
    pair,
    color,
    hint,
  }: {
    label: string;
    value: number;
    pair: MoneyPair;
    color: string;
    hint?: string;
  }) => (
    <div>
      <div className="flex items-baseline justify-between text-sm">
        <span className="text-slate-300">
          {label}
          {hint && <span className="ml-1 text-xs text-slate-500">{hint}</span>}
        </span>
        <span className="font-medium text-white">{money(pair, homeCurrency)}</span>
      </div>
      <div className="mt-1 h-2 w-full overflow-hidden rounded-full bg-slate-800">
        <div
          className={`h-full rounded-full ${color}`}
          style={{ width: `${Math.max(2, Math.min(100, (value / denom) * 100))}%` }}
        />
      </div>
    </div>
  );

  return (
    <div className="rounded-2xl border border-slate-700 bg-gradient-to-br from-slate-800 to-slate-900 p-6 shadow-xl">
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium uppercase tracking-widest text-slate-400">
          Cashflow
        </p>
        <span className="rounded-full border border-slate-600/60 bg-slate-800/60 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
          Last {summary.window_days}d
        </span>
      </div>

      {summary.txn_count === 0 ? (
        <p className="mt-3 text-sm text-slate-400">
          No transactions yet. Connect a bank via SimpleFIN and sync to pull recent
          transactions — then I'll split your spending into fixed vs. variable and track your
          savings rate.
        </p>
      ) : (
        <>
          <div className="mt-3 flex items-baseline justify-between">
            <div>
              <span
                className={`text-3xl font-bold tracking-tight ${
                  netNegative ? "text-amber-400" : "text-emerald-400"
                }`}
              >
                {ratePct}%
              </span>
              <span className="ml-2 text-sm text-slate-400">savings rate</span>
            </div>
            <div className="text-right">
              <p className="text-sm font-semibold text-white">
                {money(summary.net_savings, homeCurrency)}
              </p>
              <p className="text-xs text-slate-500">
                {netNegative ? "net burn" : "net saved"}
              </p>
            </div>
          </div>

          <div className="mt-4 space-y-3">
            <Bar
              label="Income"
              value={incomeVal}
              pair={summary.income}
              color="bg-emerald-500"
            />
            <Bar
              label="Fixed"
              hint="rent, mom support"
              value={fixedVal}
              pair={summary.fixed}
              color="bg-indigo-500"
            />
            <Bar
              label="Variable"
              hint="lifestyle"
              value={variableVal}
              pair={summary.variable}
              color="bg-amber-500"
            />
          </div>

          <p className="mt-4 rounded-xl border border-slate-700/70 bg-slate-900/40 px-4 py-3 text-xs text-slate-400">
            Fixed commitments are kept out of the <span className="text-amber-300">variable</span>{" "}
            "lifestyle creep" number, and {summary.transfer_count} internal transfer
            {summary.transfer_count === 1 ? "" : "s"} (card payments, account moves)
            {summary.transfer_count === 1 ? " was" : " were"} excluded so nothing double-counts.
            {summary.currency_warning && (
              <span className="mt-1 block text-amber-400">
                Some transactions are in a currency with no exchange rate yet — refresh FX to fold
                them in.
              </span>
            )}
          </p>

          <button
            onClick={() => setOpen((o) => !o)}
            className="mt-4 text-xs font-medium text-indigo-300 hover:text-indigo-200"
          >
            {open ? "Hide transactions" : `Review transactions (${summary.txn_count})`}
          </button>

          {open && (
            <div className="mt-3 space-y-1.5">
              {txnsLoading && txns.length === 0 ? (
                <div className="h-16 animate-pulse rounded-lg bg-slate-800" />
              ) : (
                txns.map((t) => (
                  <div
                    key={t.id}
                    className="flex items-center gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-3 py-2"
                  >
                    <span className="w-12 shrink-0 text-xs text-slate-500">
                      {shortDate(t.txn_date)}
                    </span>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm text-slate-200">{t.description}</p>
                      <p className="truncate text-[11px] text-slate-500">{t.account_name}</p>
                    </div>
                    <span
                      className={`shrink-0 text-sm font-medium ${
                        t.amount < 0 ? "text-slate-300" : "text-emerald-400"
                      }`}
                    >
                      {signedAmount(t.amount, t.currency)}
                    </span>
                    <span
                      className={`hidden shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider sm:inline ${
                        FLOW_STYLES[t.flow_type]
                      }`}
                      title={t.is_override ? "Manually set" : "Auto-classified"}
                    >
                      {t.flow_type}
                      {t.is_override ? " •" : ""}
                    </span>
                    <select
                      value={t.is_override ? t.flow_type : ""}
                      disabled={savingId === t.id}
                      onChange={(e) => handleRetag(t.id, e.target.value)}
                      className="shrink-0 rounded-md border border-slate-700 bg-slate-800 px-1.5 py-1 text-xs text-slate-300 disabled:opacity-50"
                      title="Override this transaction's classification"
                    >
                      <option value="">Auto</option>
                      {FLOW_OPTIONS.map((f) => (
                        <option key={f} value={f}>
                          {f.charAt(0).toUpperCase() + f.slice(1)}
                        </option>
                      ))}
                    </select>
                  </div>
                ))
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
