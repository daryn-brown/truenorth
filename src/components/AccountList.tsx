import type { Account, AccountNetWorth } from "../types/finance";

const ACCOUNT_TYPE_LABELS: Record<string, string> = {
  chequing: "Chequing",
  savings: "Savings",
  brokerage: "Brokerage",
  tfsa: "TFSA",
  rrsp: "RRSP",
  fhsa: "FHSA",
  "401k": "401(k)",
  ira: "IRA",
  roth_ira: "Roth IRA",
  credit: "Credit",
  crypto: "Crypto",
  other: "Other",
};

const JURISDICTION_COLORS: Record<string, string> = {
  US: "bg-blue-900/40 text-blue-300 border-blue-700/50",
  CA: "bg-red-900/40 text-red-300 border-red-700/50",
};

const fmt = (value: number, currency: string) =>
  new Intl.NumberFormat("en-CA", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(value);

interface Props {
  accounts: Account[];
  netWorthBreakdown: AccountNetWorth[];
  onAddAccount: () => void;
  onDeleteAccount: (id: number) => void;
  onUpdateBalance: (account: Account) => void;
  onEditCurrency: (account: Account) => void;
}

export default function AccountList({
  accounts,
  netWorthBreakdown,
  onAddAccount,
  onDeleteAccount,
  onUpdateBalance,
  onEditCurrency,
}: Props) {
  const breakdownMap = new Map(netWorthBreakdown.map((a) => [a.account_id, a]));

  return (
    <div className="rounded-2xl border border-slate-700 bg-slate-800/60 backdrop-blur-sm">
      <div className="flex items-center justify-between border-b border-slate-700 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-200 uppercase tracking-widest">
          Accounts
        </h2>
        <button
          onClick={onAddAccount}
          className="flex items-center gap-1.5 rounded-lg bg-indigo-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-indigo-500 transition-colors"
        >
          <span className="text-base leading-none">+</span> Add Account
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
        <ul className="divide-y divide-slate-700/60">
          {accounts.map((account) => {
            const bk = breakdownMap.get(account.id);
            return (
              <li
                key={account.id}
                className="flex items-center justify-between gap-4 px-5 py-4 hover:bg-slate-700/30 transition-colors group"
              >
                <div className="flex items-center gap-3 min-w-0">
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
                    <p className="text-xs text-slate-500">
                      {account.institution} ·{" "}
                      {ACCOUNT_TYPE_LABELS[account.account_type] ??
                        account.account_type}
                      {account.connector_kind === "snaptrade" && (
                        <span className="ml-1.5 inline-flex items-center rounded bg-indigo-900/40 border border-indigo-700/50 px-1.5 py-0.5 text-[10px] font-medium text-indigo-300">
                          via SnapTrade
                        </span>
                      )}
                    </p>
                  </div>
                </div>

                <div className="flex items-center gap-3 shrink-0">
                  <div className="text-right">
                    {bk ? (
                      <>
                        <p className="text-sm font-semibold text-slate-100">
                          {fmt(bk.balance, bk.currency)}
                        </p>
                        <p className="text-xs text-slate-500">
                          {bk.snapshot_date ?? "no snapshot"}
                        </p>
                      </>
                    ) : (
                      <p className="text-sm text-slate-500">no balance</p>
                    )}
                  </div>

                  <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button
                      onClick={() => onEditCurrency(account)}
                      title={`Change currency (now ${account.currency})`}
                      className="rounded p-1.5 text-slate-400 hover:bg-slate-600 hover:text-white transition-colors"
                    >
                      💱
                    </button>
                    <button
                      onClick={() => onUpdateBalance(account)}
                      title="Update balance"
                      className="rounded p-1.5 text-slate-400 hover:bg-slate-600 hover:text-white transition-colors"
                    >
                      ✏️
                    </button>
                    <button
                      onClick={() => onDeleteAccount(account.id)}
                      title="Delete account"
                      className="rounded p-1.5 text-slate-400 hover:bg-red-900/50 hover:text-red-400 transition-colors"
                    >
                      🗑️
                    </button>
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
