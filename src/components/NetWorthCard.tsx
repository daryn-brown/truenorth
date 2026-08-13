import type { Currency, MoneyPair, NetWorth, NetWorthDelta } from "../types/finance";
import DesktopIcon from "../apps/desktop/DesktopIcon";

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

const fmtAny = (value: number, currency: string) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 0,
  }).format(value);

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
    <div className="net-worth-insight mt-5 rounded-xl border border-slate-700/70 bg-slate-900/40 px-4 py-3">
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
          . Zoom out: the macro picture is intact.
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
  const topAccounts = [...(netWorth?.accounts ?? [])]
    .sort((a, b) => {
      const aValue = homeCurrency === "CAD" ? a.balance_cad : a.balance_usd;
      const bValue = homeCurrency === "CAD" ? b.balance_cad : b.balance_usd;
      return bValue - aValue;
    })
    .slice(0, 3);

  return (
    <div className="tn-card tn-card--hero net-worth-card rounded-2xl bg-gradient-to-br from-slate-800 to-slate-900 border border-slate-700 p-6 shadow-xl">
      <div className="net-worth-card__header flex items-start justify-between">
        <div className="net-worth-card__total">
          <p className="text-sm font-medium text-slate-400 uppercase tracking-widest">Total wealth</p>
          <span className="net-worth-card__hint">Across every account and currency</span>
          {loading ? (
            <div className="mt-2 h-10 w-56 animate-pulse rounded-lg bg-slate-700" />
          ) : (
            <p className="net-worth-card__amount mt-1 text-4xl font-bold text-white tracking-tight">
              {primary !== undefined ? fmt(primary, homeCurrency) : "—"}
            </p>
          )}
          {!loading && secondary !== undefined && (
            <p className="net-worth-card__secondary mt-1 text-base text-slate-400">
              ≈ {fmt(secondary, secondaryCurrency)}
            </p>
          )}
        </div>

        <button
          onClick={onToggleCurrency}
          className="net-worth-currency"
          title="Toggle home currency"
        >
          <span className="is-active">{homeCurrency}</span>
          <span>{secondaryCurrency}</span>
        </button>
      </div>

      {!loading && (
        <div className={`net-worth-orbs net-worth-orbs--${Math.max(topAccounts.length, 1)}`}>
          {topAccounts.length > 0 ? (
            topAccounts.map((account, index) => {
              const homeValue =
                homeCurrency === "CAD" ? account.balance_cad : account.balance_usd;
              return (
                <div className={`net-worth-orb net-worth-orb--${index + 1}`} key={account.account_id}>
                  <span>{account.institution}</span>
                  <strong>{fmtAny(homeValue, homeCurrency)}</strong>
                  <small>
                    {account.account_name}
                    {account.currency !== homeCurrency
                      ? ` · ${fmtAny(account.balance, account.currency)}`
                      : ""}
                  </small>
                </div>
              );
            })
          ) : (
            <div className="net-worth-orb net-worth-orb--empty">
              <DesktopIcon name="wallet" />
              <strong>Add your first account</strong>
              <small>Your global balance view will appear here.</small>
            </div>
          )}
        </div>
      )}

      {!loading && delta?.has_previous && (
        <AnxietyBuffer delta={delta} homeCurrency={homeCurrency} />
      )}

      {netWorth?.rate_date && (
        <div className="net-worth-card__footer">
          <span>
            <DesktopIcon name="shield" />
            Local and encrypted
          </span>
          <span>
            FX · {netWorth.rate_date} · 1 USD = {netWorth.usd_cad_rate?.toFixed(4)} CAD
          </span>
        </div>
      )}
    </div>
  );
}
