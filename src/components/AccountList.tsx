import type { Account, AccountNetWorth, Currency, SimpleFinConnectionHealth } from "../types/finance";
import DesktopIcon from "../apps/desktop/DesktopIcon";
import { ACCOUNT_TYPE_LABELS, CONNECTOR_LABELS } from "../shared/accountLabels";
import ConnectionStatus, { formatConnectionTime } from "./ConnectionStatus";

const JURISDICTION_COLORS: Record<string, string> = {
  US: "bg-blue-900/40 text-blue-300 border-blue-700/50",
  CA: "bg-red-900/40 text-red-300 border-red-700/50",
};

/** Jurisdictional portfolios; an account can still be denominated in another currency. */
const PORTFOLIO_GROUPS: { key: string; label: string }[] = [
  { key: "US", label: "US Portfolio" },
  { key: "CA", label: "Canada / International" },
];

const fmt = (value: number, currency: string) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(value);

interface Props {
  accounts: Account[];
  netWorthBreakdown: AccountNetWorth[];
  homeCurrency: Currency;
  onAddAccount: () => void;
  onDeleteAccount: (account: Account) => void;
  onUpdateBalance: (account: Account) => void;
  onEditCurrency: (account: Account) => void;
  simplefinHealth?: SimpleFinConnectionHealth[];
  onManageConnections?: () => void;
}

export default function AccountList({
  accounts,
  netWorthBreakdown,
  homeCurrency,
  onAddAccount,
  onDeleteAccount,
  onUpdateBalance,
  onEditCurrency,
  simplefinHealth = [],
  onManageConnections,
}: Props) {
  const breakdownMap = new Map(netWorthBreakdown.map((a) => [a.account_id, a]));
  const healthMap = new Map(simplefinHealth.flatMap((bank) =>
    bank.accounts.map((account) => [account.account_id, account.health] as const)));

  const homeValue = (account: Account) => {
    const bk = breakdownMap.get(account.id);
    if (!bk) return 0;
    return homeCurrency === "CAD" ? bk.balance_cad : bk.balance_usd;
  };

  // Bucket accounts into the known portfolios, then append any unexpected jurisdiction so nothing
  // silently disappears. Empty groups are dropped.
  const knownKeys = PORTFOLIO_GROUPS.map((g) => g.key);
  const extras = [...new Set(accounts.map((a) => a.jurisdiction))].filter(
    (j) => !knownKeys.includes(j),
  );
  const groups = [
    ...PORTFOLIO_GROUPS,
    ...extras.map((key) => ({ key, label: key })),
  ]
    .map((g) => ({ ...g, items: accounts.filter((a) => a.jurisdiction === g.key) }))
    .filter((g) => g.items.length > 0);

  const renderAccount = (account: Account) => {
    const bk = breakdownMap.get(account.id);
    const health = healthMap.get(account.id);
    return (
      <li
        key={account.id}
        className="account-row flex items-center justify-between gap-4 px-5 py-4 hover:bg-slate-700/30 transition-colors group"
      >
        <div className="account-row__identity flex items-center gap-3 min-w-0">
          <div className="account-row__icon">
            <DesktopIcon
              name={account.account_type === "brokerage" ? "chart" : "wallet"}
            />
          </div>
          <div className="shrink-0">
            <span
              className={`inline-flex items-center rounded border px-1.5 py-0.5 text-[10px] font-bold tracking-wider ${
                JURISDICTION_COLORS[account.jurisdiction]
              }`}
            >
              {account.jurisdiction}
            </span>
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-medium text-slate-200">
              {account.name}
            </p>
            {health && (
              <button type="button" onClick={onManageConnections} className="mt-1"
                title={health.message ?? "View connection health"}
                aria-label={`Review connection for ${account.name}`}>
                <ConnectionStatus status={health.status} />
              </button>
            )}
            <p className="text-xs text-slate-500">
              {account.institution} ·{" "}
              {ACCOUNT_TYPE_LABELS[account.account_type] ?? account.account_type}
              {account.connector_kind !== "manual" && (
                <span className="ml-1.5 inline-flex items-center rounded bg-indigo-900/40 border border-indigo-700/50 px-1.5 py-0.5 text-[10px] font-medium text-indigo-300">
                  via {CONNECTOR_LABELS[account.connector_kind]}
                </span>
              )}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3 shrink-0">
          <div className="text-right">
            {bk && bk.snapshot_date !== null ? (
              <>
                <p className="text-sm font-semibold text-slate-100">
                  {fmt(bk.balance, bk.currency)}
                </p>
                <p className="text-xs text-slate-500">
                  {bk.currency !== homeCurrency && (
                    <span className="mr-1 text-slate-600">
                      ≈ {fmt(homeValue(account), homeCurrency)} ·
                    </span>
                  )}
                  {health?.balance_as_of
                    ? `As of ${formatConnectionTime(health.balance_as_of)}`
                    : bk.snapshot_date}
                </p>
              </>
            ) : (
              <p className="text-sm text-slate-500">No balance available</p>
            )}
          </div>

          <div className="account-actions flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
            <button
              onClick={() => onEditCurrency(account)}
              title={`Change currency (now ${account.currency})`}
              className="rounded p-1.5 text-slate-400 hover:bg-slate-600 hover:text-white transition-colors"
            >
              <DesktopIcon name="exchange" />
            </button>
            <button
              onClick={() => onUpdateBalance(account)}
              title="Update balance"
              className="rounded p-1.5 text-slate-400 hover:bg-slate-600 hover:text-white transition-colors"
            >
              <DesktopIcon name="edit" />
            </button>
            <button
              onClick={() => onDeleteAccount(account)}
              title="Delete account"
              aria-label={`Delete ${account.name}`}
              className="rounded p-1.5 text-slate-400 hover:bg-red-900/50 hover:text-red-400 transition-colors"
            >
              <DesktopIcon name="trash" />
            </button>
          </div>
        </div>
      </li>
    );
  };

  return (
    <div className="tn-card tn-card--accounts account-list rounded-2xl border border-slate-700 bg-slate-800/60 backdrop-blur-sm">
      <div className="account-list__header flex items-center justify-between border-b border-slate-700 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-200 uppercase tracking-widest">
          Accounts
        </h2>
        <button
          onClick={onAddAccount}
          className="account-list__add flex items-center gap-1.5 rounded-lg bg-indigo-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-indigo-500 transition-colors"
        >
          <DesktopIcon name="plus" /> Add account
        </button>
      </div>

      {accounts.length === 0 ? (
        <div className="px-5 py-10 text-center text-sm text-slate-500">
          No accounts yet.{" "}
          <button
            onClick={onAddAccount}
            className="text-indigo-400 hover:text-indigo-300 underline"
          >
            Add your first account
          </button>{" "}
          to start tracking net worth.
        </div>
      ) : (
        <div className="divide-y divide-slate-700/60">
          {groups.map((group) => {
            const subtotal = group.items.reduce((sum, a) => sum + homeValue(a), 0);
            return (
              <section key={group.key}>
                <div className="account-group__header flex items-center justify-between bg-slate-900/40 px-5 py-2.5">
                  <span className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
                    <span className="account-group__jurisdiction">{group.key}</span>
                    {group.label}
                    <span className="rounded-full bg-slate-700/60 px-1.5 py-0.5 text-[10px] font-medium text-slate-400">
                      {group.items.length}
                    </span>
                  </span>
                  <span className="text-sm font-semibold text-slate-200">
                    {fmt(subtotal, homeCurrency)}
                  </span>
                </div>
                <ul className="divide-y divide-slate-700/60">
                  {group.items.map(renderAccount)}
                </ul>
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}
