import DesktopIcon from "../apps/desktop/DesktopIcon";
import type {
  Currency,
  DividendHoldingEstimate,
  DividendSummary,
  MoneyPair,
} from "../types/finance";

interface Props {
  summary: DividendSummary | null;
  homeCurrency: Currency;
  loading: boolean;
  error: string | null;
  onRefresh: () => Promise<void> | void;
}

const money = (
  pair: MoneyPair,
  currency: Currency,
  maximumFractionDigits = 0,
) =>
  new Intl.NumberFormat(currency === "CAD" ? "en-CA" : "en-US", {
    style: "currency",
    currency,
    maximumFractionDigits,
  }).format(currency === "CAD" ? pair.cad : pair.usd);

const localMoney = (amount: number, currency: string) =>
  new Intl.NumberFormat(currency === "CAD" ? "en-CA" : "en-US", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(amount);

const researchedLabel = (iso: string | null) => {
  if (!iso) return "Awaiting research";
  const date = new Date(`${iso}T00:00:00`);
  return Number.isNaN(date.getTime())
    ? `Research ${iso}`
    : `Research ${date.toLocaleDateString("en-US", {
        month: "short",
        day: "numeric",
      })}`;
};

const estimateValue = (
  holding: DividendHoldingEstimate,
  currency: Currency,
) =>
  currency === "CAD"
    ? holding.estimated_annual.cad
    : holding.estimated_annual.usd;

export default function DividendIncomeCard({
  summary,
  homeCurrency,
  loading,
  error,
  onRefresh,
}: Props) {
  if (loading && !summary) {
    return (
      <div className="tn-card tn-card--dividends dividend-card">
        <div className="h-56 animate-pulse rounded-xl bg-slate-800" />
      </div>
    );
  }

  const payingHoldings =
    summary?.holdings
      .filter((holding) => holding.annual_income > 0)
      .sort(
        (a, b) =>
          estimateValue(b, homeCurrency) - estimateValue(a, homeCurrency),
      ) ?? [];
  const monthlyEstimate: MoneyPair = {
    usd: (summary?.estimated_annual.usd ?? 0) / 12,
    cad: (summary?.estimated_annual.cad ?? 0) / 12,
  };
  const estimateIsPartial =
    (summary?.unresolved_positions ?? 0) > 0 ||
    Boolean(summary?.currency_warning);

  return (
    <div className="tn-card tn-card--dividends dividend-card">
      <div className="dividend-card__header">
        <div>
          <p className="dividend-card__eyebrow">Passive income</p>
          <h2>Yearly dividends</h2>
          <span>
            Current shares plus trailing distribution research, with actual
            payments shown separately.
          </span>
        </div>
        <div className="dividend-card__actions">
          <span className="dividend-card__source">
            {researchedLabel(summary?.research_as_of ?? null)}
          </span>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={loading}
            title="Refresh dividend research"
            aria-label="Refresh dividend research"
          >
            <DesktopIcon name="refresh" />
            {loading ? "Researching" : "Refresh"}
          </button>
        </div>
      </div>

      {error && <div className="dividend-card__warning">{error}</div>}

      <div className="dividend-card__content">
        <div className="dividend-card__metrics">
          <div className="dividend-card__metric dividend-card__metric--primary">
            <span>
              Estimated annual income{estimateIsPartial ? " (partial)" : ""}
            </span>
            <strong>
              {summary ? money(summary.estimated_annual, homeCurrency) : "-"}
            </strong>
            <small>
              {summary
                ? `${money(monthlyEstimate, homeCurrency, 2)} per month`
                : "Sync an investment account to calculate"}
            </small>
          </div>

          <div className="dividend-card__metric">
            <span>Recorded in transaction history</span>
            <strong>
              {summary
                ? money(summary.recorded_last_12_months, homeCurrency)
                : "-"}
            </strong>
            <small>
              {summary?.recorded_payment_count
                ? `${summary.recorded_payment_count} payment${
                    summary.recorded_payment_count === 1 ? "" : "s"
                  } since ${summary.recorded_period_start}`
                : "No dividend payments identified yet"}
            </small>
          </div>

          <div className="dividend-card__coverage">
            <span>
              <strong>{summary?.dividend_positions ?? 0}</strong> paying
              positions
            </span>
            <span>
              <strong>{summary?.researched_positions ?? 0}</strong> of{" "}
              {summary?.position_count ?? 0} researched
            </span>
          </div>
        </div>

        <div className="dividend-card__positions">
          <div className="dividend-card__positions-heading">
            <span>Income by holding</span>
            <small>{summary?.research_source ?? "Market research"}</small>
          </div>

          {!summary || summary.position_count === 0 ? (
            <div className="dividend-card__empty">
              <DesktopIcon name="chart" />
              <strong>No stock positions found</strong>
              <span>
                Sync SnapTrade, SimpleFIN, or Questrade to pull your tickers and
                share counts automatically.
              </span>
            </div>
          ) : payingHoldings.length === 0 ? (
            <div className="dividend-card__empty">
              <DesktopIcon name="sparkles" />
              <strong>No trailing dividends found</strong>
              <span>
                The researched positions did not report cash distributions in
                the last twelve months.
              </span>
            </div>
          ) : (
            <div className="dividend-card__holding-list">
              {payingHoldings.slice(0, 6).map((holding) => (
                <div
                  className="dividend-card__holding"
                  key={`${holding.account_id}-${holding.symbol}`}
                >
                  <div className="dividend-card__ticker">
                    <strong>{holding.symbol}</strong>
                    <span>{holding.account_name}</span>
                  </div>
                  <div>
                    <span>{holding.quantity.toLocaleString()} shares</span>
                    <small>
                      {localMoney(
                        holding.annual_dividend_per_share,
                        holding.currency,
                      )}
                      /share
                      {holding.yield_percent !== null
                        ? ` - ${holding.yield_percent.toFixed(2)}% yield`
                        : ""}
                    </small>
                  </div>
                  <strong>
                    {money(holding.estimated_annual, homeCurrency)}
                  </strong>
                </div>
              ))}
            </div>
          )}

          {summary && payingHoldings.length > 6 && (
            <p className="dividend-card__more">
              Plus {payingHoldings.length - 6} more dividend-paying position
              {payingHoldings.length - 6 === 1 ? "" : "s"}.
            </p>
          )}
        </div>
      </div>

      {summary &&
        (summary.unresolved_positions > 0 ||
          summary.research_error_count > 0 ||
          summary.stale_positions > 0 ||
          summary.currency_warning) && (
          <div className="dividend-card__warning">
            {summary.unresolved_positions > 0 && (
              <span>
                No matching research for{" "}
                {summary.unresolved_symbols.slice(0, 4).join(", ")}
                {summary.unresolved_positions > 4 ? ", and others" : ""}.
              </span>
            )}
            {summary.research_error_count > 0 && (
              <span>
                {summary.research_error_count} ticker refresh
                {summary.research_error_count === 1 ? "" : "es"} failed; cached
                figures were kept where available.
              </span>
            )}
            {summary.stale_positions > 0 && (
              <span>
                {summary.stale_positions} position
                {summary.stale_positions === 1 ? " is" : "s are"} using older
                cached research.
              </span>
            )}
            {summary.currency_warning && (
              <span>Refresh FX rates to complete currency conversion.</span>
            )}
          </div>
        )}

      <p className="dividend-card__footnote">
        Estimate = current connected shares x distributions paid per share over
        the trailing 12 months. Recorded payments depend on the transaction
        history supplied by your provider and may be partial.
      </p>
    </div>
  );
}
