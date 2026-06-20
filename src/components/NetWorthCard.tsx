import type { Currency, NetWorth } from "../types/finance";

interface Props {
  netWorth: NetWorth | null;
  homeCurrency: Currency;
  onToggleCurrency: () => void;
  loading: boolean;
}

const fmt = (value: number, currency: Currency) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(value);

export default function NetWorthCard({
  netWorth,
  homeCurrency,
  onToggleCurrency,
  loading,
}: Props) {
  const primary = homeCurrency === "CAD" ? netWorth?.total_cad : netWorth?.total_usd;
  const secondary = homeCurrency === "CAD" ? netWorth?.total_usd : netWorth?.total_cad;
  const secondaryCurrency: Currency = homeCurrency === "CAD" ? "USD" : "CAD";

  return (
    <div className="rounded-2xl bg-gradient-to-br from-slate-800 to-slate-900 border border-slate-700 p-6 shadow-xl">
      <div className="flex items-start justify-between">
        <div>
          <p className="text-sm font-medium text-slate-400 uppercase tracking-widest">
            Net Worth
          </p>
          {loading ? (
            <div className="mt-2 h-10 w-56 animate-pulse rounded-lg bg-slate-700" />
          ) : (
            <p className="mt-1 text-4xl font-bold text-white tracking-tight">
              {primary !== undefined ? fmt(primary, homeCurrency) : "—"}
            </p>
          )}
          {!loading && secondary !== undefined && (
            <p className="mt-1 text-base text-slate-400">
              ≈ {fmt(secondary, secondaryCurrency)}
            </p>
          )}
        </div>

        <button
          onClick={onToggleCurrency}
          className="mt-1 rounded-lg border border-slate-600 px-3 py-1.5 text-xs font-semibold text-slate-300 hover:bg-slate-700 transition-colors"
          title="Toggle home currency"
        >
          {homeCurrency}
        </button>
      </div>

      {netWorth?.rate_date && (
        <p className="mt-4 text-xs text-slate-500">
          FX rate as of {netWorth.rate_date} · 1 USD ={" "}
          {netWorth.usd_cad_rate?.toFixed(4)} CAD
        </p>
      )}
    </div>
  );
}
