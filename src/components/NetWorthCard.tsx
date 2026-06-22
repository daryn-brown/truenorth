import type { Currency, MoneyPair, NetWorth, NetWorthDelta } from "../types/finance";

interface Props {
  netWorth: NetWorth | null;
  delta: NetWorthDelta | null;
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

const fmtAbs = (value: number, currency: Currency) => fmt(Math.abs(value), currency);

const pick = (pair: MoneyPair | undefined, currency: Currency) =>
  pair ? (currency === "CAD" ? pair.cad : pair.usd) : 0;

/** Sub-dollar float noise shouldn't read as a real move. */
const EPS = 1;

const sinceLabel = (isoDate: string | null): string | null => {
  if (!isoDate) return null;
  const d = new Date(`${isoDate}T00:00:00`);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString("en-CA", { month: "short", day: "numeric" });
};

/**
 * The "Anxiety Buffer": when spendable cash drops but net worth holds or grows, lead with the
 * net-worth move (green) and explicitly reassure that the cash dip didn't shrink the macro picture.
 */
function AnxietyBuffer({
  delta,
  homeCurrency,
}: {
  delta: NetWorthDelta;
  homeCurrency: Currency;
}) {
  const totalDelta = pick(delta.total_delta, homeCurrency);
  const liquidDelta = pick(delta.liquid_delta, homeCurrency);
  const investedDelta = pick(delta.invested_delta, homeCurrency);

  const netUp = totalDelta > EPS;
  const netFlat = Math.abs(totalDelta) <= EPS;
  const cashDown = liquidDelta < -EPS;
  const since = sinceLabel(delta.previous_date);

  const headlineColor = netUp || netFlat ? "text-emerald-400" : "text-amber-400";
  const arrow = netUp ? "▲" : netFlat ? "→" : "▼";
  const headline = netFlat
    ? "Net worth holding steady"
    : `Net worth ${arrow} ${fmtAbs(totalDelta, homeCurrency)}`;

  // The reassurance case: cash fell, but the macro number didn't.
  const reassure = cashDown && totalDelta >= -EPS;

  return (
    <div className="mt-5 rounded-xl border border-slate-700/70 bg-slate-900/40 px-4 py-3">
      <div className="flex items-baseline gap-2">
        <span className={`text-sm font-semibold ${headlineColor}`}>{headline}</span>
        {since && <span className="text-xs text-slate-500">since {since}</span>}
      </div>

      {reassure ? (
        <p className="mt-1 text-xs leading-relaxed text-slate-400">
          Your cash is down {fmtAbs(liquidDelta, homeCurrency)}, but your net worth is{" "}
          {netFlat ? "holding steady" : `up ${fmtAbs(totalDelta, homeCurrency)}`}
          {investedDelta > EPS && (
            <> — investments climbed {fmtAbs(investedDelta, homeCurrency)}</>
          )}
          . Zoom out: the macro picture is intact. 🧭
        </p>
      ) : (
        <p className="mt-1 text-xs text-slate-500">
          Cash {liquidDelta >= 0 ? "+" : "−"}
          {fmtAbs(liquidDelta, homeCurrency)} · Investments {investedDelta >= 0 ? "+" : "−"}
          {fmtAbs(investedDelta, homeCurrency)}
        </p>
      )}
    </div>
  );
}

export default function NetWorthCard({
  netWorth,
  delta,
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

      {!loading && delta?.has_previous && (
        <AnxietyBuffer delta={delta} homeCurrency={homeCurrency} />
      )}

      {netWorth?.rate_date && (
        <p className="mt-4 text-xs text-slate-500">
          FX rate as of {netWorth.rate_date} · 1 USD ={" "}
          {netWorth.usd_cad_rate?.toFixed(4)} CAD
        </p>
      )}
    </div>
  );
}
