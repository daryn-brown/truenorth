import type { Currency, MoneyPair, NetWorth, NetWorthDelta } from "../types/finance";
import DesktopIcon from "../apps/desktop/DesktopIcon";
import DismissibleBanner from "./DismissibleBanner";

interface Props {
  netWorth: NetWorth | null;
  delta: NetWorthDelta | null;
  homeCurrency: Currency;
  onToggleCurrency: () => void;
  loading: boolean;
  connectionWarningKey?: string | null;
  onReviewConnections?: () => void;
}

const fmt = (value: number, currency: Currency) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(value);

const fmtAbs = (value: number, currency: Currency) => fmt(Math.abs(value), currency);

const fmtWhole = (value: number, currency: Currency) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 0,
  }).format(value);

const pick = (pair: MoneyPair | undefined, currency: Currency) =>
  pair ? (currency === "CAD" ? pair.cad : pair.usd) : 0;

const ALLOCATION_GROUPS = [
  { key: "investments", label: "Investments", description: "Stocks, plans & retirement" },
  { key: "savings", label: "Savings", description: "Cash assets" },
  { key: "liabilities", label: "Liabilities", description: "Credit cards & other debt" },
] as const;

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
  connectionWarningKey = null,
  onReviewConnections,
}: Props) {
  const primary = homeCurrency === "CAD" ? netWorth?.total_cad : netWorth?.total_usd;
  const secondary = homeCurrency === "CAD" ? netWorth?.total_usd : netWorth?.total_cad;
  const secondaryCurrency: Currency = homeCurrency === "CAD" ? "USD" : "CAD";
  const hasAccounts = (netWorth?.accounts.length ?? 0) > 0;
  const unclassified = pick(netWorth?.allocation.unclassified, homeCurrency);

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

      <DismissibleBanner
        noticeKey={connectionWarningKey}
        dismissLabel="Dismiss balance warning"
        className="mt-4 rounded-xl border border-amber-700/40 bg-amber-900/20 px-4 py-3 text-amber-200"
        role="status"
        hidden={loading}
      >
        <p className="text-sm font-semibold text-amber-200">Figures may be out of date or incomplete</p>
        <p className="mt-1 text-xs text-slate-400">
          Some SimpleFIN connections or balances need attention. Totals include last-known
          balances; accounts without a balance are not included.
        </p>
        <button type="button" onClick={onReviewConnections} className="mt-2 text-xs text-amber-200 underline">
          Review connections
        </button>
      </DismissibleBanner>

      {!loading && (
        <div className={`net-worth-orbs net-worth-orbs--${hasAccounts ? 3 : 1}`}>
          {hasAccounts ? (
            ALLOCATION_GROUPS.map(({ key, label, description }, index) => (
              <div
                className={`net-worth-orb net-worth-orb--${index + 1}`}
                key={key}
                role="group"
                aria-label={label}
              >
                <span>{label}</span>
                <strong>{fmtWhole(pick(netWorth?.allocation[key], homeCurrency), homeCurrency)}</strong>
                <small>{description}</small>
              </div>
            ))
          ) : (
            <div className="net-worth-orb net-worth-orb--empty">
              <DesktopIcon name="wallet" />
              <strong>Add your first account</strong>
              <small>Your global balance view will appear here.</small>
            </div>
          )}
        </div>
      )}

      {!loading && unclassified !== 0 && (
        <p className="mt-2 text-xs text-slate-400">
          Unclassified assets: {fmt(unclassified, homeCurrency)} included in total wealth,
          outside these groups.
        </p>
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
